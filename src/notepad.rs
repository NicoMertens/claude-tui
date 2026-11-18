//! Notizblock (Ctrl+A p): ein Text unabhängig von den Sessions, der wie eine Session in einem
//! Fenster liegt. Lange Zeilen werden an Wortgrenzen umbrochen (kein waagerechtes Scrollen),
//! senkrecht scrollt die Ansicht mit dem Cursor bzw. dem Mausrad. Der Text liegt als
//! `notepad.txt` im Konfigurationsordner und wird spätestens eine Sekunde nach einer Änderung
//! gespeichert. Ändert eine andere claude-tui-Instanz die Datei, wird sie neu gelesen, solange hier
//! nichts ungespeichert ist; sonst gewinnt, wer zuletzt speichert.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

use crate::{
    paths,
    textbuf::{self, TextBuf},
};

const FILE_NAME: &str = "notepad.txt";
/// So lange nach der ersten ungespeicherten Änderung wird gespeichert (Tippen schreibt nicht bei
/// jeder Taste); ebenso oft wird geprüft, ob eine andere Instanz die Datei geändert hat.
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// Zeilen je Mausrad-Raste.
const WHEEL_ROWS: usize = 3;

/// Eine Bildschirmzeile: Zeichenbereich `start..end` im Text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    /// Die Textzeile geht in der nächsten Bildschirmzeile weiter (Umbruch, kein `\n`).
    pub soft: bool,
}

#[derive(Debug, Default)]
pub struct Notepad {
    buf: TextBuf,
    /// Erste sichtbare Bildschirmzeile.
    scroll: usize,
    /// Spalte, die ↑/↓ beibehalten; jede andere Bewegung vergisst sie.
    goal: Option<usize>,
    /// Breite und Höhe der Textfläche, zuletzt gesetzt über [`Self::set_view`].
    view: (usize, usize),
    /// Zeitpunkt der ersten noch nicht gespeicherten Änderung.
    unsaved_since: Option<Instant>,
    /// Änderungszeit der Datei beim letzten Lesen bzw. Schreiben.
    disk: Option<SystemTime>,
    last_check: Option<Instant>,
    /// Die Datei ließ sich nicht lesen (sie fehlte nicht nur): nie speichern, sonst wäre ihr Text
    /// verloren.
    blocked: bool,
}

impl Notepad {
    /// Liest den gespeicherten Text; fehlt die Datei, ist der Notizblock leer. Lässt sie sich nicht
    /// lesen, bleibt er leer und speichert nie — mit Hinweis für den Benutzer.
    pub fn load() -> (Self, Option<String>) {
        let Some(file) = file() else {
            return (Self::default(), None);
        };
        match read(&file) {
            Ok(Some((text, modified))) => (
                Self {
                    disk: modified,
                    ..Self::with_text(&text)
                },
                None,
            ),
            Ok(None) => (Self::default(), None),
            Err(e) => (
                Self {
                    blocked: true,
                    ..Self::default()
                },
                Some(format!("notepad not loaded, changes are not saved: {e:#}")),
            ),
        }
    }

    /// Je Durchlauf der Hauptschleife: Ungespeichertes nach [`SAVE_DELAY`] speichern, sonst
    /// höchstens so oft nachsehen, ob eine andere Instanz die Datei geändert hat.
    pub fn sync(&mut self) -> Result<()> {
        if self.unsaved_since.is_some() {
            return self.save_if_due(false);
        }
        if self.blocked || self.last_check.is_some_and(|at| at.elapsed() < SAVE_DELAY) {
            return Ok(());
        }
        self.last_check = Some(Instant::now());
        let Some(file) = file() else {
            return Ok(());
        };
        let modified = modified(&file);
        if modified.is_none() || modified == self.disk {
            return Ok(());
        }
        self.disk = modified;
        match read(&file) {
            Ok(Some((text, modified))) => {
                let cursor = self.buf.cursor();
                self.buf = TextBuf::new(&text);
                self.buf.set_cursor(cursor);
                self.disk = modified;
                self.goal = None;
                self.follow_cursor();
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => {
                self.blocked = true;
                Err(e.context("notepad changed elsewhere, changes are no longer saved"))
            }
        }
    }

    fn with_text(text: &str) -> Self {
        let mut buf = TextBuf::new(text);
        buf.set_cursor(0);
        Self {
            buf,
            ..Self::default()
        }
    }

    /// Speichert, wenn sich seit [`SAVE_DELAY`] nichts mehr geändert hat (`force`: sofort).
    pub fn save_if_due(&mut self, force: bool) -> Result<()> {
        let Some(since) = self.unsaved_since else {
            return Ok(());
        };
        if self.blocked {
            self.unsaved_since = None;
            return Ok(());
        }
        if !force && since.elapsed() < SAVE_DELAY {
            return Ok(());
        }
        let file = file().context("no config directory")?;
        let saved = paths::write_atomic(&file, self.buf.text()).context("saving notepad");
        // Fehlgeschlagen: nach SAVE_DELAY erneut versuchen, nicht in jedem Durchlauf.
        self.unsaved_since = saved.is_err().then(Instant::now);
        if saved.is_ok() {
            self.disk = modified(&file);
        }
        saved
    }

    /// Anzahl der Textzeilen (ohne Umbruch gezählt); leerer Text hat keine.
    pub fn line_count(&self) -> usize {
        self.buf.text().lines().count()
    }

    /// Größe der Textfläche; ändert sie sich, liegt der Cursor danach wieder im sichtbaren
    /// Bereich (sonst bleibt eine per Mausrad verschobene Ansicht stehen).
    pub fn set_view(&mut self, width: usize, height: usize) {
        if self.view != (width, height) {
            self.view = (width, height);
            self.follow_cursor();
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let before = self.buf.text().len();
        let rows = self.rows();
        let row = row_of(&rows, self.buf.cursor());
        let height = self.view.1.max(1);
        let mut keep_goal = false;
        let mut changed = false;
        match key.code {
            KeyCode::Enter => {
                self.buf.insert("\n");
                changed = true;
            }
            KeyCode::Tab => {
                self.buf.insert("    ");
                changed = true;
            }
            KeyCode::Up => {
                self.move_to_row(&rows, row.checked_sub(1), 0);
                keep_goal = true;
            }
            KeyCode::Down => {
                let end = self.buf.len();
                self.move_to_row(&rows, Some(row + 1).filter(|&r| r < rows.len()), end);
                keep_goal = true;
            }
            KeyCode::PageUp => {
                self.move_to_row(&rows, Some(row.saturating_sub(height)), 0);
                keep_goal = true;
            }
            KeyCode::PageDown => {
                let last = rows.len() - 1;
                self.move_to_row(&rows, Some((row + height).min(last)), 0);
                keep_goal = true;
            }
            KeyCode::Home if ctrl => self.buf.set_cursor(0),
            KeyCode::End if ctrl => self.buf.set_cursor(self.buf.len()),
            KeyCode::Home => self.buf.set_cursor(rows[row].start),
            KeyCode::End => self.buf.set_cursor(last_index(rows[row])),
            _ => changed = self.buf.edit(key) && self.buf.text().len() != before,
        }
        if !keep_goal {
            self.goal = None;
        }
        if changed {
            self.touch();
        }
        self.follow_cursor();
    }

    pub fn paste(&mut self, text: &str) {
        self.buf.paste(text);
        self.goal = None;
        self.touch();
        self.follow_cursor();
    }

    /// Mausrad: verschiebt nur die Ansicht, der Cursor bleibt.
    pub fn wheel(&mut self, up: bool) {
        self.scroll = if up {
            self.scroll.saturating_sub(WHEEL_ROWS)
        } else {
            (self.scroll + WHEEL_ROWS).min(self.max_scroll(&self.rows()))
        };
    }

    /// Klick auf Zeile `row`, Spalte `col` der Textfläche setzt den Cursor dorthin.
    pub fn click(&mut self, row: usize, col: usize) {
        let rows = self.rows();
        let target = (self.scroll + row).min(rows.len() - 1);
        self.buf
            .set_cursor(index_at(self.buf.text(), rows[target], col));
        self.goal = None;
    }

    /// Sichtbare Zeilen für eine Fläche von `width` × `height` und der Cursor darin als (Zeile,
    /// Spalte), dazu die Gesamtzahl der Zeilen und die erste sichtbare.
    pub fn render(&self, width: usize, height: usize) -> Rendered {
        let rows = wrap(self.buf.text(), width);
        let scroll = self.scroll.min(rows.len().saturating_sub(height));
        let chars: Vec<char> = self.buf.text().chars().collect();
        let lines = rows
            .iter()
            .skip(scroll)
            .take(height)
            .map(|r| chars[r.start..r.end].iter().collect())
            .collect();
        let row = row_of(&rows, self.buf.cursor());
        let cursor = (scroll..scroll + height).contains(&row).then(|| {
            let col = display_width(&chars[rows[row].start..self.buf.cursor()]);
            (row - scroll, col)
        });
        Rendered {
            lines,
            cursor,
            total: rows.len(),
            scroll,
        }
    }

    fn rows(&self) -> Vec<Row> {
        wrap(self.buf.text(), self.view.0)
    }

    fn max_scroll(&self, rows: &[Row]) -> usize {
        rows.len().saturating_sub(self.view.1.max(1))
    }

    fn touch(&mut self) {
        self.unsaved_since.get_or_insert_with(Instant::now);
    }

    /// Cursor in Bildschirmzeile `target`, möglichst in der gemerkten Spalte; ohne Ziel (über die
    /// erste bzw. letzte Zeile hinaus) an `fallback`.
    fn move_to_row(&mut self, rows: &[Row], target: Option<usize>, fallback: usize) {
        let text = self.buf.text();
        let start = rows[row_of(rows, self.buf.cursor())].start;
        let chars: Vec<char> = text.chars().collect();
        let col = *self
            .goal
            .get_or_insert_with(|| display_width(&chars[start..self.buf.cursor()]));
        let cursor = target.map_or(fallback, |target| index_at(text, rows[target], col));
        self.buf.set_cursor(cursor);
    }

    fn follow_cursor(&mut self) {
        let rows = self.rows();
        let row = row_of(&rows, self.buf.cursor());
        let height = self.view.1.max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll(&rows));
    }
}

fn file() -> Option<PathBuf> {
    paths::config_dir().map(|d| d.join(FILE_NAME))
}

fn modified(file: &Path) -> Option<SystemTime> {
    fs::metadata(file).and_then(|m| m.modified()).ok()
}

/// Text und Änderungszeit der Datei; `None`, wenn es sie nicht gibt.
fn read(file: &Path) -> Result<Option<(String, Option<SystemTime>)>> {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not UTF-8", file.display()))?;
    Ok(Some((clean(&text), modified(file))))
}

/// Mit einem anderen Editor bearbeitet: BOM und `\r` lägen sonst als unsichtbare Zeichen unter dem
/// Cursor.
fn clean(text: &str) -> String {
    textbuf::sanitize(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// Ergebnis von [`Notepad::render`].
pub struct Rendered {
    pub lines: Vec<String>,
    pub cursor: Option<(usize, usize)>,
    pub total: usize,
    pub scroll: usize,
}

/// Bricht `text` für die Breite `width` in Bildschirmzeilen um: an der letzten Leerstelle, die
/// noch passt, sonst mitten im Wort. Jede Zeile hat mindestens ein Zeichen, auch wenn es breiter
/// ist als `width`.
pub fn wrap(text: &str, width: usize) -> Vec<Row> {
    let width = width.max(1);
    let chars: Vec<char> = text.chars().collect();
    let mut rows = Vec::new();
    let mut line_start = 0;
    loop {
        let line_end = chars[line_start..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(chars.len(), |p| line_start + p);
        let mut start = line_start;
        loop {
            let (mut used, mut i, mut space_break) = (0, start, None);
            while i < line_end {
                let w = char_width(chars[i]);
                if used + w > width && i > start {
                    break;
                }
                used += w;
                i += 1;
                if chars[i - 1] == ' ' {
                    space_break = Some(i);
                }
            }
            if i >= line_end {
                rows.push(Row {
                    start,
                    end: line_end,
                    soft: false,
                });
                break;
            }
            // Ein Leerzeichen, das nicht mehr passt, bleibt am Zeilenende statt die nächste
            // Zeile einzurücken.
            let end = if chars[i] == ' ' {
                i + 1
            } else {
                space_break.unwrap_or(i)
            };
            rows.push(Row {
                start,
                end,
                soft: true,
            });
            start = end;
        }
        if line_end >= chars.len() {
            return rows;
        }
        line_start = line_end + 1;
    }
}

/// Bildschirmzeile des Cursors. Am Umbruch steht er am Anfang der nächsten Zeile.
fn row_of(rows: &[Row], cursor: usize) -> usize {
    rows.partition_point(|r| r.start <= cursor)
        .saturating_sub(1)
}

/// Letzte Cursorposition in `row`: bei einem Umbruch vor dessen letztem Zeichen, sonst am
/// Zeilenende — sonst stünde der Cursor schon in der nächsten Zeile.
fn last_index(row: Row) -> usize {
    if row.soft {
        row.end - 1
    } else {
        row.end
    }
}

/// Zeichenindex in `row`, der der Anzeigespalte `col` am nächsten kommt (nicht dahinter).
fn index_at(text: &str, row: Row, col: usize) -> usize {
    let mut used = 0;
    let mut index = row.start;
    for c in text
        .chars()
        .skip(row.start)
        .take(last_index(row) - row.start)
    {
        let w = char_width(c);
        if used + w > col {
            break;
        }
        used += w;
        index += 1;
    }
    index
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

fn display_width(chars: &[char]) -> usize {
    chars.iter().map(|&c| char_width(c)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn texts(text: &str, width: usize) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        wrap(text, width)
            .iter()
            .map(|r| chars[r.start..r.end].iter().collect())
            .collect()
    }

    fn pad(text: &str, width: usize, height: usize) -> Notepad {
        let mut p = Notepad::with_text(text);
        p.set_view(width, height);
        p
    }

    #[test]
    fn wraps_at_word_boundaries_and_breaks_long_words() {
        assert_eq!(texts("eins zwei drei", 9), ["eins zwei ", "drei"]);
        assert_eq!(texts("eins zwei drei", 8), ["eins ", "zwei ", "drei"]);
        assert_eq!(texts("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(texts("abcde fg", 5), ["abcde ", "fg"]);
        assert_eq!(texts("a\n\nb", 5), ["a", "", "b"]);
        assert_eq!(texts("", 5), [""]);
        assert_eq!(texts("ab\n", 5), ["ab", ""]);
    }

    #[test]
    fn wide_characters_count_double() {
        assert_eq!(texts("日本語", 4), ["日本", "語"]);
        assert_eq!(
            texts("日", 1),
            ["日"],
            "breiter als die Zeile: trotzdem ein Zeichen"
        );
    }

    #[test]
    fn cursor_at_a_soft_break_starts_the_next_row() {
        let p = pad("eins zwei", 5, 5);
        // Cursor ans Ende von "eins " = Anfang von "zwei".
        let mut p = p;
        p.buf.set_cursor(5);
        assert_eq!(p.render(5, 5).cursor, Some((1, 0)));
        p.on_key(&key(KeyCode::Up));
        assert_eq!(p.buf.cursor(), 0);
        p.on_key(&key(KeyCode::End));
        assert_eq!(p.buf.cursor(), 4, "vor dem Leerzeichen am Umbruch");
        assert_eq!(p.render(5, 5).cursor, Some((0, 4)));
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let mut p = pad("abcdef\nx\nabcdef", 20, 5);
        p.buf.set_cursor(4);
        p.on_key(&key(KeyCode::Down));
        assert_eq!(p.render(20, 5).cursor, Some((1, 1)));
        p.on_key(&key(KeyCode::Down));
        assert_eq!(p.render(20, 5).cursor, Some((2, 4)), "Spalte gemerkt");
        p.on_key(&key(KeyCode::Down));
        assert_eq!(
            p.buf.cursor(),
            p.buf.len(),
            "über die letzte Zeile: ans Ende"
        );
        p.on_key(&key(KeyCode::Home));
        p.on_key(&key(KeyCode::Up));
        p.on_key(&key(KeyCode::Up));
        p.on_key(&key(KeyCode::Up));
        assert_eq!(p.buf.cursor(), 0, "über die erste Zeile: an den Anfang");
    }

    #[test]
    fn enter_breaks_the_line_and_view_follows_the_cursor() {
        let mut p = pad("", 10, 2);
        for _ in 0..4 {
            p.on_key(&key(KeyCode::Enter));
        }
        let shown = p.render(10, 2);
        assert_eq!((shown.total, shown.scroll), (5, 3));
        assert_eq!(shown.cursor, Some((1, 0)));
        p.on_key(&KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
        assert_eq!(p.render(10, 2).scroll, 0);
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let mut p = pad(&"x\n".repeat(20), 10, 5);
        p.wheel(false);
        let shown = p.render(10, 5);
        assert_eq!(shown.scroll, 3);
        assert_eq!(shown.cursor, None, "Cursor oben, nicht mehr sichtbar");
        for _ in 0..10 {
            p.wheel(false);
        }
        assert_eq!(p.render(10, 5).scroll, 16, "nicht über das Ende hinaus");
    }

    #[test]
    fn click_places_the_cursor() {
        let mut p = pad("eins zwei\ndrei", 5, 5);
        p.click(1, 2);
        assert_eq!(p.buf.cursor(), 7);
        p.click(9, 9);
        assert_eq!(p.buf.cursor(), p.buf.len(), "unter dem Text: letzte Zeile");
    }

    #[test]
    fn text_from_other_editors_is_cleaned() {
        assert_eq!(clean("\u{feff}a\r\n\tb"), "a\n    b");
    }

    #[test]
    fn unreadable_file_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("claude-tui-notepad-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("Ordner");
        let file = dir.join(FILE_NAME);
        fs::write(&file, [0xff, 0xfe, b'a']).expect("schreiben");
        assert!(read(&file).is_err(), "kein UTF-8");
        assert_eq!(read(&dir.join("fehlt.txt")).expect("fehlt"), None);
        let mut p = Notepad {
            blocked: true,
            ..Notepad::default()
        };
        p.paste("x");
        p.save_if_due(true).expect("nichts zu tun");
        assert!(p.unsaved_since.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_count_ignores_wrapping() {
        assert_eq!(pad("", 3, 5).line_count(), 0);
        assert_eq!(pad("eins zwei drei\nvier\n", 3, 5).line_count(), 2);
    }

    #[test]
    fn only_text_changes_need_saving() {
        let mut p = pad("ab", 10, 5);
        p.on_key(&key(KeyCode::Left));
        p.on_key(&key(KeyCode::End));
        assert!(p.unsaved_since.is_none());
        p.on_key(&key(KeyCode::Backspace));
        assert!(p.unsaved_since.is_some());
    }
}
