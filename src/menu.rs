//! Das Ctrl+A-Menü (Overlay über dem fokussierten Fenster) und die Fensterauswahl.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use crate::{
    keys,
    windows::{Focus, Pane, Spot, MAX_WINDOWS},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewSession,
    CloseWindow,
    /// Fragt nach, ob die Session endgültig gelöscht werden soll.
    DeleteSession,
    /// Bestätigung: Session(s) löschen (Ziel steht in [Purpose::Delete] bzw.
    /// [Purpose::DeleteMany]).
    ConfirmDelete,
    /// Auswahl, ab welchem Alter gespeicherte Sessions gelöscht werden.
    DeleteOlder,
    /// Gespeicherte Sessions löschen, deren letzte Aktivität so viele Tage zurückliegt.
    DeleteOlderThan(u64),
    /// Bestätigung: anderswo laufende Session trotzdem öffnen (Ziel in [Purpose::OpenElsewhere]).
    ConfirmOpen,
    Cancel,
    Focus(Focus),
    /// Sessionliste ein- bzw. ausblenden.
    ToggleList,
    /// Fenster mit Fokus auf die ganze Fensterfläche zoomen bzw. den Zoom aufheben.
    ToggleZoom,
    /// Gruppen-Dialog für die Session mit Fokus bzw. die markierten Sessions öffnen.
    SetGroup,
    /// Notiz-Editor für die Session mit Fokus bzw. die in der Liste gewählte öffnen.
    EditNote,
    /// Notizblock in einem Fenster zeigen (danach Fensterauswahl).
    ShowNotepad,
    /// Todo-Liste in einem Fenster zeigen (danach Fensterauswahl).
    ShowTodo,
    /// Notizübersicht in einem Fenster zeigen (danach Fensterauswahl).
    ShowNotes,
    /// Abschnitt der Auswahl in der Liste ein- bzw. ausklappen.
    ToggleCollapsed,
    /// Alle Abschnitte einklappen (`true`) bzw. ausklappen.
    CollapseAll(bool),
    /// Neuere Version von GitHub installieren.
    Update,
    /// Hilfeseite öffnen.
    Help,
    /// Beenden; fragt nach, solange Sessions laufen.
    Quit,
    /// Bestätigung: beenden, obwohl Sessions laufen.
    ConfirmQuit,
    /// Fensterauswahl: Ziel-Fensterindex.
    Place(usize),
    /// Fensterauswahl: ins temporäre Fenster.
    PlaceTemporary,
    /// Auswahl der zuletzt geschlossenen Fenster öffnen.
    RecentlyClosed,
    /// Zuletzt geschlossenes Fenster wieder öffnen: Index in [Purpose::Reopen].
    Reopen(usize),
}

/// Eintrag der gemeinsamen Tabelle für Menü und Hilfe: Taste, Grundbeschriftung im Menü und
/// Erklärung in der Hilfe. Punkte mit wechselnder Beschriftung (z. B. „Delete 3 marked“) nehmen
/// hier nur die Taste.
pub struct Entry {
    pub hotkey: char,
    /// Tasten, wie die Hilfe sie nennt, falls nicht nur `hotkey` (sonst leer).
    keys: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

const fn entry(hotkey: char, label: &'static str, help: &'static str) -> Entry {
    Entry {
        hotkey,
        keys: "",
        label,
        help,
    }
}

impl Entry {
    /// Tasten, wie die Hilfe sie nennt.
    pub fn keys(&self) -> String {
        if self.keys.is_empty() {
            self.hotkey.to_string()
        } else {
            self.keys.to_string()
        }
    }
}

pub const NEW_SESSION: Entry = entry(
    'n',
    "New session",
    "New session: pick a directory, then a window",
);
pub const CLOSE_WINDOW: Entry = entry(
    'c',
    "Close window",
    "Close the window; the session keeps running in the background",
);
pub const RECENTLY_CLOSED: Entry = entry(
    'r',
    "Recently closed …",
    "Reopen a running session whose window was closed; in a window it replaces that window",
);
pub const DELETE: Entry = entry(
    'x',
    "Delete session",
    "Delete the session for good; in the list all marked sessions",
);
pub const GROUP: Entry = entry(
    'g',
    "Group …",
    "Put the session (in the list: all marked) into a group; None removes it",
);
pub const NOTE: Entry = entry(
    't',
    "Note …",
    "Write, edit or remove the note of the session; it shows at the bottom of its window and as ✎ in the list",
);
pub const NOTEPAD: Entry = entry(
    'p',
    "Notepad",
    "Show the notepad in a window of your choice: free text, independent of sessions, saved automatically",
);
pub const TODO: Entry = entry(
    'd',
    "Todo list",
    "Show the todo list in a window of your choice: items to tick off, independent of sessions, saved automatically",
);
pub const NOTES: Entry = entry(
    'm',
    "Notes overview",
    "Show the notes of all sessions in a window of your choice; Enter or 1…4 opens a session",
);
pub const COLLAPSE: Entry = Entry {
    hotkey: 'f',
    keys: "f / ← →",
    label: "Collapse group",
    help: "List only: collapse or expand the group or directory of the selection",
};
pub const COLLAPSE_ALL: Entry = entry(
    'a',
    "Collapse all groups",
    "List only: collapse all groups and directories",
);
pub const EXPAND_ALL: Entry = entry(
    'e',
    "Expand all groups",
    "List only: expand all groups and directories",
);
pub const DELETE_OLDER: Entry = entry(
    'o',
    "Delete sessions older than …",
    "List only: delete saved sessions older than a day, week or month",
);
/// Fokuswechsel; die Fenster nehmen ihre Nummer als Taste, die Liste diese.
pub const FOCUS: Entry = Entry {
    hotkey: '0',
    keys: "1…4 / 0",
    label: "Focus session list",
    help: "Focus this window / the session list",
};
pub const TOGGLE_LIST: Entry = entry('s', "Hide session list", "Hide or show the session list");
pub const TOGGLE_ZOOM: Entry = entry(
    'z',
    "Zoom window",
    "From 2 windows: give the focused one the whole area; again to unzoom",
);
pub const UPDATE: Entry = entry(
    'u',
    "Update",
    "Install a newer release (only when one is available)",
);
pub const HELP: Entry = entry('h', "Help", "Show this help");
pub const QUIT: Entry = entry(
    'q',
    "Quit claude-tui",
    "Quit claude-tui; all sessions end (asks while any run)",
);

/// Alle Punkte des Befehlsmenüs in Menüreihenfolge.
pub const ENTRIES: [&Entry; 19] = [
    &NEW_SESSION,
    &CLOSE_WINDOW,
    &RECENTLY_CLOSED,
    &DELETE,
    &GROUP,
    &NOTE,
    &NOTEPAD,
    &TODO,
    &NOTES,
    &COLLAPSE,
    &COLLAPSE_ALL,
    &EXPAND_ALL,
    &DELETE_OLDER,
    &FOCUS,
    &TOGGLE_LIST,
    &TOGGLE_ZOOM,
    &UPDATE,
    &HELP,
    &QUIT,
];

#[derive(Debug)]
pub struct Item {
    pub hotkey: char,
    pub label: String,
    pub action: Action,
}

/// Wofür die Fensterauswahl geöffnet wurde.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Neue Session im gewählten Verzeichnis.
    NewSession(PathBuf),
    /// Vorhandenen Inhalt anzeigen: laufende Session, Notizblock, Todo-Liste usw.
    Show(Pane),
    /// Gespeicherte Claude-Session fortsetzen (`--resume`).
    Resume(String),
    /// Claude-Session, deren Löschen bestätigt werden soll.
    Delete(String),
    /// Mehrere Claude-Sessions (markiert oder nach Alter), deren Löschen bestätigt werden soll.
    DeleteMany(Vec<String>),
    /// Claude-Session, die außerhalb von claude-tui läuft und trotzdem geöffnet werden soll; mit
    /// Ziel (`None` = danach Fensterauswahl).
    OpenElsewhere(String, Option<Spot>),
    /// Claude-IDs der zuletzt geschlossenen Fenster, wie sie die Auswahl zeigt.
    Reopen(Vec<String>),
}

#[derive(Debug)]
pub struct Menu {
    pub title: String,
    pub items: Vec<Item>,
    pub selected: usize,
    pub purpose: Option<Purpose>,
}

/// Ergebnis eines Tastendrucks im offenen Menü.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
    Run(Action),
}

/// Was das Befehlsmenü über den aktuellen Zustand wissen muss.
pub struct Context<'a> {
    pub focus: Focus,
    /// Welche Fensterplätze belegt sind.
    pub windows: [bool; MAX_WINDOWS],
    /// Die in der Liste ausgewählte Session hat ein Fenster.
    pub selected_has_window: bool,
    /// Das Fenster mit Fokus zeigt keine Session (Notizblock, Todo-Liste oder Notizübersicht).
    pub panel_focused: bool,
    /// In der Liste ist eine Session ausgewählt.
    pub has_selection: bool,
    /// Version eines verfügbaren Updates (`None` = keins oder läuft schon).
    pub update: Option<&'a str>,
    /// Die Sessionliste ist ausgeblendet.
    pub list_hidden: bool,
    /// Das Fenster mit Fokus ist gezoomt.
    pub zoomed: bool,
    /// Anzahl der in der Liste markierten Sessions.
    pub marked: usize,
    /// Abschnitt der Auswahl in der Liste ist eingeklappt (`None` = keine Auswahl).
    pub collapsed: Option<bool>,
    /// Mindestens ein Abschnitt ist ausgeklappt.
    pub any_expanded: bool,
    /// Mindestens ein Abschnitt ist eingeklappt.
    pub any_collapsed: bool,
    /// Es gibt zuletzt geschlossene Fenster zum Wiederöffnen.
    pub recently_closed: bool,
}

impl Menu {
    pub fn commands(ctx: &Context) -> Self {
        let mut items = Vec::new();
        let mut add = |entry: &Entry, label: &str, action| {
            items.push(Item {
                hotkey: entry.hotkey,
                label: label.to_string(),
                action,
            })
        };
        let in_window = matches!(ctx.focus, Focus::Window(_));
        // Gibt es eine Session, auf die sich x, g und t beziehen (im Fenster bzw. in der Liste)?
        let one_session = if in_window {
            !ctx.panel_focused
        } else {
            ctx.has_selection
        };

        add(&NEW_SESSION, NEW_SESSION.label, Action::NewSession);
        if in_window || ctx.selected_has_window {
            add(&CLOSE_WINDOW, CLOSE_WINDOW.label, Action::CloseWindow);
        }
        if ctx.recently_closed {
            add(
                &RECENTLY_CLOSED,
                RECENTLY_CLOSED.label,
                Action::RecentlyClosed,
            );
        }
        // Markierte Sessions gelten nur mit Fokus auf der Liste; im Fenster meint x dessen Session.
        if !in_window && ctx.marked > 0 {
            let label = format!("Delete {} marked", ctx.marked);
            add(&DELETE, &label, Action::DeleteSession);
        } else if one_session {
            add(&DELETE, DELETE.label, Action::DeleteSession);
        }
        if !in_window && ctx.marked > 0 {
            let label = format!("Group {} marked …", ctx.marked);
            add(&GROUP, &label, Action::SetGroup);
        } else if one_session {
            add(&GROUP, GROUP.label, Action::SetGroup);
        }
        // Gilt immer der einen Session mit Fokus bzw. Auswahl, nie den markierten.
        if one_session {
            add(&NOTE, NOTE.label, Action::EditNote);
        }
        add(&NOTEPAD, NOTEPAD.label, Action::ShowNotepad);
        add(&TODO, TODO.label, Action::ShowTodo);
        add(&NOTES, NOTES.label, Action::ShowNotes);
        if let (false, Some(collapsed)) = (in_window, ctx.collapsed) {
            let label = if collapsed {
                "Expand group"
            } else {
                COLLAPSE.label
            };
            add(&COLLAPSE, label, Action::ToggleCollapsed);
        }
        if !in_window && ctx.any_expanded {
            add(&COLLAPSE_ALL, COLLAPSE_ALL.label, Action::CollapseAll(true));
        }
        if !in_window && ctx.any_collapsed {
            add(&EXPAND_ALL, EXPAND_ALL.label, Action::CollapseAll(false));
        }
        // Betrifft die ganze Liste, nicht das Fenster – deshalb nur mit Fokus auf der Liste.
        if !in_window {
            add(&DELETE_OLDER, DELETE_OLDER.label, Action::DeleteOlder);
        }
        let open = ctx.windows.iter().filter(|&&o| o).count();
        for i in (0..MAX_WINDOWS).filter(|&i| ctx.windows[i]) {
            if ctx.focus != Focus::Window(i) {
                let window = Entry {
                    hotkey: digit(i + 1),
                    ..FOCUS
                };
                let label = format!("Focus window {}", i + 1);
                add(&window, &label, Action::Focus(Focus::Window(i)));
            }
        }
        if in_window {
            add(&FOCUS, FOCUS.label, Action::Focus(Focus::List));
        }
        // Ausblenden nur, wenn ein Fenster den Platz übernimmt.
        if ctx.list_hidden {
            add(&TOGGLE_LIST, "Show session list", Action::ToggleList);
        } else if open > 0 {
            add(&TOGGLE_LIST, TOGGLE_LIST.label, Action::ToggleList);
        }
        // Zoomen lohnt erst ab zwei Fenstern; aufheben muss immer gehen.
        if ctx.zoomed {
            add(&TOGGLE_ZOOM, "Unzoom", Action::ToggleZoom);
        } else if in_window && open >= 2 {
            add(&TOGGLE_ZOOM, TOGGLE_ZOOM.label, Action::ToggleZoom);
        }
        if let Some(version) = ctx.update {
            let label = format!("Update to v{version}");
            add(&UPDATE, &label, Action::Update);
        }
        add(&HELP, HELP.label, Action::Help);
        add(&QUIT, QUIT.label, Action::Quit);

        let title = match ctx.focus {
            Focus::Window(i) => format!("Menue {}", i + 1),
            Focus::List => "Session list".to_string(),
        };
        Self {
            title,
            items,
            selected: 0,
            purpose: None,
        }
    }

    /// Fensterauswahl: jedes der vier Fenster, belegte ersetzen, freie neu öffnen, dazu das
    /// temporäre Fenster.
    pub fn pick_window(purpose: Purpose, title: String, occupied: [bool; MAX_WINDOWS]) -> Self {
        let temporary = Item {
            hotkey: '0',
            label: "Temporary window".to_string(),
            action: Action::PlaceTemporary,
        };
        let items: Vec<Item> = (0..MAX_WINDOWS)
            .map(|i| Item {
                hotkey: digit(i + 1),
                label: if occupied[i] {
                    format!("Replace Window {}", i + 1)
                } else {
                    format!("New Window {}", i + 1)
                },
                action: Action::Place(i),
            })
            .chain([temporary])
            .collect();
        Self {
            title,
            items,
            selected: 0,
            purpose: Some(purpose),
        }
    }

    /// Sicherheitsabfrage vor dem endgültigen Löschen; Enter bestätigt.
    pub fn confirm_delete(title: &str, claude_id: String) -> Self {
        let item = |c, label: &str, action| Item {
            hotkey: c,
            label: label.to_string(),
            action,
        };
        Self {
            title: format!("Delete {title}?"),
            items: vec![
                item('y', "Yes, delete permanently", Action::ConfirmDelete),
                item('n', "No", Action::Cancel),
            ],
            selected: 0,
            purpose: Some(Purpose::Delete(claude_id)),
        }
    }

    /// Zuletzt geschlossene Fenster: `(Claude-ID, Beschriftung)`, neueste zuerst, Tasten 1–9.
    pub fn recently_closed(sessions: Vec<(String, String)>) -> Self {
        let (ids, items) = sessions
            .into_iter()
            .take(9)
            .enumerate()
            .map(|(i, (claude_id, label))| {
                let item = Item {
                    hotkey: digit(i + 1),
                    label,
                    action: Action::Reopen(i),
                };
                (claude_id, item)
            })
            .unzip();
        Self {
            title: "Recently closed".to_string(),
            items,
            selected: 0,
            purpose: Some(Purpose::Reopen(ids)),
        }
    }

    /// Auswahl der Altersgrenze für das Löschen alter Sessions.
    pub fn pick_age() -> Self {
        let items = AGES
            .iter()
            .map(|&(hotkey, days)| Item {
                hotkey,
                label: age_label(days).to_string(),
                action: Action::DeleteOlderThan(days),
            })
            .collect();
        Self {
            title: "Delete sessions older than".to_string(),
            items,
            selected: 0,
            purpose: None,
        }
    }

    /// Sicherheitsabfrage vor dem Löschen mehrerer Sessions. Enter wählt „No“: ein versehentliches
    /// Enter soll nicht gleich viele Sessions kosten.
    pub fn confirm_delete_many(question: String, claude_ids: Vec<String>) -> Self {
        let item = |c, label: &str, action| Item {
            hotkey: c,
            label: label.to_string(),
            action,
        };
        Self {
            title: question,
            items: vec![
                item('y', "Yes, delete permanently", Action::ConfirmDelete),
                item('n', "No", Action::Cancel),
            ],
            selected: 1,
            purpose: Some(Purpose::DeleteMany(claude_ids)),
        }
    }

    /// Rückfrage, bevor eine anderswo laufende Session ein zweites Mal gestartet wird. Enter wählt
    /// „No“: zwei Prozesse in einem Protokoll sollen nie aus Versehen entstehen.
    pub fn confirm_open(title: &str, claude_id: String, spot: Option<Spot>) -> Self {
        let item = |c, label: &str, action| Item {
            hotkey: c,
            label: label.to_string(),
            action,
        };
        // Lange Titel kürzen, damit die Frage selbst im Rahmen sichtbar bleibt.
        let title = shorten(title, 30);
        Self {
            title: format!("{title} runs elsewhere – open anyway?"),
            items: vec![
                item('y', "Yes, open a second instance", Action::ConfirmOpen),
                item('n', "No", Action::Cancel),
            ],
            selected: 1,
            purpose: Some(Purpose::OpenElsewhere(claude_id, spot)),
        }
    }

    /// Rückfrage vor dem Beenden, solange `running` Sessions laufen – sie enden mit claude-tui.
    /// Enter wählt „No“: ein versehentliches Enter soll keine laufende Arbeit abbrechen.
    pub fn confirm_quit(running: usize) -> Self {
        let item = |c, label: &str, action| Item {
            hotkey: c,
            label: label.to_string(),
            action,
        };
        let verb = if running == 1 { "is" } else { "are" };
        Self {
            title: format!("{} {verb} running – quit anyway?", sessions(running)),
            items: vec![
                item('y', "Yes, quit and end them", Action::ConfirmQuit),
                item('n', "No", Action::Cancel),
            ],
            selected: 1,
            purpose: None,
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Outcome {
        let count = self.items.len();
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Up if count > 0 => {
                self.selected = (self.selected + count - 1) % count;
                Outcome::Stay
            }
            KeyCode::Down if count > 0 => {
                self.selected = (self.selected + 1) % count;
                Outcome::Stay
            }
            KeyCode::Enter => self
                .items
                .get(self.selected)
                .map_or(Outcome::Close, |item| Outcome::Run(item.action)),
            // Ctrl+A noch einmal schließt das Menü wieder.
            _ if keys::is_prefix(key) => Outcome::Close,
            KeyCode::Char(c) => self
                .items
                .iter()
                .find(|item| item.hotkey == c)
                .map_or(Outcome::Stay, |item| Outcome::Run(item.action)),
            _ => Outcome::Stay,
        }
    }
}

/// Wählbare Altersgrenzen: Taste und Tage.
const AGES: [(char, u64); 3] = [('d', 1), ('w', 7), ('m', 30)];

/// „1 day“, „1 week“, „1 month“ bzw. „N days“.
pub fn age_label(days: u64) -> String {
    match days {
        1 => "1 day".to_string(),
        7 => "1 week".to_string(),
        30 => "1 month".to_string(),
        n => format!("{n} days"),
    }
}

/// Kürzt hinten mit „…“ auf höchstens `max` Zeichen.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// „1 session“ bzw. „N sessions“.
pub fn sessions(n: usize) -> String {
    if n == 1 {
        "1 session".to_string()
    } else {
        format!("{n} sessions")
    }
}

fn digit(n: usize) -> char {
    char::from_digit(n as u32, 10).unwrap_or('?')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Fenster 1 bis `n` offen.
    fn open(n: usize) -> [bool; MAX_WINDOWS] {
        std::array::from_fn(|i| i < n)
    }

    #[test]
    fn window_context_offers_close_and_list_focus() {
        let windows = open(2);
        let menu = Menu::commands(&Context {
            focus: Focus::Window(0),
            windows,
            selected_has_window: true,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        let actions: Vec<Action> = menu.items.iter().map(|i| i.action).collect();
        assert!(actions.contains(&Action::CloseWindow));
        assert!(actions.contains(&Action::Focus(Focus::List)));
        assert!(actions.contains(&Action::Focus(Focus::Window(1))));
        assert!(!actions.contains(&Action::Focus(Focus::Window(0))));
    }

    #[test]
    fn window_context_title_names_menu_number() {
        let windows = open(2);
        let menu = Menu::commands(&Context {
            focus: Focus::Window(1),
            windows,
            selected_has_window: true,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        assert_eq!(menu.title, "Menue 2");
    }

    #[test]
    fn list_context_hides_close_without_window() {
        let windows = open(1);
        let menu = Menu::commands(&Context {
            focus: Focus::List,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        assert!(menu.items.iter().all(|i| i.action != Action::CloseWindow));
    }

    fn toggle_label(windows: usize, list_hidden: bool) -> Option<String> {
        let windows = open(windows);
        let menu = Menu::commands(&Context {
            focus: Focus::List,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        menu.items
            .iter()
            .find(|i| i.action == Action::ToggleList)
            .map(|i| format!("{} {}", i.hotkey, i.label))
    }

    #[test]
    fn session_list_toggle_needs_a_window_to_hide() {
        assert_eq!(
            toggle_label(1, false).as_deref(),
            Some("s Hide session list")
        );
        assert_eq!(
            toggle_label(1, true).as_deref(),
            Some("s Show session list")
        );
        assert_eq!(toggle_label(0, false), None);
    }

    fn zoom_label(focus: Focus, windows: usize, zoomed: bool) -> Option<String> {
        let windows = open(windows);
        let menu = Menu::commands(&Context {
            focus,
            windows,
            selected_has_window: true,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        menu.items
            .iter()
            .find(|i| i.action == Action::ToggleZoom)
            .map(|i| format!("{} {}", i.hotkey, i.label))
    }

    #[test]
    fn zoom_only_in_window_with_two_windows_but_unzoom_always() {
        let w0 = Focus::Window(0);
        assert_eq!(zoom_label(w0, 2, false).as_deref(), Some("z Zoom window"));
        assert_eq!(zoom_label(w0, 1, false), None);
        assert_eq!(zoom_label(Focus::List, 3, false), None);
        assert_eq!(zoom_label(w0, 4, true).as_deref(), Some("z Unzoom"));
        assert_eq!(zoom_label(w0, 1, true).as_deref(), Some("z Unzoom"));
    }

    #[test]
    fn hotkey_and_arrows_select_items() {
        let mut menu = Menu::pick_window(
            Purpose::NewSession(PathBuf::from(".")),
            "Neu".into(),
            open(2),
        );
        assert_eq!(menu.items.len(), 5);
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('3'))),
            Outcome::Run(Action::Place(2))
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('0'))),
            Outcome::Run(Action::PlaceTemporary)
        );
        assert_eq!(menu.on_key(&key(KeyCode::Up)), Outcome::Stay);
        assert_eq!(menu.on_key(&key(KeyCode::Up)), Outcome::Stay);
        assert_eq!(
            menu.on_key(&key(KeyCode::Enter)),
            Outcome::Run(Action::Place(3))
        );
        assert_eq!(menu.on_key(&key(KeyCode::Esc)), Outcome::Close);
    }

    #[test]
    fn prefix_again_closes_menu() {
        let mut menu = Menu::pick_window(
            Purpose::NewSession(PathBuf::from(".")),
            "Neu".into(),
            open(1),
        );
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(menu.on_key(&ctrl_a), Outcome::Close);
    }

    #[test]
    fn pick_window_offers_every_number() {
        let occupied = [false, true, false, true];
        let menu = Menu::pick_window(
            Purpose::NewSession(PathBuf::from(".")),
            "Neu".into(),
            occupied,
        );
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "New Window 1",
                "Replace Window 2",
                "New Window 3",
                "Replace Window 4",
                "Temporary window"
            ]
        );
    }

    #[test]
    fn temporary_window_for_every_content() {
        for pane in [Pane::Session(7), Pane::Notepad, Pane::Todo, Pane::Notes] {
            let menu = Menu::pick_window(Purpose::Show(pane), "Show".into(), open(0));
            let last = menu.items.last().map(|i| (i.hotkey, i.action));
            assert_eq!(last, Some(('0', Action::PlaceTemporary)));
        }
    }

    #[test]
    fn focus_items_follow_window_numbers() {
        let menu = Menu::commands(&Context {
            focus: Focus::Window(1),
            windows: [false, true, false, true],
            selected_has_window: true,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        let focus: Vec<String> = menu
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::Focus(Focus::Window(_))))
            .map(|i| format!("{} {}", i.hotkey, i.label))
            .collect();
        assert_eq!(focus, ["4 Focus window 4"]);
    }

    #[test]
    fn notepad_window_offers_no_session_items() {
        let menu = Menu::commands(&Context {
            focus: Focus::Window(0),
            windows: [true, true, false, false],
            selected_has_window: true,
            panel_focused: true,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 2,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        let hotkeys: String = menu.items.iter().map(|i| i.hotkey).collect();
        for absent in ['x', 'g', 't'] {
            assert!(!hotkeys.contains(absent), "{absent} in {hotkeys}");
        }
        for present in ['n', 'c', 'p', 'd', 'm', 'z'] {
            assert!(hotkeys.contains(present), "{present} fehlt in {hotkeys}");
        }
    }

    #[test]
    fn marked_sessions_change_delete_in_list_only() {
        let windows = open(1);
        let ctx = |focus| Context {
            focus,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 3,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        };
        let label = |menu: Menu| {
            menu.items
                .into_iter()
                .find(|i| i.hotkey == 'x')
                .map(|i| i.label)
        };
        assert_eq!(
            label(Menu::commands(&ctx(Focus::List))).as_deref(),
            Some("Delete 3 marked")
        );
        assert_eq!(
            label(Menu::commands(&ctx(Focus::Window(0)))).as_deref(),
            Some("Delete session")
        );
    }

    #[test]
    fn collapse_items_follow_state_and_appear_in_list_only() {
        let windows = open(1);
        let ctx = |focus, collapsed, any_expanded, any_collapsed| Context {
            focus,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed,
            any_expanded,
            any_collapsed,
            recently_closed: false,
        };
        let items = |menu: Menu| {
            menu.items
                .into_iter()
                .filter(|i| matches!(i.hotkey, 'f' | 'a' | 'e'))
                .map(|i| (i.label, i.action))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            items(Menu::commands(&ctx(Focus::List, Some(false), true, false))),
            [
                ("Collapse group".to_string(), Action::ToggleCollapsed),
                ("Collapse all groups".to_string(), Action::CollapseAll(true)),
            ]
        );
        assert_eq!(
            items(Menu::commands(&ctx(Focus::List, Some(true), true, true))),
            [
                ("Expand group".to_string(), Action::ToggleCollapsed),
                ("Collapse all groups".to_string(), Action::CollapseAll(true)),
                ("Expand all groups".to_string(), Action::CollapseAll(false)),
            ]
        );
        assert!(items(Menu::commands(&ctx(Focus::List, None, false, false))).is_empty());
        let in_window = ctx(Focus::Window(0), Some(false), true, true);
        assert!(items(Menu::commands(&in_window)).is_empty());

        // Ctrl+A schließt das Menü, auch wenn `a` (Collapse all) darin steht.
        let mut menu = Menu::commands(&ctx(Focus::List, Some(false), true, false));
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(menu.on_key(&ctrl_a), Outcome::Close);
    }

    #[test]
    fn group_item_in_window_and_list_with_marked_count() {
        let windows = open(1);
        let ctx = |focus, has_selection, marked| Context {
            focus,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        };
        let label = |menu: Menu| {
            menu.items
                .into_iter()
                .find(|i| i.action == Action::SetGroup)
                .map(|i| format!("{} {}", i.hotkey, i.label))
        };
        assert_eq!(
            label(Menu::commands(&ctx(Focus::Window(0), false, 2))).as_deref(),
            Some("g Group …")
        );
        assert_eq!(
            label(Menu::commands(&ctx(Focus::List, true, 0))).as_deref(),
            Some("g Group …")
        );
        assert_eq!(
            label(Menu::commands(&ctx(Focus::List, true, 2))).as_deref(),
            Some("g Group 2 marked …")
        );
        assert_eq!(label(Menu::commands(&ctx(Focus::List, false, 0))), None);
    }

    #[test]
    fn every_menu_key_is_in_the_shared_table_once() {
        let mut hotkeys: Vec<char> = ENTRIES.iter().map(|e| e.hotkey).collect();
        hotkeys.sort_unstable();
        hotkeys.dedup();
        assert_eq!(hotkeys.len(), ENTRIES.len());

        for (focus, marked, flag) in [
            (Focus::List, 0, false),
            (Focus::List, 2, true),
            (Focus::Window(1), 0, true),
        ] {
            let menu = Menu::commands(&Context {
                focus,
                windows: open(if flag { 2 } else { 4 }),
                selected_has_window: true,
                panel_focused: false,
                has_selection: true,
                update: Some("9.9.9"),
                list_hidden: flag,
                zoomed: flag,
                marked,
                collapsed: None,
                any_expanded: false,
                any_collapsed: false,
                recently_closed: false,
            });
            for item in menu.items {
                let known = item.hotkey.is_ascii_digit() || hotkeys.contains(&item.hotkey);
                assert!(known, "{} {}", item.hotkey, item.label);
            }
        }
    }

    #[test]
    fn help_item_opens_help() {
        let windows = open(1);
        let mut menu = Menu::commands(&Context {
            focus: Focus::List,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection: false,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        });
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('h'))),
            Outcome::Run(Action::Help)
        );
    }

    #[test]
    fn delete_older_only_in_session_list() {
        let windows = open(1);
        let ctx = |focus| Context {
            focus,
            windows,
            selected_has_window: false,
            panel_focused: false,
            has_selection: false,
            update: None,
            list_hidden: false,
            zoomed: false,
            marked: 0,
            collapsed: None,
            any_expanded: false,
            any_collapsed: false,
            recently_closed: false,
        };
        let has_older = |menu: Menu| menu.items.iter().any(|i| i.action == Action::DeleteOlder);
        assert!(has_older(Menu::commands(&ctx(Focus::List))));
        assert!(!has_older(Menu::commands(&ctx(Focus::Window(0)))));
        let has_closed = |menu: Menu| {
            menu.items
                .iter()
                .any(|i| (i.hotkey, i.action) == ('r', Action::RecentlyClosed))
        };
        assert!(!has_closed(Menu::commands(&ctx(Focus::List))));
        for focus in [Focus::List, Focus::Window(0)] {
            let menu = Menu::commands(&Context {
                recently_closed: true,
                ..ctx(focus)
            });
            assert!(has_closed(menu));
        }
    }

    #[test]
    fn age_picker_offers_day_week_month() {
        let mut menu = Menu::pick_age();
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('w'))),
            Outcome::Run(Action::DeleteOlderThan(7))
        );
        assert_eq!(age_label(30), "1 month");
    }

    #[test]
    fn recently_closed_numbers_sessions_newest_first() {
        let sessions = (0..12)
            .map(|i| (format!("id{i}"), format!("s{i}")))
            .collect();
        let mut menu = Menu::recently_closed(sessions);
        assert_eq!(menu.items.len(), 9);
        assert_eq!(menu.items[0].hotkey, '1');
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('3'))),
            Outcome::Run(Action::Reopen(2))
        );
        let Some(Purpose::Reopen(ids)) = menu.purpose else {
            panic!("purpose")
        };
        assert_eq!(ids[2], "id2");
    }

    #[test]
    fn bulk_delete_defaults_to_no() {
        let mut menu = Menu::confirm_delete_many("Delete 2 sessions?".into(), vec!["a".into()]);
        assert_eq!(
            menu.on_key(&key(KeyCode::Enter)),
            Outcome::Run(Action::Cancel)
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('y'))),
            Outcome::Run(Action::ConfirmDelete)
        );
    }

    #[test]
    fn delete_needs_confirmation_and_enter_confirms() {
        let mut menu = Menu::confirm_delete("Alte Analyse", "abc".into());
        assert_eq!(menu.purpose, Some(Purpose::Delete("abc".into())));
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('n'))),
            Outcome::Run(Action::Cancel)
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Enter)),
            Outcome::Run(Action::ConfirmDelete)
        );
        assert_eq!(menu.on_key(&key(KeyCode::Esc)), Outcome::Close);
    }

    #[test]
    fn quit_with_running_sessions_defaults_to_no() {
        let mut menu = Menu::confirm_quit(2);
        assert_eq!(menu.title, "2 sessions are running – quit anyway?");
        assert_eq!(
            Menu::confirm_quit(1).title,
            "1 session is running – quit anyway?"
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Enter)),
            Outcome::Run(Action::Cancel)
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('y'))),
            Outcome::Run(Action::ConfirmQuit)
        );
    }

    #[test]
    fn open_elsewhere_needs_confirmation_and_enter_cancels() {
        let mut menu = Menu::confirm_open("Analyse", "abc".into(), Some(Spot::Window(1)));
        assert_eq!(menu.title, "Analyse runs elsewhere – open anyway?");
        assert_eq!(
            menu.purpose,
            Some(Purpose::OpenElsewhere("abc".into(), Some(Spot::Window(1))))
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Enter)),
            Outcome::Run(Action::Cancel)
        );
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('y'))),
            Outcome::Run(Action::ConfirmOpen)
        );
    }
}
