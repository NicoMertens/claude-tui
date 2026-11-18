//! Notiz-Editor (Ctrl+A t): mehrzeiliges Eingabefeld für die Notiz einer Session. Enter speichert,
//! Shift+Enter bricht die Zeile um, leerer Text oder Ctrl+D entfernt die Notiz.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    text: String,
    /// Cursor als Zeichenindex in `text`.
    cursor: usize,
}

impl NoteEditor {
    pub fn new(title: String, target: String, text: &str) -> Self {
        Self {
            title,
            target,
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter if shift || alt => self.insert("\n"),
            KeyCode::Enter => return Outcome::Save(normalize(&self.text)),
            // Ctrl+J ist ein Zeilenumbruch in Terminals, die Shift+Enter nicht unterscheiden.
            KeyCode::Char('j') if ctrl && !alt => self.insert("\n"),
            KeyCode::Char('d') if ctrl && !alt => return Outcome::Save(None),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Up => self.move_vertically(false),
            KeyCode::Down => self.move_vertically(true),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.remove_at(self.cursor);
            }
            KeyCode::Delete if self.cursor < self.len() => self.remove_at(self.cursor),
            // Ctrl+Alt ist unter Windows AltGr (@, {, [ auf deutscher Tastatur).
            KeyCode::Char(c) if !ctrl || alt => self.insert(&c.to_string()),
            _ => {}
        }
        Outcome::Stay
    }

    /// Zeilenumbrüche bleiben, Tabs werden Leerzeichen, andere Steuerzeichen fallen weg — sie
    /// verschöben beim Zeichnen die Spalten.
    pub fn paste(&mut self, text: &str) {
        let text: String = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
            .chars()
            .filter(|&c| c == '\n' || !c.is_control())
            .collect();
        self.insert(&text);
    }

    /// Zeilen des Texts; mindestens eine (leere).
    pub fn lines(&self) -> Vec<&str> {
        self.text.split('\n').collect()
    }

    /// Cursor als (Zeile, Spalte in Zeichen).
    pub fn cursor_position(&self) -> (usize, usize) {
        let before: String = self.text.chars().take(self.cursor).collect();
        let row = before.matches('\n').count();
        let col = before
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count();
        (row, col)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_index(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn insert(&mut self, s: &str) {
        let at = self.byte_index(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    fn remove_at(&mut self, chars: usize) {
        let at = self.byte_index(chars);
        self.text.remove(at);
    }

    fn line_start(&self) -> usize {
        let (_, col) = self.cursor_position();
        self.cursor - col
    }

    fn line_end(&self) -> usize {
        let rest = self.text.chars().skip(self.cursor);
        self.cursor + rest.take_while(|&c| c != '\n').count()
    }

    /// Eine Zeile hoch bzw. runter, möglichst in derselben Spalte.
    fn move_vertically(&mut self, down: bool) {
        let (row, col) = self.cursor_position();
        let lines = self.lines();
        // Über die erste bzw. letzte Zeile hinaus: an deren Anfang bzw. Ende.
        if !down && row == 0 {
            self.cursor = 0;
            return;
        }
        if down && row + 1 >= lines.len() {
            self.cursor = self.len();
            return;
        }
        let target = if down { row + 1 } else { row - 1 };
        let start: usize = lines[..target].iter().map(|l| l.chars().count() + 1).sum();
        self.cursor = start + col.min(lines[target].chars().count());
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
    fn paste_keeps_line_breaks() {
        let mut e = empty();
        e.paste("a\r\nb\tc\x07");
        assert_eq!(e.lines(), ["a", "b    c"]);
        assert_eq!(e.cursor_position(), (1, 6));
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
