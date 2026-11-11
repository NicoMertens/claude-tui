use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::windows::SessionId;

/// Ausgabe innerhalb dieses Fensters gilt als „arbeitet“ — Claude Code animiert seinen Spinner,
/// solange es rechnet, und schreibt nichts, wenn es auf Eingabe wartet.
const WORKING_WINDOW: Duration = Duration::from_millis(1500);
const SCROLLBACK: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Working,
    Waiting,
    Idle,
    Exited,
}

pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub status: Status,
    size: (u16, u16),
    parser: Arc<Mutex<vt100::Parser>>,
    last_output: Arc<Mutex<Instant>>,
    reader_done: Arc<AtomicBool>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Session {
    pub fn spawn(id: SessionId, name: String, command: &str, rows: u16, cols: u16) -> Result<Self> {
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols))
            .context("PTY konnte nicht geöffnet werden")?;

        let mut cmd = shell_command(command);
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("'{command}' konnte nicht gestartet werden"))?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
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

        Ok(Self {
            id,
            name,
            status: Status::Working,
            size: (rows, cols),
            parser,
            last_output,
            reader_done,
            master: pair.master,
            writer,
            child,
        })
    }

    pub fn screen(&self) -> MutexGuard<'_, vt100::Parser> {
        lock(&self.parser)
    }

    pub fn application_cursor(&self) -> bool {
        self.screen().screen().application_cursor()
    }

    pub fn refresh(&mut self) {
        if self.status == Status::Exited {
            return;
        }
        // Unter ConPTY endet der Lesekanal nicht zuverlässig mit dem Prozess, daher zusätzlich try_wait.
        let exited =
            self.reader_done.load(Ordering::SeqCst) || matches!(self.child.try_wait(), Ok(Some(_)));
        let recently_active = lock(&self.last_output).elapsed() < WORKING_WINDOW;
        let waiting = !recently_active && waits_for_decision(&self.screen().screen().contents());
        self.status = if exited {
            Status::Exited
        } else if recently_active {
            Status::Working
        } else if waiting {
            Status::Waiting
        } else {
            Status::Idle
        };
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
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
        lock(&self.parser).set_size(rows, cols);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Heuristik für eine offene Rückfrage (Berechtigungsdialog) von Claude Code.
fn waits_for_decision(screen: &str) -> bool {
    const MARKERS: [&str; 3] = ["Do you want to", "❯ 1. Yes", "Would you like to"];
    MARKERS.iter().any(|m| screen.contains(m))
}

fn shell_command(command: &str) -> CommandBuilder {
    // Über die Shell starten, damit auch npm-Wrapper (claude.cmd) und PATH-Auflösung funktionieren.
    if cfg!(windows) {
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/C", command]);
        cmd
    } else {
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", &format!("exec {command}")]);
        cmd
    }
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
