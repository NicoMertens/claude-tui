//! Hilfeseite (Ctrl+A h): erklärt Aufbau, Zustände, Tasten, Menü und Maus. Die Menüpunkte kommen
//! aus derselben Tabelle wie das Menü selbst ([`menu::ENTRIES`]), damit beide nicht auseinanderlaufen.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Span;

use crate::{keys, menu};

/// Breite der Tastenspalte (Anzeigespalten).
pub const KEYS_WIDTH: usize = 14;
/// Bevorzugte Breite des Inhalts; schmaler, wenn der Platz fehlt.
pub const PREFERRED_WIDTH: usize = 78;
/// Zeilen je Mausrad-Raste.
const WHEEL_STEP: usize = 3;

/// Baustein des Hilfetexts vor dem Umbruch.
enum Row {
    Heading(&'static str),
    Text(&'static str),
    Key(&'static str, &'static str),
}

use Row::{Heading, Key, Text};

const OVERVIEW: &[Row] = &[
    Heading("claude-tui"),
    Text(
        "Runs several Claude Code sessions side by side: the session list on the left, up to 4 \
         windows on the right. The focused area has a blue frame.",
    ),
    Heading("Session list"),
    Text(
        "All saved Claude Code sessions. Your own groups (📌) come first, sorted alphabetically, \
         then the sessions by working directory (📁), most recent first. Running sessions lead \
         each section.",
    ),
    Text(
        "Each row shows state, quick number (#3), title, window number ([1]) and the time since \
         the last activity. A /color set in Claude colours the title.",
    ),
    Heading("Windows"),
    Text(
        "Numbered by position. The window bar shows number, title and state. The session of a \
         closed or replaced window keeps running in the background and stays in the list.",
    ),
    Heading("States"),
    Key("✻ working", "Claude works on a prompt or uses a tool"),
    Key("▲ waiting", "Claude waits for your permission or input"),
    Key("● ready", "Claude is done and waits for the next prompt"),
    Key("○ saved", "Not running; opening resumes it"),
    Key("◇ ext", "Runs outside claude-tui; opening asks first"),
];

const KEYS: &[Row] = &[
    Heading("Everywhere"),
    Key("Ctrl+A", "Open the menu; Ctrl+A again or Esc closes it"),
    Key(
        "Ctrl+Arrow",
        "Move the focus to the neighbouring window or the session list",
    ),
    Key(
        "Alt+1…9",
        "Give the focused session a quick number; the same number again removes it",
    ),
    Key("Ctrl+1…9", "Show the session with this quick number"),
    Heading("Window focused"),
    Key("Other keys", "Go to the session"),
    Key("Shift+Enter", "New line in Claude's input"),
    Key("Alt+← / →", "Jump by word in Claude's input"),
    Heading("Session list focused"),
    Key("↑ / ↓", "Select a session"),
    Key(
        "Enter",
        "Go to its window, or pick a window (a saved session resumes)",
    ),
    Key("1…4", "Put the session into this window"),
    Key("Space", "Mark or unmark the session and move on"),
    Key("Esc", "Clear all marks"),
    Heading("Dialogs"),
    Key("Enter / Esc", "Confirm / cancel"),
    Key(
        "y / n",
        "Yes / no when asked; deleting several sessions, Enter means no",
    ),
    Key(
        "↑ / ↓",
        "Pick an entry (directory: take a recently used one)",
    ),
    Key("Tab", "Directory: complete the folder name"),
    Key(
        "1…4",
        "Window picker: replace this window, or open a new one",
    ),
    Key("d / w / m", "Delete older than a day / week / month"),
    Key(
        "Typing",
        "Directory or group name; in groups it also filters",
    ),
];

const MENU_INTRO: &[Row] = &[
    Heading("Menu (Ctrl+A)"),
    Text(
        "Opens over the focused area. Which items appear depends on the focus and on what is \
         possible right now. Pick with the key, or with ↑ / ↓ and Enter.",
    ),
];

const MOUSE: &[Row] = &[
    Heading("Mouse"),
    Key("Click", "Focus a window, or select a session in the list"),
    Key(
        "Double-click",
        "On a session in the list: like Enter; in a window: copy a word",
    ),
    Key("Wheel", "Scroll a window's history"),
    Key(
        "Drag",
        "Select text; in fullscreen mode Claude selects and copies, otherwise claude-tui",
    ),
    Key(
        "Alt+drag",
        "Always claude-tui's selection, copied on release",
    ),
    Key(
        "Shift+drag",
        "The terminal's own selection (whole screen lines)",
    ),
    Key("Click outside", "Close the menu, a dialog or this help"),
];

/// Fertig umbrochene Zeile der Hilfe.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    Blank,
    Heading(String),
    Text(String),
    /// Tastenzeile; Folgezeilen eines umbrochenen Texts haben leere `keys`.
    Key {
        keys: String,
        text: String,
    },
}

/// Die ganze Hilfe, umbrochen auf `width` Anzeigespalten.
pub fn lines(width: usize) -> Vec<Line> {
    let mut out = Vec::new();
    for rows in [OVERVIEW, KEYS, MENU_INTRO] {
        push_rows(&mut out, rows, width);
    }
    for entry in menu::ENTRIES {
        push_key(&mut out, &entry.keys(), entry.help, width);
    }
    push_rows(&mut out, MOUSE, width);
    out
}

fn push_rows(out: &mut Vec<Line>, rows: &[Row], width: usize) {
    for row in rows {
        match row {
            Heading(title) => {
                if !out.is_empty() {
                    out.push(Line::Blank);
                }
                out.push(Line::Heading((*title).to_string()));
            }
            Text(text) => out.extend(wrap(text, width).into_iter().map(Line::Text)),
            Key(keys, text) => push_key(out, keys, text, width),
        }
    }
}

/// Tastenzeile; der Text wird neben der Tastenspalte umbrochen.
fn push_key(out: &mut Vec<Line>, keys: &str, text: &str, width: usize) {
    let parts = wrap(text, width.saturating_sub(KEYS_WIDTH));
    for (i, part) in parts.into_iter().enumerate() {
        let keys = if i == 0 {
            keys.to_string()
        } else {
            String::new()
        };
        out.push(Line::Key { keys, text: part });
    }
}

/// Anzeigebreite (Emojis wie 📌 belegen zwei Spalten).
pub fn width_of(text: &str) -> usize {
    Span::raw(text).width()
}

/// Bricht an Leerzeichen auf höchstens `width` Spalten um; ein zu langes Wort steht allein.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && width_of(&current) + 1 + width_of(word) > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Ergebnis eines Tastendrucks in der Hilfe.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
}

/// Zustand der offenen Hilfe. Wie weit sich scrollen lässt, steht erst beim Zeichnen fest
/// (Breite und Höhe des Overlays); [Help::view] merkt es sich deshalb in `Cell`s.
#[derive(Debug, Default)]
pub struct Help {
    scroll: usize,
    max: Cell<usize>,
    page: Cell<usize>,
}

impl Help {
    /// Erste sichtbare Zeile für `total` Zeilen bei `height` sichtbaren; merkt sich die Grenzen.
    pub fn view(&self, total: usize, height: usize) -> usize {
        self.max.set(total.saturating_sub(height));
        self.page.set(height.max(1));
        self.scroll.min(self.max.get())
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Outcome {
        let max = self.max.get();
        let page = self.page.get();
        // Stand nach einer Verkleinerung des Terminals erst auf die Grenze holen.
        let scroll = self.scroll.min(max);
        self.scroll = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Outcome::Close,
            _ if keys::is_prefix(key) => return Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => (scroll + 1).min(max),
            KeyCode::PageUp => scroll.saturating_sub(page),
            KeyCode::PageDown | KeyCode::Char(' ') => (scroll + page).min(max),
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => scroll,
        };
        Outcome::Stay
    }

    pub fn wheel(&mut self, up: bool) {
        let scroll = self.scroll.min(self.max.get());
        self.scroll = if up {
            scroll.saturating_sub(WHEEL_STEP)
        } else {
            (scroll + WHEEL_STEP).min(self.max.get())
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn wrap_breaks_at_spaces_and_counts_emoji_as_two_columns() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("📌 ab", 4), ["📌", "ab"]);
        assert_eq!(wrap("toolongword x", 4), ["toolongword", "x"]);
    }

    #[test]
    fn lines_fit_the_width_and_list_every_menu_entry() {
        let width = 40;
        let lines = lines(width);
        for line in &lines {
            let used = match line {
                Line::Blank => 0,
                Line::Heading(t) | Line::Text(t) => width_of(t),
                Line::Key { text, .. } => KEYS_WIDTH + width_of(text),
            };
            assert!(used <= width, "{line:?}");
        }
        for entry in menu::ENTRIES {
            assert!(lines
                .iter()
                .any(|l| matches!(l, Line::Key { keys, .. } if *keys == entry.keys())));
        }
    }

    #[test]
    fn keys_fit_their_column() {
        for line in lines(PREFERRED_WIDTH) {
            if let Line::Key { keys, .. } = line {
                assert!(width_of(&keys) < KEYS_WIDTH, "{keys}");
            }
        }
    }

    #[test]
    fn scrolling_stays_within_the_page() {
        let mut help = Help::default();
        assert_eq!(help.view(100, 30), 0);
        help.on_key(&key(KeyCode::Up));
        assert_eq!(help.view(100, 30), 0);
        help.on_key(&key(KeyCode::PageDown));
        assert_eq!(help.view(100, 30), 30);
        help.on_key(&key(KeyCode::End));
        assert_eq!(help.view(100, 30), 70);
        help.on_key(&key(KeyCode::Down));
        assert_eq!(help.view(100, 30), 70);
        help.wheel(true);
        assert_eq!(help.view(100, 30), 67);
        help.on_key(&key(KeyCode::Home));
        assert_eq!(help.view(100, 30), 0);
        assert_eq!(help.on_key(&key(KeyCode::Esc)), Outcome::Close);
    }

    #[test]
    fn everything_fits_means_no_scrolling() {
        let mut help = Help::default();
        assert_eq!(help.view(10, 30), 0);
        help.on_key(&key(KeyCode::End));
        assert_eq!(help.view(10, 30), 0);
    }
}
