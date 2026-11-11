mod app;
mod claude_store;
mod dirpick;
mod keys;
mod layout;
mod list;
mod menu;
mod paths;
mod recent;
mod session;
mod state;
mod theme;
mod ui;
mod windows;

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
            println!("Aufruf: claude-tui [BEFEHL]\n\nBEFEHL startet Claude Code (Standard: claude); claude-tui hängt --session-id bzw. --resume an.");
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
    let mut app = app::App::new(command, ui::windows_area(size.width, size.height))?;

    while !app.should_quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, &app))?;
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key)?,
                Event::Paste(text) => app.on_paste(&text)?,
                Event::Resize(w, h) => app.resize(ui::windows_area(w, h)),
                _ => {}
            }
        }
    }
    Ok(())
}
