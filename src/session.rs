use std::{
    io::{Read, Write},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, SlavePty};

use crate::{
    hooks::{self, Report},
    trace,
    windows::SessionId,
};

/// Ausgabe innerhalb dieses Fensters gilt als „arbeitet“ — Claude Code animiert seinen Spinner,
/// solange es rechnet, und schreibt nichts, wenn es auf Eingabe wartet.
const WORKING_WINDOW: Duration = Duration::from_millis(1500);
const SCROLLBACK: usize = 10_000;
/// Zeilen je Mausrad-Raste.
const WHEEL_LINES: usize = 3;
/// So oft wird die Statusdatei der Hooks gelesen.
const HOOK_POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Prozess noch nicht gestartet (wartet in der Startreihe) oder noch ohne Ausgabe.
    Starting,
    Working,
    Waiting,
    Idle,
    Exited,
}

pub struct Session {
    pub id: SessionId,
    /// ID der Claude-Code-Session (`--session-id` bzw. `--resume`).
    pub claude_id: String,
    pub cwd: PathBuf,
    pub status: Status,
    size: (u16, u16),
    parser: Arc<Mutex<vt100::Parser>>,
    last_output: Arc<Mutex<Instant>>,
    reader_done: Arc<AtomicBool>,
    /// Zeitpunkt der letzten Eingabe in die Session.
    last_input: SystemTime,
    /// Letzte Meldung der Claude-Code-Hooks; `None`, solange keine vorliegt.
    hook: Option<Report>,
    hook_checked: Option<Instant>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    /// Befehl und PTY-Gegenstelle, solange der Prozess noch nicht gestartet ist.
    pending: Option<(CommandBuilder, Box<dyn SlavePty + Send>, String)>,
    child: Option<Box<dyn Child + Send + Sync>>,
    /// Prozessstart; `None`, solange er in der Startreihe wartet.
    started: Option<Instant>,
    /// Erste sichtbare Ausgabe; bis dahin zeigt das Fenster „starting claude …“.
    first_content: Option<Instant>,
    /// Erste Hook-Meldung schon im Trace vermerkt.
    hook_traced: bool,
}

impl Session {
    /// `extra_args` werden einzeln an `command` gehängt und dabei für die Shell gequotet. Mit
    /// `start_now = false` wird nur das PTY geöffnet; der Prozess startet erst mit [`Self::start`]
    /// (gestaffeltes Wiederherstellen) oder mit der ersten Eingabe.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        id: SessionId,
        claude_id: String,
        command: &str,
        extra_args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
        start_now: bool,
    ) -> Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols))
            .context("cannot open PTY")?;

        let mut cmd = shell_command(command, extra_args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env(hooks::SESSION_ENV, &claude_id);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            non_zero(rows),
            non_zero(cols),
            SCROLLBACK,
        )));
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let reader_done = Arc::new(AtomicBool::new(false));

        {
            let parser = Arc::clone(&parser);
            let last_output = Arc::clone(&last_output);
            let reader_done = Arc::clone(&reader_done);
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            lock(&parser).process(&buf[..n]);
                            *lock(&last_output) = Instant::now();
                        }
                    }
                }
                reader_done.store(true, Ordering::SeqCst);
            });
        }

        let mut session = Self {
            id,
            claude_id,
            cwd: cwd.to_path_buf(),
            status: Status::Starting,
            size: (rows, cols),
            parser,
            last_output,
            reader_done,
            last_input: SystemTime::UNIX_EPOCH,
            hook: None,
            hook_checked: None,
            master: pair.master,
            writer,
            pending: Some((cmd, pair.slave, command.to_string())),
            child: None,
            started: None,
            first_content: None,
            hook_traced: false,
        };
        if start_now {
            session.start()?;
        }
        Ok(session)
    }

    /// Startet den Prozess, falls er noch in der Startreihe wartet.
    pub fn start(&mut self) -> Result<()> {
        let Some((cmd, slave, program)) = self.pending.take() else {
            return Ok(());
        };
        let child = slave
            .spawn_command(cmd)
            .with_context(|| format!("cannot start '{program}'"))?;
        drop(slave);
        self.child = Some(child);
        self.started = Some(Instant::now());
        trace::mark(format_args!("session {}: process started", self.claude_id));
        Ok(())
    }

    /// Prozess wartet noch in der Startreihe.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Seit wann der Prozess läuft, solange er noch nichts angezeigt hat; sonst `None`.
    pub fn starting_since(&self) -> Option<Instant> {
        self.started.filter(|_| self.first_content.is_none())
    }

    /// Der Prozess hat schon etwas angezeigt (oder ist beendet).
    pub fn has_content(&self) -> bool {
        self.first_content.is_some() || self.status == Status::Exited
    }

    pub fn screen(&self) -> MutexGuard<'_, vt100::Parser> {
        lock(&self.parser)
    }

    /// Zeitpunkt der letzten Ausgabe — bei einer wartenden Session also, seit wann sie wartet.
    pub fn last_output(&self) -> Instant {
        *lock(&self.last_output)
    }

    /// Letzte nicht leere Zeile auf dem Bildschirm — nach dem Ende meist die Fehlermeldung.
    pub fn last_line(&self) -> Option<String> {
        last_line(&self.screen().screen().contents())
    }

    pub fn application_cursor(&self) -> bool {
        self.screen().screen().application_cursor()
    }

    pub fn refresh(&mut self) {
        if self.status == Status::Exited {
            return;
        }
        let Some(child) = self.child.as_mut() else {
            self.status = Status::Starting;
            return;
        };
        // Unter ConPTY endet der Lesekanal nicht zuverlässig mit dem Prozess, daher zusätzlich try_wait.
        let exited =
            self.reader_done.load(Ordering::SeqCst) || matches!(child.try_wait(), Ok(Some(_)));
        if exited {
            trace::mark(format_args!("session {}: exited", self.claude_id));
            self.status = Status::Exited;
            return;
        }
        if self.hook_checked.is_none_or(|t| t.elapsed() >= HOOK_POLL) {
            self.hook = hooks::read(&self.claude_id);
            self.hook_checked = Some(Instant::now());
            if self.hook.is_some() && !self.hook_traced {
                self.hook_traced = true;
                trace::mark(format_args!(
                    "session {}: first hook report after {}",
                    self.claude_id,
                    since(self.started)
                ));
            }
        }
        if self.first_content.is_none() {
            if self.screen().screen().contents().trim().is_empty() {
                self.status = Status::Starting;
                return;
            }
            self.first_content = Some(Instant::now());
            trace::mark(format_args!(
                "session {}: first output after {}",
                self.claude_id,
                since(self.started)
            ));
        }
        let recently_active = lock(&self.last_output).elapsed() < WORKING_WINDOW;
        let on_screen = || waits_for_decision(&self.screen().screen().contents());
        self.status = resolve(self.hook, self.last_input, recently_active, on_screen);
    }

    /// Mausrad: scrollt den Verlauf (positiv = nach oben). Hat das Programm in der Session selbst
    /// die Maus eingeschaltet, bekommt es stattdessen das Rad-Ereignis. `col`/`row` sind 0-basiert
    /// relativ zum Fensterinhalt.
    pub fn wheel(&mut self, up: bool, col: u16, row: u16) -> Result<()> {
        let (mode, encoding) = {
            let parser = self.screen();
            let screen = parser.screen();
            (
                screen.mouse_protocol_mode(),
                screen.mouse_protocol_encoding(),
            )
        };
        if mode == vt100::MouseProtocolMode::None {
            let mut parser = lock(&self.parser);
            let offset = parser.screen().scrollback();
            let next = if up {
                offset + WHEEL_LINES
            } else {
                offset.saturating_sub(WHEEL_LINES)
            };
            parser.screen_mut().set_scrollback(next);
            return Ok(());
        }
        let button = if up { 64 } else { 65 };
        let (x, y) = (col + 1, row + 1);
        let bytes = match encoding {
            vt100::MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{x};{y}M").into_bytes(),
            // Klassische Kodierung: Werte + 32 als einzelne Bytes (nur bis Spalte/Zeile 223).
            _ => {
                let byte = |v: u16| u8::try_from(v + 32).unwrap_or(u8::MAX);
                vec![0x1b, b'[', b'M', byte(button), byte(x), byte(y)]
            }
        };
        self.write(&bytes)
    }

    /// Wie weit der Verlauf hochgescrollt ist (0 = aktuelle Ansicht).
    pub fn scroll_offset(&self) -> usize {
        self.screen().screen().scrollback()
    }

    /// Zurück zur aktuellen Ansicht, z. B. sobald getippt wird.
    pub fn scroll_to_bottom(&mut self) {
        lock(&self.parser).screen_mut().set_scrollback(0);
    }

    /// Schickt Eingabe an die Session; wie in einem Terminal springt die Ansicht dabei ans Ende.
    /// Wartet sie noch in der Startreihe, startet sie jetzt.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.start()?;
        self.scroll_to_bottom();
        self.last_input = SystemTime::now();
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn paste(&mut self, text: &str) -> Result<()> {
        if self.screen().screen().bracketed_paste() {
            let wrapped = format!("\x1b[200~{text}\x1b[201~");
            self.write(wrapped.as_bytes())
        } else {
            self.write(text.as_bytes())
        }
    }

    /// Passt PTY und Bildschirm an; ohne Größenänderung passiert nichts (kein unnötiges Neuzeichnen
    /// in der Session).
    pub fn resize(&mut self, rows: u16, cols: u16) {
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(pty_size(rows, cols));
        lock(&self.parser).set_size(non_zero(rows), non_zero(cols));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
        hooks::clear(&self.claude_id);
    }
}

/// „1234 ms“ seit `start` für den Trace.
fn since(start: Option<Instant>) -> String {
    start.map_or_else(
        || "?".to_string(),
        |s| format!("{} ms", s.elapsed().as_millis()),
    )
}

/// Zustand aus der letzten Hook-Meldung, sonst aus der Heuristik (Ausgabe-Aktivität,
/// Rückfrage-Text auf dem Bildschirm). Hooks melden nicht jeden Übergang: Eine beantwortete
/// Rückfrage meldet sich erst mit dem nächsten Tool-Ende, ein mit Esc abgebrochener Lauf gar nicht.
/// Kam nach der Meldung „wartet“ bzw. „arbeitet“ eine Eingabe, entscheidet daher wieder die
/// Heuristik bis zur nächsten Meldung. Nach „bereit“ nicht: Tippen erzeugt Ausgabe (Echo), Claude
/// arbeitet aber erst nach dem Absenden — und das meldet `UserPromptSubmit`.
fn resolve(
    hook: Option<Report>,
    last_input: SystemTime,
    recently_active: bool,
    decision_on_screen: impl FnOnce() -> bool,
) -> Status {
    let heuristic = |on_screen: bool| {
        if recently_active {
            Status::Working
        } else if on_screen {
            Status::Waiting
        } else {
            Status::Idle
        }
    };
    match hook {
        None => heuristic(decision_on_screen()),
        Some(report) => match report.status {
            Status::Waiting | Status::Working if last_input > report.at => {
                heuristic(decision_on_screen())
            }
            status => status,
        },
    }
}

fn last_line(screen: &str) -> Option<String> {
    screen
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(str::to_string)
}

/// Heuristik für eine offene Rückfrage (Berechtigungsdialog) von Claude Code.
fn waits_for_decision(screen: &str) -> bool {
    const MARKERS: [&str; 3] = ["Do you want to", "❯ 1. Yes", "Would you like to"];
    MARKERS.iter().any(|m| screen.contains(m))
}

fn shell_command(command: &str, extra_args: &[String]) -> CommandBuilder {
    // Über die Shell starten, damit auch npm-Wrapper (claude.cmd) und PATH-Auflösung funktionieren.
    if cfg!(windows) {
        // portable-pty quotet jedes Argument nach den C-Runtime-Regeln (`\"` für Anführungszeichen),
        // die cmd.exe nicht kennt. Deshalb den Befehl wortweise übergeben: Nur Argumente mit
        // Leerzeichen bekommen dann äußere Anführungszeichen, und die versteht cmd.exe.
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/C");
        cmd.args(command.split_whitespace());
        cmd.args(extra_args);
        cmd
    } else {
        let extra: String = extra_args
            .iter()
            .map(|a| format!(" {}", sh_quote(a)))
            .collect();
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", &format!("exec {command}{extra}")]);
        cmd
    }
}

/// Ein Argument für `sh -c` in einfache Anführungszeichen setzen.
fn sh_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// vt100 verlangt Größen > 0; ein winziges Fenster wird zu 1.
fn non_zero(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn report(status: Status, secs: u64) -> Option<Report> {
        Some(Report {
            status,
            at: at(secs),
        })
    }

    #[test]
    fn without_hooks_the_heuristic_decides() {
        assert_eq!(resolve(None, at(0), true, || false), Status::Working);
        assert_eq!(resolve(None, at(0), false, || true), Status::Waiting);
        assert_eq!(resolve(None, at(0), false, || false), Status::Idle);
    }

    #[test]
    fn hook_state_beats_output_and_screen_text() {
        // Tippen im Eingabefeld erzeugt Ausgabe, macht eine bereite Session aber nicht „working“.
        assert_eq!(
            resolve(report(Status::Idle, 10), at(20), true, || true),
            Status::Idle
        );
        assert_eq!(
            resolve(report(Status::Waiting, 10), at(5), false, || false),
            Status::Waiting
        );
        assert_eq!(
            resolve(report(Status::Working, 10), at(5), false, || false),
            Status::Working
        );
    }

    #[test]
    fn input_after_waiting_hands_back_to_heuristic() {
        // Freigabe erteilt, Claude arbeitet weiter.
        assert_eq!(
            resolve(report(Status::Waiting, 10), at(11), true, || false),
            Status::Working
        );
        // Abgelehnt: keine Ausgabe, keine Rückfrage mehr.
        assert_eq!(
            resolve(report(Status::Waiting, 10), at(11), false, || false),
            Status::Idle
        );
    }

    #[test]
    fn interrupted_work_without_output_is_idle() {
        assert_eq!(
            resolve(report(Status::Working, 10), at(11), false, || false),
            Status::Idle
        );
        // Vorab getippt, während Claude noch arbeitet.
        assert_eq!(
            resolve(report(Status::Working, 10), at(11), true, || false),
            Status::Working
        );
    }

    #[test]
    fn last_line_skips_trailing_blank_lines() {
        assert_eq!(
            last_line(
                "x
  Error: busy  

   
"
            )
            .as_deref(),
            Some("Error: busy")
        );
        assert_eq!(
            last_line(
                "
 
"
            ),
            None
        );
    }

    #[test]
    fn sh_quote_survives_spaces_and_quotes() {
        assert_eq!(sh_quote("/home/a b/x.json"), "'/home/a b/x.json'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
