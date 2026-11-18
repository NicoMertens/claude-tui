//! Notiz-Editor (Ctrl+A t): mehrzeiliges Eingabefeld für die Notiz einer Session. Enter speichert,
//! Shift+Enter bricht die Zeile um, leerer Text oder Ctrl+D entfernt die Notiz.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::textbuf::TextBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Cancel,
    /// Neue Notiz; `None` = entfernen.
    Save(Option<String>),
}

#[derive(Debug)]
pub struct NoteEditor {
    pub title: String,
    /// Claude-ID der Session, deren Notiz bearbeitet wird (beim Öffnen festgelegt).
    pub target: String,
    buf: TextBuf,
}

impl NoteEditor {
    pub fn new(title: String, target: String, text: &str) -> Self {
        Self {
            title,
            target,
            buf: TextBuf::new(text),
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter if shift || alt => self.buf.insert(
                "
",
            ),
            KeyCode::Enter => return Outcome::Save(normalize(self.buf.text())),
            // Ctrl+J ist ein Zeilenumbruch in Terminals, die Shift+Enter nicht unterscheiden.
            KeyCode::Char('j') if ctrl && !alt => self.buf.insert(
                "
",
            ),
            KeyCode::Char('d') if ctrl && !alt => return Outcome::Save(None),
            KeyCode::Home => self.buf.set_cursor(self.buf.line_start()),
            KeyCode::End => self.buf.set_cursor(self.buf.line_end()),
            KeyCode::Up => self.buf.move_vertically(false),
            KeyCode::Down => self.buf.move_vertically(true),
            _ => {
                self.buf.edit(key);
            }
        }
        Outcome::Stay
    }

    pub fn paste(&mut self, text: &str) {
        self.buf.paste(text);
    }

    /// Zeilen des Texts; mindestens eine (leere).
    pub fn lines(&self) -> Vec<&str> {
        self.buf.lines()
    }

    /// Cursor als (Zeile, Spalte in Zeichen).
    pub fn cursor_position(&self) -> (usize, usize) {
        self.buf.cursor_position()
    }
}

/// Entfernt Leerraum am Zeilenende sowie Leerzeilen am Anfang und Ende; leer = keine Notiz.
pub fn normalize(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let first = lines.iter().position(|l| !l.is_empty())?;
    let last = lines.iter().rposition(|l| !l.is_empty())?;
    Some(lines[first..=last].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(editor: &mut NoteEditor, text: &str) {
        for c in text.chars() {
            editor.on_key(&key(KeyCode::Char(c)));
        }
    }

    fn empty() -> NoteEditor {
        NoteEditor::new("Note".into(), "id".into(), "")
    }

    #[test]
    fn enter_saves_and_shift_enter_breaks_the_line() {
        let mut e = empty();
        type_text(&mut e, "wartet");
        e.on_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        type_text(&mut e, "auf Review");
        assert_eq!(e.lines(), ["wartet", "auf Review"]);
        assert_eq!(
            e.on_key(&key(KeyCode::Enter)),
            Outcome::Save(Some("wartet\nauf Review".into()))
        );
    }

    #[test]
    fn empty_text_or_ctrl_d_removes_the_note() {
        let mut e = NoteEditor::new("Note".into(), "id".into(), "x");
        e.on_key(&key(KeyCode::Backspace));
        type_text(&mut e, "  ");
        assert_eq!(e.on_key(&key(KeyCode::Enter)), Outcome::Save(None));
        let mut e = NoteEditor::new("Note".into(), "id".into(), "x");
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(e.on_key(&ctrl_d), Outcome::Save(None));
        assert_eq!(e.on_key(&key(KeyCode::Esc)), Outcome::Cancel);
    }

    #[test]
    fn editing_existing_text_in_the_middle() {
        let mut e = NoteEditor::new("Note".into(), "id".into(), "äbc\ndef");
        assert_eq!(e.cursor_position(), (1, 3));
        e.on_key(&key(KeyCode::Up));
        assert_eq!(e.cursor_position(), (0, 3));
        e.on_key(&key(KeyCode::Home));
        e.on_key(&key(KeyCode::Delete));
        type_text(&mut e, "A");
        e.on_key(&key(KeyCode::End));
        e.on_key(&key(KeyCode::Backspace));
        assert_eq!(e.lines(), ["Ab", "def"]);
        e.on_key(&key(KeyCode::Down));
        assert_eq!(e.cursor_position(), (1, 2));
        e.on_key(&key(KeyCode::Down));
        assert_eq!(e.cursor_position(), (1, 3), "letzte Zeile: ans Ende");
    }

    #[test]
    fn altgr_character_is_typed() {
        let mut e = empty();
        let altgr = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        e.on_key(&altgr);
        assert_eq!(e.lines(), ["@"]);
    }

    #[test]
    fn normalize_trims_blank_edges() {
        assert_eq!(normalize("\n  a  \n\nb\n\n"), Some("  a\n\nb".into()));
        assert_eq!(normalize(" \n \n"), None);
    }
}
