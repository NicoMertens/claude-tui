mod app;
mod keys;
mod session;
mod theme;
mod ui;

use std::{io, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

type Term = Terminal<CrosstermBackend<io::Stdout>>;

fn main() -> Result<()> {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("-V" | "--version") => {
            println!("claude-tui {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("-h" | "--help") => {
            println!("Aufruf: claude-tui [BEFEHL]\n\nBEFEHL wird in jeder Session gestartet (Standard: claude).");
            return Ok(());
        }
        _ => {}
    }
    let command = arg.unwrap_or_else(|| "claude".to_string());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Nicht jedes Windows-Terminal kennt Bracketed Paste; dann kommt Einfügen als Tastendrücke an.
    let _ = execute!(stdout, EnableBracketedPaste);
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run(&mut terminal, command);

    let _ = execute!(terminal.backend_mut(), DisableBracketedPaste);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run(terminal: &mut Term, command: String) -> Result<()> {
    let size = terminal.size()?;
    let mut app = app::App::new(command, ui::pane_size(size.width, size.height))?;

    while !app.should_quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, &app))?;
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key)?,
                Event::Paste(text) => app.on_paste(&text)?,
                Event::Resize(w, h) => app.resize(ui::pane_size(w, h)),
                _ => {}
            }
        }
    }
    Ok(())
}
