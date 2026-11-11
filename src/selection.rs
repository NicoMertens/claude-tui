//! Textmarkierung mit der Maus innerhalb eines Fensters.
//!
//! Koordinaten sind Zeile/Spalte im sichtbaren Bildschirm der Session (0-basiert). Die Markierung
//! läuft in Leserichtung von `anchor` (Maus gedrückt) bis `head` (aktuelle Mausposition), beide
//! Zellen eingeschlossen.

use crate::windows::SessionId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cell {
    pub row: u16,
    pub col: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub session: SessionId,
    pub anchor: Cell,
    pub head: Cell,
}

impl Selection {
    pub fn new(session: SessionId, at: Cell) -> Self {
        Self {
            session,
            anchor: at,
            head: at,
        }
    }

    /// Anfang und Ende in Leserichtung.
    pub fn range(&self) -> (Cell, Cell) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Ein bloßer Klick (ohne Ziehen) markiert nichts.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Text der Markierung; Zeilenenden setzt `vt100`, Leerraum am Zeilenende fällt weg.
    pub fn text(&self, screen: &vt100::Screen) -> String {
        let (start, end) = self.range();
        screen.contents_between(start.row, start.col, end.row, end.col.saturating_add(1))
    }
}

/// Liegt `cell` im Bereich `range` (Leserichtung, beide Enden eingeschlossen)?
pub fn contains(range: (Cell, Cell), cell: Cell) -> bool {
    range.0 <= cell && cell <= range.1
}

/// Wort unter der Zelle (zusammenhängende Nicht-Leerzeichen) als Markierung — für Doppelklick.
pub fn word_at(session: SessionId, screen: &vt100::Screen, at: Cell) -> Option<Selection> {
    let (_, cols) = screen.size();
    let is_word = |col: u16| {
        screen
            .cell(at.row, col)
            .is_some_and(|c| c.has_contents() && !c.contents().trim().is_empty())
    };
    if !is_word(at.col) {
        return None;
    }
    let mut start = at.col;
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    let mut end = at.col;
    while end + 1 < cols.get() && is_word(end + 1) {
        end += 1;
    }
    Some(Selection {
        session,
        anchor: Cell {
            row: at.row,
            col: start,
        },
        head: Cell {
            row: at.row,
            col: end,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU16;

    fn screen(text: &[u8]) -> vt100::Parser {
        let size = |n| NonZeroU16::new(n).expect("> 0");
        let mut parser = vt100::Parser::new(size(3), size(20), 0);
        parser.process(text);
        parser
    }

    fn cell(row: u16, col: u16) -> Cell {
        Cell { row, col }
    }

    #[test]
    fn backwards_drag_is_normalized() {
        let mut s = Selection::new(1, cell(2, 5));
        s.head = cell(0, 3);
        assert_eq!(s.range(), (cell(0, 3), cell(2, 5)));
        assert!(contains(s.range(), cell(1, 0)));
        assert!(!contains(s.range(), cell(2, 6)));
    }

    #[test]
    fn text_spans_lines_and_includes_end_cell() {
        let parser = screen(b"hallo welt\r\nzweite zeile");
        let mut s = Selection::new(1, cell(0, 6));
        s.head = cell(1, 5);
        assert_eq!(s.text(parser.screen()), "welt\nzweite");
    }

    #[test]
    fn double_click_selects_word() {
        let parser = screen(b"cargo build --release");
        let s = word_at(1, parser.screen(), cell(0, 8)).expect("Wort");
        assert_eq!(s.text(parser.screen()), "build");
        assert!(
            word_at(1, parser.screen(), cell(0, 5)).is_none(),
            "Leerzeichen"
        );
    }
}
