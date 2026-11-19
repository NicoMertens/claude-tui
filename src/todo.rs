//! Todo-Liste (Ctrl+A d): Einträge unabhängig von den Sessions, die wie der Notizblock in einem
//! Fenster liegen. Abhaken lässt einen Eintrag an seiner Stelle; die Reihenfolge ändert nur das
//! Verschieben (Shift+↑/↓). Einträge können mehrzeilig sein; bearbeitet wird direkt in der Liste. Gespeichert wird als
//! `todo.json` im Konfigurationsordner, geteilt mit anderen claude-tui-Instanzen wie beim
//! Notizblock (siehe [`crate::filesync`]).

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

use crate::{
    filesync::SyncedFile,
    noteedit, notepad,
    textbuf::{self, TextBuf},
};

const FILE_NAME: &str = "todo.json";
/// Zeilen je Mausrad-Raste.
const WHEEL_ROWS: usize = 3;
/// Breite von Häkchen und Abstand vor dem Text; Folgezeilen sind ebenso weit eingerückt.
pub const MARK_WIDTH: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug)]
struct Edit {
    index: usize,
    buf: TextBuf,
    /// Eben angelegt: Abbrechen oder leerer Text entfernt den Eintrag wieder.
    new: bool,
}

#[derive(Debug)]
pub struct Todo {
    /// In der vom Benutzer bestimmten Reihenfolge.
    items: Vec<Item>,
    /// Index des gewählten Eintrags (0 bei leerer Liste).
    selected: usize,
    /// Erste sichtbare Bildschirmzeile.
    scroll: usize,
    /// Breite und Höhe der Fläche, zuletzt gesetzt über [`Self::set_view`].
    view: (usize, usize),
    edit: Option<Edit>,
    file: SyncedFile,
}

impl Default for Todo {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            view: (0, 0),
            edit: None,
            file: SyncedFile::new(FILE_NAME, "todo list"),
        }
    }
}

/// Ergebnis von [`Todo::render`]: die sichtbaren Bildschirmzeilen.
pub struct Rendered {
    pub lines: Vec<Line>,
    /// Cursor des Eingabefelds als (Zeile, Spalte) in der Textspalte, falls sichtbar.
    pub cursor: Option<(usize, usize)>,
    pub total: usize,
    pub scroll: usize,
}

pub struct Line {
    /// Erste Zeile eines Eintrags: dessen Zustand (Häkchen davor); Folgezeilen `None`.
    pub mark: Option<bool>,
    pub text: String,
    pub done: bool,
    pub selected: bool,
    pub editing: bool,
}

/// Ein sichtbarer Eintrag: Index und Bildschirmzeilen seines Texts.
struct Laid {
    index: usize,
    first_row: usize,
    rows: Vec<notepad::Row>,
}

impl Todo {
    /// Liest die gespeicherte Liste; fehlt die Datei, ist sie leer. Lässt sie sich nicht lesen oder
    /// verstehen, bleibt sie leer und speichert nie — mit Hinweis für den Benutzer.
    pub fn load() -> (Self, Option<String>) {
        let mut todo = Self::default();
        let text = match todo.file.load() {
            Ok(text) => text,
            Err(e) => return (todo, Some(format!("{e:#}"))),
        };
        let Some(text) = text else {
            return (todo, None);
        };
        match parse(&text) {
            Ok(items) => {
                todo.items = items;
                (todo, None)
            }
            Err(e) => {
                todo.file.block();
                (
                    todo,
                    Some(format!("{e:#}, todo list changes are not saved")),
                )
            }
        }
    }

    /// Je Durchlauf der Hauptschleife: Ungespeichertes nach der Wartezeit speichern, sonst
    /// übernehmen, was eine andere Instanz gespeichert hat — nicht, solange ein Eintrag bearbeitet
    /// wird.
    pub fn sync(&mut self) -> Result<()> {
        if self.file.is_unsaved() {
            return self.save_if_due(false);
        }
        if self.edit.is_some() {
            return Ok(());
        }
        if let Some(text) = self.file.poll()? {
            match parse(&text) {
                Ok(items) => {
                    self.items = items;
                    self.clamp_selection();
                    self.follow_selection();
                }
                Err(e) => {
                    self.file.block();
                    return Err(
                        e.context("todo list changed elsewhere, changes are no longer saved")
                    );
                }
            }
        }
        Ok(())
    }

    /// Speichert, wenn sich seit der Wartezeit nichts mehr geändert hat (`force`: sofort). Ein eben
    /// angelegter, noch leerer Eintrag wird nicht mitgespeichert.
    pub fn save_if_due(&mut self, force: bool) -> Result<()> {
        let skip = self.edit.as_ref().filter(|e| e.new).map(|e| e.index);
        let items = &self.items;
        self.file.save_if_due(force, || {
            let kept: Vec<&Item> = items
                .iter()
                .enumerate()
                .filter(|&(i, _)| Some(i) != skip)
                .map(|(_, item)| item)
                .collect();
            serde_json::to_string_pretty(&kept).unwrap_or_default()
        })
    }

    /// Beim Beenden: ein offenes Eingabefeld übernehmen und sofort speichern.
    pub fn save_now(&mut self) -> Result<()> {
        if self.edit.is_some() {
            self.finish_edit(true);
        }
        self.save_if_due(true)
    }

    /// Anzahl der offenen Einträge.
    pub fn open_count(&self) -> usize {
        self.items.iter().filter(|i| !i.done).count()
    }

    pub fn done_count(&self) -> usize {
        self.items.len() - self.open_count()
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    /// Größe der Fläche; ändert sie sich, ist die Auswahl danach wieder sichtbar.
    pub fn set_view(&mut self, width: usize, height: usize) {
        if self.view != (width, height) {
            self.view = (width, height);
            self.follow_selection();
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) {
        if self.edit.is_some() {
            self.on_edit_key(key);
        } else {
            self.on_list_key(key);
        }
        self.follow_selection();
    }

    fn on_list_key(&mut self, key: &KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // Kein Buchstabenbefehl mit Ctrl oder Alt (AltGr-Zeichen u. a.).
        let plain = !ctrl && !alt;
        let count = self.items.len();
        let last = count.saturating_sub(1);
        let page = self.page();
        match key.code {
            // Alt+↑/↓ für Terminals, die Shift+Pfeil nicht unterscheiden.
            KeyCode::Up if shift || alt => self.move_selected(false),
            KeyCode::Down if shift || alt => self.move_selected(true),
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(page),
            KeyCode::PageDown => self.selected = (self.selected + page).min(last),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = last,
            KeyCode::Char(' ') if plain => self.toggle(),
            KeyCode::Enter if count == 0 => self.add(),
            KeyCode::Enter | KeyCode::F(2) => self.start_edit(),
            KeyCode::Char('e') if plain => self.start_edit(),
            KeyCode::Char('a') if plain => self.add(),
            // Bewusst mit Shift: Löschen lässt sich nicht rückgängig machen.
            KeyCode::Char('X') if plain => self.delete(),
            _ => {}
        }
    }

    /// Wie im Notiz-Editor: Enter übernimmt, Shift+Enter bzw. Ctrl+J bricht die Zeile um, Esc
    /// bricht ab.
    fn on_edit_key(&mut self, key: &KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => return self.finish_edit(false),
            KeyCode::Enter if !shift && !alt => return self.finish_edit(true),
            _ => {}
        }
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        let buf = &mut edit.buf;
        match key.code {
            KeyCode::Enter => buf.insert("\n"),
            KeyCode::Char('j') if ctrl && !alt => buf.insert("\n"),
            KeyCode::Home => buf.set_cursor(buf.line_start()),
            KeyCode::End => buf.set_cursor(buf.line_end()),
            KeyCode::Up => buf.move_vertically(false),
            KeyCode::Down => buf.move_vertically(true),
            _ => {
                buf.edit(key);
            }
        }
    }

    /// Eingefügter Text geht nur ins Eingabefeld; in der Liste würde er sonst als Befehle wirken.
    pub fn paste(&mut self, text: &str) {
        if let Some(edit) = self.edit.as_mut() {
            edit.buf.paste(text);
            self.follow_selection();
        }
    }

    /// Mausrad: verschiebt nur die Ansicht, die Auswahl bleibt.
    pub fn wheel(&mut self, up: bool) {
        let total = self.layout(self.text_width()).1;
        let max = total.saturating_sub(self.view.1.max(1));
        self.scroll = if up {
            self.scroll.saturating_sub(WHEEL_ROWS)
        } else {
            (self.scroll + WHEEL_ROWS).min(max)
        };
    }

    /// Klick auf Zeile `row`, Spalte `col` der Fläche: wählt den Eintrag dort; ein Klick auf das
    /// Häkchen schaltet ihn um, einer in das Eingabefeld setzt dessen Cursor. Ein anderes offenes
    /// Eingabefeld wird dabei übernommen.
    pub fn click(&mut self, row: usize, col: usize) {
        let (laid, _) = self.layout(self.text_width());
        let row = self.scroll + row;
        let Some(item) = laid
            .iter()
            .find(|l| (l.first_row..l.first_row + l.rows.len()).contains(&row))
        else {
            return;
        };
        let index = item.index;
        if let Some(edit) = self.edit.as_mut().filter(|e| e.index == index) {
            if let Some(&r) = item.rows.get(row - item.first_row) {
                let text = edit.buf.text().to_string();
                let at = index_at(&text, r, col.saturating_sub(MARK_WIDTH));
                edit.buf.set_cursor(at);
            }
            return;
        }
        let on_mark = col < MARK_WIDTH && row == item.first_row;
        if self.edit.is_some() {
            // Übernehmen kann Einträge entfernen (leer = gelöscht); dann nur auswählen.
            self.finish_edit(true);
            self.selected = index.min(self.items.len().saturating_sub(1));
        } else {
            self.selected = index;
            if on_mark {
                self.toggle();
            }
        }
        self.follow_selection();
    }

    /// Sichtbare Zeilen für eine Fläche von `width` × `height` (samt Häkchen-Spalte).
    pub fn render(&self, width: usize, height: usize) -> Rendered {
        let text_width = width.saturating_sub(MARK_WIDTH).max(1);
        let (laid, total) = self.layout(text_width);
        let scroll = self.scroll.min(total.saturating_sub(height));
        let mut lines = Vec::new();
        let mut cursor = None;
        for item in &laid {
            let editing = self.edit.as_ref().filter(|e| e.index == item.index);
            let text = self.text_of(item.index);
            let chars: Vec<char> = text.chars().collect();
            let done = self.items[item.index].done;
            for (n, r) in item.rows.iter().enumerate() {
                let row = item.first_row + n;
                if row < scroll || row >= scroll + height {
                    continue;
                }
                lines.push(Line {
                    mark: (n == 0).then_some(done),
                    text: chars[r.start..r.end].iter().collect(),
                    done,
                    selected: item.index == self.selected,
                    editing: editing.is_some(),
                });
            }
            if let Some(edit) = editing {
                let n = row_of(&item.rows, edit.buf.cursor());
                let row = item.first_row + n;
                if (scroll..scroll + height).contains(&row) {
                    let start = item.rows[n].start;
                    let col = display_width(&chars[start..edit.buf.cursor()]);
                    cursor = Some((row - scroll, col));
                }
            }
        }
        Rendered {
            lines,
            cursor,
            total,
            scroll,
        }
    }

    fn text_width(&self) -> usize {
        self.view.0.saturating_sub(MARK_WIDTH).max(1)
    }

    fn page(&self) -> usize {
        // Grob: eine Bildschirmzeile je Eintrag.
        self.view.1.saturating_sub(1).max(1)
    }

    /// Text eines Eintrags, beim bearbeiteten der des Eingabefelds.
    fn text_of(&self, index: usize) -> &str {
        match self.edit.as_ref().filter(|e| e.index == index) {
            Some(edit) => edit.buf.text(),
            None => &self.items[index].text,
        }
    }

    /// Einträge mit ihren Bildschirmzeilen und die Gesamtzahl der Zeilen — gemeinsame
    /// Grundlage für Zeichnen, Scrollen und Klicks.
    fn layout(&self, text_width: usize) -> (Vec<Laid>, usize) {
        let mut laid = Vec::new();
        let mut row = 0;
        for index in 0..self.items.len() {
            let rows = notepad::wrap(self.text_of(index), text_width);
            let height = rows.len();
            laid.push(Laid {
                index,
                first_row: row,
                rows,
            });
            row += height;
        }
        (laid, row)
    }

    /// Scrollt so, dass der gewählte Eintrag (beim Bearbeiten die Cursorzeile) sichtbar ist.
    fn follow_selection(&mut self) {
        let height = self.view.1.max(1);
        let (laid, total) = self.layout(self.text_width());
        if let Some(item) = laid.get(self.selected) {
            let (top, bottom) = match self.edit.as_ref().filter(|e| e.index == item.index) {
                Some(edit) => {
                    let row = item.first_row + row_of(&item.rows, edit.buf.cursor());
                    (row, row + 1)
                }
                None => (item.first_row, item.first_row + item.rows.len()),
            };
            if bottom > self.scroll + height {
                self.scroll = bottom - height;
            }
            if top < self.scroll {
                self.scroll = top;
            }
        }
        self.scroll = self.scroll.min(total.saturating_sub(height));
    }

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
    }

    /// Neuer offener Eintrag unter der Auswahl, gleich im Eingabefeld.
    fn add(&mut self) {
        let index = if self.items.is_empty() {
            0
        } else {
            self.selected + 1
        };
        self.items.insert(
            index,
            Item {
                text: String::new(),
                done: false,
            },
        );
        self.selected = index;
        self.edit = Some(Edit {
            index,
            buf: TextBuf::default(),
            new: true,
        });
    }

    fn start_edit(&mut self) {
        if let Some(item) = self.items.get(self.selected) {
            self.edit = Some(Edit {
                index: self.selected,
                buf: TextBuf::new(&item.text),
                new: false,
            });
        }
    }

    /// Schließt das Eingabefeld: `keep` übernimmt den Text. Leer wird ein neuer Eintrag verworfen,
    /// ein bestehender gelöscht.
    fn finish_edit(&mut self, keep: bool) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        let text = keep.then(|| noteedit::normalize(edit.buf.text())).flatten();
        match (text, edit.new) {
            (Some(text), _) => {
                if self.items[edit.index].text != text {
                    self.items[edit.index].text = text;
                    self.file.touch();
                }
                if edit.new {
                    self.file.touch();
                }
            }
            (None, true) => {
                self.items.remove(edit.index);
                self.clamp_selection();
            }
            (None, false) if keep => {
                self.selected = edit.index;
                self.delete();
            }
            (None, false) => {}
        }
    }

    /// Leertaste: offen ↔ erledigt; der Eintrag bleibt an seiner Stelle.
    fn toggle(&mut self) {
        if let Some(item) = self.items.get_mut(self.selected) {
            item.done = !item.done;
            self.file.touch();
        }
    }

    /// Shift+↑/↓: gewählten Eintrag um eine Stelle verschieben; die Auswahl geht mit.
    fn move_selected(&mut self, down: bool) {
        let i = self.selected;
        let Some(j) = (if down {
            i.checked_add(1)
        } else {
            i.checked_sub(1)
        }) else {
            return;
        };
        if j >= self.items.len() {
            return;
        }
        self.items.swap(i, j);
        self.selected = j;
        self.file.touch();
    }

    fn delete(&mut self) {
        if self.selected >= self.items.len() {
            return;
        }
        self.items.remove(self.selected);
        self.clamp_selection();
        self.file.touch();
    }
}

/// Liest die gespeicherte Liste; leere Einträge fallen weg.
fn parse(text: &str) -> Result<Vec<Item>> {
    let mut items: Vec<Item> = serde_json::from_str(text).context("todo.json is not readable")?;
    for item in &mut items {
        item.text = textbuf::sanitize(&item.text);
    }
    items.retain(|i| !i.text.trim().is_empty());
    Ok(items)
}

/// Bildschirmzeile des Cursors. Am Umbruch steht er am Anfang der nächsten Zeile.
fn row_of(rows: &[notepad::Row], cursor: usize) -> usize {
    rows.partition_point(|r| r.start <= cursor)
        .saturating_sub(1)
}

/// Zeichenindex in `row`, der der Anzeigespalte `col` am nächsten kommt (nicht dahinter).
fn index_at(text: &str, row: notepad::Row, col: usize) -> usize {
    let last = if row.soft { row.end - 1 } else { row.end };
    let mut used = 0;
    let mut index = row.start;
    for c in text.chars().skip(row.start).take(last - row.start) {
        let w = c.width().unwrap_or(0);
        if used + w > col {
            break;
        }
        used += w;
        index += 1;
    }
    index
}

fn display_width(chars: &[char]) -> usize {
    chars.iter().map(|c| c.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn type_text(t: &mut Todo, text: &str) {
        for c in text.chars() {
            t.on_key(&key(KeyCode::Char(c)));
        }
    }

    fn todo(items: &[(&str, bool)]) -> Todo {
        let mut t = Todo {
            items: items
                .iter()
                .map(|&(text, done)| Item {
                    text: text.into(),
                    done,
                })
                .collect(),
            ..Todo::default()
        };
        t.set_view(20, 10);
        t
    }

    fn texts(t: &Todo) -> Vec<(&str, bool)> {
        t.items.iter().map(|i| (i.text.as_str(), i.done)).collect()
    }

    #[test]
    fn add_types_a_multiline_item_and_saves_on_enter() {
        let mut t = todo(&[]);
        t.on_key(&key(KeyCode::Char('a')));
        assert!(t.is_editing());
        type_text(&mut t, "eins");
        t.on_key(&shift(KeyCode::Enter));
        type_text(&mut t, "zwei");
        t.on_key(&key(KeyCode::Enter));
        assert!(!t.is_editing());
        assert_eq!(texts(&t), [("eins\nzwei", false)]);
        assert_eq!(t.open_count(), 1);
        assert!(t.file.is_unsaved());
    }

    #[test]
    fn new_item_goes_below_the_selection() {
        let mut t = todo(&[("a", false), ("b", true), ("c", false)]);
        t.on_key(&key(KeyCode::Down));
        t.on_key(&key(KeyCode::Char('a')));
        type_text(&mut t, "neu");
        t.on_key(&key(KeyCode::Enter));
        assert_eq!(
            texts(&t),
            [("a", false), ("b", true), ("neu", false), ("c", false)]
        );
        assert_eq!(t.selected, 2);
    }

    #[test]
    fn cancelling_or_emptying_a_new_item_discards_it() {
        let mut t = todo(&[("a", false)]);
        t.on_key(&key(KeyCode::Char('a')));
        type_text(&mut t, "x");
        t.on_key(&key(KeyCode::Esc));
        t.on_key(&key(KeyCode::Char('a')));
        t.on_key(&key(KeyCode::Enter));
        assert_eq!(texts(&t), [("a", false)]);
        assert!(!t.file.is_unsaved());
    }

    #[test]
    fn editing_changes_the_text_and_emptying_deletes() {
        let mut t = todo(&[("alt", false), ("b", false)]);
        t.on_key(&key(KeyCode::Char('e')));
        t.on_key(&key(KeyCode::Backspace));
        type_text(&mut t, "so");
        t.on_key(&key(KeyCode::Enter));
        assert_eq!(texts(&t)[0], ("also", false));
        t.on_key(&key(KeyCode::Enter));
        type_text(&mut t, "!");
        t.on_key(&key(KeyCode::Esc));
        assert_eq!(texts(&t)[0], ("also", false), "Esc verwirft");
        t.on_key(&key(KeyCode::Enter));
        for _ in 0..4 {
            t.on_key(&key(KeyCode::Backspace));
        }
        t.on_key(&key(KeyCode::Enter));
        assert_eq!(texts(&t), [("b", false)]);
    }

    #[test]
    fn toggling_keeps_item_and_selection_in_place() {
        let mut t = todo(&[("a", false), ("b", false), ("c", true)]);
        t.on_key(&key(KeyCode::Char(' ')));
        assert_eq!(texts(&t), [("a", true), ("b", false), ("c", true)]);
        assert_eq!(t.selected, 0);
        t.on_key(&key(KeyCode::End));
        t.on_key(&key(KeyCode::Char(' ')));
        assert_eq!(texts(&t), [("a", true), ("b", false), ("c", false)]);
        assert_eq!(t.selected, 2);
        assert_eq!(t.open_count(), 2);
    }

    #[test]
    fn shift_or_alt_arrows_move_by_one_position() {
        let mut t = todo(&[("a", false), ("b", true), ("c", false)]);
        t.on_key(&shift(KeyCode::Down));
        assert_eq!(texts(&t), [("b", true), ("a", false), ("c", false)]);
        assert_eq!(t.selected, 1);
        t.on_key(&KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
        assert_eq!(texts(&t), [("b", true), ("c", false), ("a", false)]);
        assert_eq!(t.selected, 2);
        t.on_key(&shift(KeyCode::Down));
        assert_eq!(t.selected, 2, "unten bleibt unten");
        t.on_key(&key(KeyCode::Home));
        t.on_key(&shift(KeyCode::Up));
        assert_eq!(texts(&t)[0], ("b", true), "oben bleibt oben");
    }

    #[test]
    fn shift_x_deletes_and_clamps_the_selection() {
        let mut t = todo(&[("a", false), ("b", false)]);
        t.on_key(&key(KeyCode::End));
        t.on_key(&key(KeyCode::Char('x')));
        t.on_key(&key(KeyCode::Delete));
        assert_eq!(texts(&t).len(), 2, "x und Entf löschen nicht");
        t.on_key(&shift(KeyCode::Char('X')));
        assert_eq!(texts(&t), [("a", false)]);
        assert_eq!(t.selected, 0);
        t.on_key(&shift(KeyCode::Char('X')));
        assert!(t.items.is_empty());
        t.on_key(&shift(KeyCode::Char('X')));
        t.on_key(&key(KeyCode::Char(' ')));
        t.on_key(&key(KeyCode::Char('e')));
        assert!(!t.is_editing(), "leere Liste: nichts zu bearbeiten");
    }

    #[test]
    fn removed_keys_do_nothing() {
        let mut t = todo(&[("a", true)]);
        t.on_key(&key(KeyCode::Char('h')));
        t.on_key(&key(KeyCode::Char('u')));
        t.on_key(&key(KeyCode::Char('x')));
        assert_eq!(texts(&t), [("a", true)]);
        assert_eq!(t.render(20, 10).lines.len(), 1);
    }

    #[test]
    fn letters_with_ctrl_or_alt_are_no_commands() {
        let mut t = todo(&[("a", false)]);
        t.on_key(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        t.on_key(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
        assert_eq!(texts(&t), [("a", false)]);
        assert!(!t.is_editing());
    }

    #[test]
    fn paste_only_reaches_the_input_field() {
        let mut t = todo(&[("a", false)]);
        t.paste("xX");
        assert_eq!(texts(&t), [("a", false)]);
        t.on_key(&key(KeyCode::Char('e')));
        t.paste("b\r\nc");
        t.on_key(&key(KeyCode::Enter));
        assert_eq!(texts(&t), [("ab\nc", false)]);
    }

    #[test]
    fn render_wraps_with_hanging_indent_and_marks_first_rows() {
        let t = todo(&[("eins zwei drei", false), ("x\ny", true)]);
        let shown = t.render(MARK_WIDTH + 10, 10);
        let rows: Vec<(Option<bool>, &str)> = shown
            .lines
            .iter()
            .map(|l| (l.mark, l.text.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (Some(false), "eins zwei "),
                (None, "drei"),
                (Some(true), "x"),
                (None, "y")
            ]
        );
        assert!(shown.lines[0].selected && shown.lines[1].selected);
        assert!(!shown.lines[2].selected);
        assert_eq!(shown.cursor, None);
    }

    #[test]
    fn view_follows_the_selection_and_the_edit_cursor() {
        let mut t = todo(&[("a", false), ("b", false), ("c", false), ("d", false)]);
        t.set_view(20, 2);
        t.on_key(&key(KeyCode::End));
        assert_eq!(t.render(20, 2).scroll, 2);
        t.on_key(&key(KeyCode::Char('e')));
        t.on_key(&shift(KeyCode::Enter));
        t.on_key(&shift(KeyCode::Enter));
        let shown = t.render(20, 2);
        assert_eq!(shown.scroll, 4);
        assert_eq!(shown.cursor, Some((1, 0)));
    }

    #[test]
    fn click_selects_and_the_mark_toggles_in_place() {
        let mut t = todo(&[("a", false), ("b", false)]);
        t.click(1, 5);
        assert_eq!(t.selected, 1);
        t.click(0, 0);
        assert_eq!(texts(&t), [("a", true), ("b", false)]);
        assert_eq!(t.selected, 0);
        t.click(9, 0);
        assert_eq!(texts(&t), [("a", true), ("b", false)], "unter der Liste");
    }

    #[test]
    fn click_elsewhere_commits_an_open_edit() {
        let mut t = todo(&[("a", false), ("b", false)]);
        t.on_key(&key(KeyCode::Char('e')));
        type_text(&mut t, "!");
        t.click(1, 3);
        assert!(!t.is_editing());
        assert_eq!(texts(&t)[0], ("a!", false));
        assert_eq!(t.selected, 1);
    }

    #[test]
    fn save_now_commits_the_open_edit() {
        let mut t = todo(&[]);
        t.on_key(&key(KeyCode::Char('a')));
        type_text(&mut t, "noch");
        t.finish_edit(true);
        assert_eq!(texts(&t), [("noch", false)]);
    }

    #[test]
    fn parse_keeps_the_order_and_drops_empty_items() {
        let items =
            parse(r#"[{"text":"b","done":true},{"text":"a"},{"text":"  "}]"#).expect("json");
        assert_eq!(
            items,
            [
                Item {
                    text: "b".into(),
                    done: true
                },
                Item {
                    text: "a".into(),
                    done: false
                }
            ]
        );
        assert!(parse("kaputt").is_err());
    }
}
