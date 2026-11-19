//! Notizübersicht (`Ctrl+A m`): die Notizen aller Sessions untereinander, je Session eine
//! Kopfzeile wie in der Sessionliste und darunter die Notiz. Die Notizen selbst gehören der App
//! (`App::notes`); hier liegen nur Auswahl, Scrollen und die gemeinsame Zeilenaufteilung für
//! Zeichnen, Scrollen und Klicks.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{app::DOUBLE_CLICK, notepad};

/// Einzug der Notizzeilen unter der Kopfzeile.
pub const INDENT: usize = 2;
/// Zeilen je Mausrad-Raste.
const WHEEL_ROWS: usize = 3;

/// Ein Eintrag: Claude-ID der Session und ihre Notiz, in der Reihenfolge der Sessionliste.
pub type Item<'a> = (&'a str, &'a str);

/// Eine Bildschirmzeile der Übersicht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// Kopfzeile des Eintrags `index`.
    Header(usize),
    /// Eine (umbrochene) Notizzeile des Eintrags `index`, ohne Einzug.
    Note(usize, String),
}

impl Row {
    pub fn item(&self) -> usize {
        match self {
            Self::Header(i) | Self::Note(i, _) => *i,
        }
    }
}

/// Sichtbarer Ausschnitt für eine Fläche.
#[derive(Debug, PartialEq, Eq)]
pub struct Rendered {
    pub rows: Vec<Row>,
    /// Index des gewählten Eintrags.
    pub selected: Option<usize>,
    pub total: usize,
    pub scroll: usize,
}

/// Was die App tun soll: die Session `claude_id` zeigen bzw. öffnen, in Fenster `slot` oder
/// (`None`) wie Enter in der Sessionliste.
#[derive(Debug, PartialEq, Eq)]
pub struct Open {
    pub claude_id: String,
    pub slot: Option<usize>,
}

#[derive(Debug, Default)]
pub struct NoteList {
    /// Claude-ID des gewählten Eintrags; die Liste ordnet sich laufend neu, ein Index wanderte mit.
    selected: Option<String>,
    /// Letzter Index der Auswahl: Verschwindet ihre Notiz, gilt der Eintrag an dieser Stelle.
    last_index: usize,
    scroll: usize,
    /// Breite × Höhe der Fläche.
    view: (usize, usize),
    last_click: Option<(Instant, String)>,
}

impl NoteList {
    pub fn set_view(&mut self, width: usize, height: usize) {
        self.view = (width, height);
    }

    /// Index des gewählten Eintrags in `items`.
    pub fn selected_index(&self, items: &[Item]) -> Option<usize> {
        if items.is_empty() {
            return None;
        }
        let found = self
            .selected
            .as_deref()
            .and_then(|id| items.iter().position(|&(i, _)| i == id));
        Some(found.unwrap_or(self.last_index.min(items.len() - 1)))
    }

    pub fn on_key(&mut self, key: &KeyEvent, items: &[Item]) -> Option<Open> {
        let current = self.selected_index(items)?;
        let page = self.view.1.max(2) as isize / 2;
        match key.code {
            KeyCode::Up => self.select(items, current.saturating_sub(1)),
            KeyCode::Down => self.select(items, current + 1),
            KeyCode::PageUp => self.select(items, current.saturating_add_signed(-page)),
            KeyCode::PageDown => self.select(items, current + page as usize),
            KeyCode::Home => self.select(items, 0),
            KeyCode::End => self.select(items, items.len() - 1),
            KeyCode::Enter => return Some(self.open(items, current, None)),
            KeyCode::Char(d @ '1'..='4')
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let slot = d as usize - '1' as usize;
                return Some(self.open(items, current, Some(slot)));
            }
            _ => {}
        }
        None
    }

    /// Mausrad: verschiebt nur die Ansicht, die Auswahl bleibt.
    pub fn wheel(&mut self, up: bool, items: &[Item]) {
        let total = layout(items, self.view.0).len();
        let max = total.saturating_sub(self.view.1.max(1));
        let scroll = self.scroll.min(max);
        self.scroll = if up {
            scroll.saturating_sub(WHEEL_ROWS)
        } else {
            (scroll + WHEEL_ROWS).min(max)
        };
    }

    /// Klick auf Zeile `row` der Fläche wählt den Eintrag dort; ein zweiter Klick auf denselben
    /// kurz danach öffnet ihn wie Enter.
    pub fn click(&mut self, row: usize, items: &[Item], now: Instant) -> Option<Open> {
        let rows = layout(items, self.view.0);
        // Gezeichnet wird mit begrenztem Scrollen (siehe `render`); kürzere Notizen seit dem
        // letzten Scrollen verschieben sonst den Treffer.
        let scroll = self.scroll.min(rows.len().saturating_sub(self.view.1));
        let index = rows.get(scroll + row).map(Row::item)?;
        let id = items[index].0.to_string();
        let double = self
            .last_click
            .as_ref()
            .is_some_and(|(at, last)| *last == id && now.duration_since(*at) < DOUBLE_CLICK);
        self.select(items, index);
        if double {
            self.last_click = None;
            return Some(self.open(items, index, None));
        }
        self.last_click = Some((now, id));
        None
    }

    /// Sichtbare Zeilen für eine Fläche von `width` × `height`.
    pub fn render(&self, items: &[Item], width: usize, height: usize) -> Rendered {
        let rows = layout(items, width);
        let total = rows.len();
        let scroll = self.scroll.min(total.saturating_sub(height));
        Rendered {
            rows: rows.into_iter().skip(scroll).take(height).collect(),
            selected: self.selected_index(items),
            total,
            scroll,
        }
    }

    fn open(&mut self, items: &[Item], index: usize, slot: Option<usize>) -> Open {
        self.select(items, index);
        Open {
            claude_id: items[index].0.to_string(),
            slot,
        }
    }

    fn select(&mut self, items: &[Item], index: usize) {
        let Some(last) = items.len().checked_sub(1) else {
            return;
        };
        let index = index.min(last);
        self.selected = Some(items[index].0.to_string());
        self.last_index = index;
        self.follow(items, index);
    }

    /// Scrollt so, dass der Eintrag `index` möglichst ganz sichtbar ist (mindestens seine
    /// Kopfzeile).
    fn follow(&mut self, items: &[Item], index: usize) {
        let height = self.view.1.max(1);
        let rows = layout(items, self.view.0);
        let mut own = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.item() == index)
            .map(|(i, _)| i);
        let Some(top) = own.next() else {
            return;
        };
        let bottom = own.next_back().unwrap_or(top) + 1;
        if bottom > self.scroll + height {
            self.scroll = bottom - height;
        }
        if top < self.scroll {
            self.scroll = top;
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
    }
}

/// Alle Zeilen für die Breite `width`: je Eintrag die Kopfzeile, dann die Notiz, umbrochen auf
/// die Breite ohne Einzug.
pub fn layout(items: &[Item], width: usize) -> Vec<Row> {
    let text_width = width.saturating_sub(INDENT).max(1);
    let mut rows = Vec::new();
    for (index, &(_, note)) in items.iter().enumerate() {
        rows.push(Row::Header(index));
        let chars: Vec<char> = note.chars().collect();
        for r in notepad::wrap(note, text_width) {
            rows.push(Row::Note(index, chars[r.start..r.end].iter().collect()));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn view(width: usize, height: usize) -> NoteList {
        let mut list = NoteList::default();
        list.set_view(width, height);
        list
    }

    const ITEMS: [Item; 3] = [("a", "one"), ("b", "two\nlines"), ("c", "three")];

    #[test]
    fn layout_puts_header_then_wrapped_note_and_gaps_between() {
        let items = [("a", "aaa bbb"), ("b", "x\ny")];
        assert_eq!(
            layout(&items, 6),
            [
                Row::Header(0),
                Row::Note(0, "aaa ".into()),
                Row::Note(0, "bbb".into()),
                Row::Header(1),
                Row::Note(1, "x".into()),
                Row::Note(1, "y".into()),
            ]
        );
    }

    #[test]
    fn arrows_move_the_selection_within_bounds() {
        let mut list = view(20, 10);
        assert_eq!(list.selected_index(&ITEMS), Some(0));
        list.on_key(&key(KeyCode::Up), &ITEMS);
        assert_eq!(list.selected_index(&ITEMS), Some(0));
        list.on_key(&key(KeyCode::Down), &ITEMS);
        list.on_key(&key(KeyCode::Down), &ITEMS);
        list.on_key(&key(KeyCode::Down), &ITEMS);
        assert_eq!(list.selected_index(&ITEMS), Some(2));
        list.on_key(&key(KeyCode::Home), &ITEMS);
        assert_eq!(list.selected_index(&ITEMS), Some(0));
        list.on_key(&key(KeyCode::End), &ITEMS);
        assert_eq!(list.selected_index(&ITEMS), Some(2));
    }

    #[test]
    fn selection_follows_the_session_when_the_order_changes() {
        let mut list = view(20, 10);
        list.on_key(&key(KeyCode::Down), &ITEMS);
        let reordered = [ITEMS[1], ITEMS[0], ITEMS[2]];
        assert_eq!(list.selected_index(&reordered), Some(0));
        // Ohne die Notiz bleibt die Auswahl an ihrer Stelle.
        let without = [ITEMS[0], ITEMS[2]];
        assert_eq!(list.selected_index(&without), Some(1));
        assert_eq!(list.selected_index(&[]), None);
    }

    #[test]
    fn enter_and_digits_open_the_selected_session() {
        let mut list = view(20, 10);
        list.on_key(&key(KeyCode::Down), &ITEMS);
        let open = |claude_id: &str, slot| {
            Some(Open {
                claude_id: claude_id.into(),
                slot,
            })
        };
        assert_eq!(list.on_key(&key(KeyCode::Enter), &ITEMS), open("b", None));
        assert_eq!(
            list.on_key(&key(KeyCode::Char('3')), &ITEMS),
            open("b", Some(2))
        );
        assert_eq!(list.on_key(&key(KeyCode::Char('5')), &ITEMS), None);
        let alt = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT);
        assert_eq!(list.on_key(&alt, &ITEMS), None);
        assert_eq!(list.on_key(&key(KeyCode::Enter), &[]), None);
    }

    #[test]
    fn view_follows_the_selection() {
        // Zeilen: a(0,1) b(2,3,4) c(5,6)
        let mut list = view(20, 3);
        list.on_key(&key(KeyCode::End), &ITEMS);
        let shown = list.render(&ITEMS, 20, 3);
        assert_eq!(shown.scroll, 4);
        assert_eq!(
            shown.rows[1..],
            [Row::Header(2), Row::Note(2, "three".into())]
        );
        list.on_key(&key(KeyCode::Up), &ITEMS);
        assert_eq!(list.render(&ITEMS, 20, 3).scroll, 2);
        list.on_key(&key(KeyCode::Up), &ITEMS);
        assert_eq!(list.render(&ITEMS, 20, 3).scroll, 0);
    }

    #[test]
    fn click_selects_and_a_second_click_opens() {
        let mut list = view(20, 10);
        let now = Instant::now();
        assert_eq!(list.click(2, &ITEMS, now), None);
        assert_eq!(list.selected_index(&ITEMS), Some(1));
        let open = list.click(3, &ITEMS, now + Duration::from_millis(100));
        assert_eq!(open.map(|o| o.claude_id), Some("b".into()));
        assert_eq!(list.click(5, &ITEMS, now + DOUBLE_CLICK * 2), None);
        assert_eq!(list.selected_index(&ITEMS), Some(2));
        assert_eq!(list.click(9, &ITEMS, now + DOUBLE_CLICK * 3), None);
    }

    #[test]
    fn wheel_scrolls_within_the_rows() {
        let mut list = view(20, 4);
        list.wheel(false, &ITEMS);
        list.wheel(false, &ITEMS);
        assert_eq!(list.render(&ITEMS, 20, 4).scroll, 3);
        list.wheel(true, &ITEMS);
        assert_eq!(list.render(&ITEMS, 20, 4).scroll, 0);
    }

    #[test]
    fn click_and_wheel_use_the_scroll_that_is_drawn() {
        // Zeilen von ITEMS: 7, Ansicht 3 → am Ende ab Zeile 4 gescrollt.
        let mut list = view(20, 3);
        list.on_key(&key(KeyCode::End), &ITEMS);
        // Danach nur noch 5 Zeilen: gezeichnet ab Zeile 2 („2“, a, „one“).
        let other = [
            (
                "x", "1
2",
            ),
            ("a", "one"),
        ];
        assert_eq!(list.render(&other, 20, 3).scroll, 2);
        let now = Instant::now();
        list.click(2, &other, now);
        let open = list.click(2, &other, now);
        assert_eq!(open.map(|o| o.claude_id), Some("a".into()));
        list.wheel(true, &other);
        assert_eq!(list.render(&other, 20, 3).scroll, 0);
    }
}
