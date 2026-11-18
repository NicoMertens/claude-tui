//! Text mit Cursor für die Eingabefelder (Notiz-Editor, Notizblock): Einfügen, Löschen, Cursor
//! links/rechts und das Bereinigen eingefügten Texts. Wie Zeilen angezeigt und Up/Down/Home/End
//! gedeutet werden, entscheidet das jeweilige Feld.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Default)]
pub struct TextBuf {
    text: String,
    /// Cursor als Zeichenindex in `text`.
    cursor: usize,
}

impl TextBuf {
    /// Text mit dem Cursor am Ende.
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.len());
    }

    /// Länge in Zeichen.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Tasten, die in allen Feldern gleich wirken: Zeichen, Backspace, Delete, Cursor links und
    /// rechts. `true`, wenn die Taste damit erledigt ist.
    pub fn edit(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.remove_at(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.len() {
                    self.remove_at(self.cursor);
                }
            }
            // Ctrl+Alt ist unter Windows AltGr (@, {, [ auf deutscher Tastatur).
            KeyCode::Char(c) if !ctrl || alt => self.insert(&c.to_string()),
            _ => return false,
        }
        true
    }

    /// Zeilenumbrüche bleiben, Tabs werden Leerzeichen, andere Steuerzeichen fallen weg — sie
    /// verschöben beim Zeichnen die Spalten.
    pub fn paste(&mut self, text: &str) {
        self.insert(&sanitize(text));
    }

    pub fn insert(&mut self, s: &str) {
        let at = self.byte_index(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
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

    /// Anfang der Zeile des Cursors.
    pub fn line_start(&self) -> usize {
        let (_, col) = self.cursor_position();
        self.cursor - col
    }

    /// Ende der Zeile des Cursors (vor dem Zeilenumbruch).
    pub fn line_end(&self) -> usize {
        let rest = self.text.chars().skip(self.cursor);
        self.cursor + rest.take_while(|&c| c != '\n').count()
    }

    /// Eine Zeile hoch bzw. runter, möglichst in derselben Spalte; über die erste bzw. letzte
    /// Zeile hinaus an deren Anfang bzw. Ende.
    pub fn move_vertically(&mut self, down: bool) {
        let (row, col) = self.cursor_position();
        let lines = self.lines();
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

    fn byte_index(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn remove_at(&mut self, chars: usize) {
        let at = self.byte_index(chars);
        self.text.remove(at);
    }
}

/// Bereinigt Text für ein Eingabefeld: Zeilenumbrüche werden `\n`, Tabs vier Leerzeichen, andere
/// Steuerzeichen fallen weg.
pub fn sanitize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
        .chars()
        .filter(|&c| c == '\n' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_keys_insert_and_delete_around_the_cursor() {
        let mut b = TextBuf::new("äc");
        b.edit(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        b.edit(&KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(b.text(), "äbc");
        b.edit(&KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        b.edit(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(b.text(), "ä");
        assert!(!b.edit(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn paste_keeps_line_breaks_and_drops_control_chars() {
        let mut b = TextBuf::default();
        b.paste("a\r\nb\tc\x07");
        assert_eq!(b.lines(), ["a", "b    c"]);
        assert_eq!(b.cursor_position(), (1, 6));
    }
}
