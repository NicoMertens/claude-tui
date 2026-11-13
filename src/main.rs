mod app;
mod claude_store;
mod dirpick;
mod external;
mod grouppick;
mod groups;
mod hook_server;
mod hooks;
mod http;
mod install;
mod keys;
mod layout;
mod list;
mod menu;
mod mouse;
mod paths;
mod recent;
mod selection;
mod session;
mod state;
mod tags;
mod term_view;
mod theme;
mod trace;
mod ui;
mod update;
mod waiting;
mod windows;

use std::{
    io::{self, Write as _},
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};

type Term = Terminal<CrosstermBackend<io::Stdout>>;

const HELP: &str = "\
Usage:
  claude-tui [COMMAND]   start the workspace; COMMAND starts Claude Code (default: claude),
                         claude-tui appends --session-id or --resume
  claude-tui install     copy claude-tui to a fixed place and add it to your PATH
  claude-tui uninstall   remove it again (settings are kept)
  claude-tui update      install the newest release from GitHub
  claude-tui --version

Set CLAUDE_TUI_NO_UPDATE_CHECK=1 to skip the daily update check on start.
Set CLAUDE_TUI_NO_BELL=1 to keep the terminal bell silent when a session waits for you.
Set CLAUDE_TUI_TRACE=1 to log start-up and session timings to trace.log in the config folder.";

fn main() -> Result<()> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("-V" | "--version") => {
            println!("claude-tui {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("-h" | "--help") => {
            println!("{HELP}");
            return Ok(());
        }
        Some("install") => return install::install(),
        Some("uninstall") => return install::uninstall(),
        Some("update") => return update::run_cli(),
        Some("hook") => {
            hooks::run_cli(std::env::args().nth(2).as_deref());
            return Ok(());
        }
        _ => {}
    }
    let command = arg.unwrap_or_else(|| "claude".to_string());

    // Bei einem Absturz erst das Terminal zurücksetzen, dann die Meldung ausgeben — sonst bliebe
    // die Shell im Rohmodus mit Mausabfrage. Läuft auch mit `panic = "abort"`.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Nicht jedes Windows-Terminal kennt Bracketed Paste; dann kommt Einfügen als Tastendrücke an.
    let _ = execute!(stdout, EnableBracketedPaste);
    // Ohne Mausabfrage macht das Terminal aus dem Mausrad ↑/↓ — Claude blättert dann durch die
    // letzten Prompts statt durch den Verlauf.
    execute!(stdout, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run(&mut terminal, command);

    hooks::remove_settings();
    restore_terminal();
    terminal.show_cursor()?;
    result
}

/// Terminal in den Normalzustand: Maus und Bracketed Paste aus, Rohmodus aus, Hauptbildschirm.
fn restore_terminal() {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, DisableMouseCapture, DisableBracketedPaste);
    let _ = disable_raw_mode();
    let _ = execute!(stdout, LeaveAlternateScreen, crossterm::cursor::Show);
}

fn run(terminal: &mut Term, command: String) -> Result<()> {
    let size = terminal.size()?;
    let mut app = app::App::new(command, Rect::new(0, 0, size.width, size.height))?;
    // Seit dem letzten Bild hat sich etwas geändert (Ereignis, Zustand); `None` = noch kein Bild.
    let mut changed = true;
    let mut last_draw: Option<Instant> = None;

    while !app.should_quit {
        changed |= app.tick();
        let due = redraw_due(
            changed,
            app.latest_output(),
            last_draw,
            app.animating(),
            Instant::now(),
        );
        if due {
            let started = Instant::now();
            terminal.draw(|f| ui::draw(f, &app))?;
            if last_draw.is_none() {
                trace::mark("app: first frame drawn");
            }
            last_draw = Some(started);
            changed = false;
        }
        if std::mem::take(&mut app.ring_bell) {
            // BEL bewegt den Cursor nicht; nach dem Zeichnen stört es das Bild nicht.
            let backend = terminal.backend_mut();
            let _ = backend.write_all(b"\x07").and_then(|()| backend.flush());
        }
        // Alles abarbeiten, was schon ansteht (schnelles Tippen, Mausrad), statt nach jedem
        // einzelnen Ereignis neu zu zeichnen; begrenzt, damit das Bild nicht einfriert.
        let mut timeout = Duration::from_millis(16);
        for _ in 0..MAX_EVENTS_PER_FRAME {
            if !event::poll(timeout)? {
                break;
            }
            timeout = Duration::ZERO;
            changed |= handle(&mut app, event::read()?)?;
            if app.should_quit {
                break;
            }
        }
    }
    app.save_cache();
    Ok(())
}

/// Verarbeitet ein Ereignis; `true`, wenn sich dadurch etwas geändert haben kann.
fn handle(app: &mut app::App, event: Event) -> Result<bool> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Release => return Ok(false),
        Event::Key(key) if keys::pasted_char(&key).is_some() => {
            if let Some(next) = on_typed(app, key)? {
                handle(app, next)?;
            }
        }
        Event::Key(key) => app.on_key(key)?,
        Event::Paste(text) => app.on_paste(&text)?,
        // Reine Mausbewegung ändert nichts — das Terminal meldet sie ständig.
        Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved => return Ok(false),
        Event::Mouse(mouse) => app.on_mouse(mouse),
        Event::Resize(w, h) => app.resize(Rect::new(0, 0, w, h)),
        _ => return Ok(false),
    }
    Ok(true)
}

/// Windows-Konsolen kennen kein Bracketed Paste (crossterm lehnt es dort ab): Eingefügter Text kommt
/// als Folge von Tastendrücken, jeder Zeilenumbruch als Enter — Claude schickte den Text dann schon
/// nach der ersten Zeile ab. Deshalb alles sammeln, was ohne Pause ansteht, und es als eingefügten
/// Text weitergeben, wenn [`keys::is_paste_burst`] zutrifft; sonst Taste für Taste wie gewohnt.
/// Gibt das erste nicht dazugehörige Ereignis zurück.
fn on_typed(app: &mut app::App, first: event::KeyEvent) -> Result<Option<Event>> {
    let mut burst = vec![first];
    let mut chars: Vec<char> = keys::pasted_char(&first).into_iter().collect();
    let mut rest = None;
    loop {
        // Nach einem Enter kurz warten: Lange Texte liefert die Konsole stückweise.
        let wait = if chars.len() > 1 && chars.last() == Some(&'\n') {
            PASTE_GAP
        } else {
            Duration::ZERO
        };
        if !event::poll(wait)? {
            break;
        }
        match event::read()? {
            Event::Key(key)
                if key.kind == KeyEventKind::Release
                    || matches!(key.code, event::KeyCode::Modifier(_)) => {}
            Event::Key(key) => match keys::pasted_char(&key) {
                Some(c) => {
                    burst.push(key);
                    chars.push(c);
                }
                None => {
                    rest = Some(Event::Key(key));
                    break;
                }
            },
            other => {
                rest = Some(other);
                break;
            }
        }
    }
    if keys::is_paste_burst(&chars) {
        app.on_paste(&chars.into_iter().collect::<String>())?;
    } else {
        for key in burst {
            app.on_key(key)?;
        }
    }
    Ok(rest)
}

/// So lange darf nach einem Zeilenumbruch eingefügten Textes Pause sein, bevor der Rest kommt.
const PASTE_GAP: Duration = Duration::from_millis(10);

/// Höchstens so viele Ereignisse je Bild.
const MAX_EVENTS_PER_FRAME: usize = 64;
/// Bildabstand, solange sich etwas von selbst bewegt (Spinner).
const ANIMATION_FRAME: Duration = theme::SPINNER_FRAME;
/// Ohne Anlass wird spätestens nach dieser Zeit neu gezeichnet — für Uhrzeit-Anzeigen
/// („starting … 3s“, „vor 2m“) und als Sicherheitsnetz.
const HEARTBEAT: Duration = Duration::from_secs(1);

/// Neu zeichnen? Ja beim ersten Bild, nach einer Änderung, nach neuer Ausgabe einer Session
/// (`latest_output` nach dem Beginn des letzten Bildes) und wenn Animation bzw. Uhr es verlangen.
/// Sonst nicht: Jedes Bild setzt alle Zellen der Fenster neu, bei 60 Bildern je Sekunde kostete
/// das auch im Leerlauf spürbar Rechenzeit.
fn redraw_due(
    changed: bool,
    latest_output: Option<Instant>,
    last_draw: Option<Instant>,
    animating: bool,
    now: Instant,
) -> bool {
    let Some(last) = last_draw else {
        return true;
    };
    let interval = if animating {
        ANIMATION_FRAME
    } else {
        HEARTBEAT
    };
    changed
        || latest_output.is_some_and(|output| output >= last)
        || now.saturating_duration_since(last) >= interval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_changes_and_output_force_a_redraw() {
        let last = Instant::now();
        let soon = last + Duration::from_millis(20);
        assert!(redraw_due(false, None, None, false, soon), "erstes Bild");
        assert!(redraw_due(true, None, Some(last), false, soon), "Änderung");
        assert!(
            redraw_due(
                false,
                Some(last + Duration::from_millis(5)),
                Some(last),
                false,
                soon
            ),
            "Ausgabe nach dem letzten Bild"
        );
        assert!(
            !redraw_due(
                false,
                Some(last - Duration::from_millis(5)),
                Some(last),
                false,
                soon
            ),
            "Ausgabe schon gezeichnet"
        );
    }

    #[test]
    fn idle_screen_redraws_only_on_heartbeat_or_animation() {
        let last = Instant::now();
        let soon = last + Duration::from_millis(200);
        assert!(
            !redraw_due(false, None, Some(last), false, soon),
            "Leerlauf"
        );
        assert!(redraw_due(false, None, Some(last), true, soon), "Spinner");
        assert!(
            redraw_due(false, None, Some(last), false, last + HEARTBEAT),
            "Uhr"
        );
    }
}
