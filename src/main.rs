mod app;
mod claude_store;
mod dirpick;
mod external;
mod hook_server;
mod hooks;
mod http;
mod install;
mod keys;
mod layout;
mod list;
mod menu;
mod paths;
mod recent;
mod selection;
mod session;
mod state;
mod term_view;
mod theme;
mod trace;
mod ui;
mod update;
mod waiting;
mod windows;

use std::{
    io::{self, Write as _},
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind,
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
    let mut first_frame = true;

    while !app.should_quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, &app))?;
        if std::mem::take(&mut first_frame) {
            trace::mark("app: first frame drawn");
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
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key)?,
                Event::Paste(text) => app.on_paste(&text)?,
                Event::Mouse(mouse) => app.on_mouse(mouse),
                Event::Resize(w, h) => app.resize(Rect::new(0, 0, w, h)),
                _ => {}
            }
            if app.should_quit {
                break;
            }
        }
    }
    app.save_cache();
    Ok(())
}

/// Höchstens so viele Ereignisse je Bild.
const MAX_EVENTS_PER_FRAME: usize = 64;
