//! Das Ctrl+A-Menü (Overlay über dem fokussierten Fenster) und die Fensterauswahl.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use crate::{
    keys,
    windows::{Focus, SessionId, MAX_WINDOWS},
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
    /// Zwei Fenster übereinander bzw. nebeneinander anordnen.
    ToggleStacked,
    /// Fenster mit Fokus auf die ganze Fensterfläche zoomen bzw. den Zoom aufheben.
    ToggleZoom,
    /// Zur nächsten Session springen, die auf dich wartet.
    NextWaiting,
    /// Gruppen-Dialog für die Session mit Fokus bzw. die markierten Sessions öffnen.
    SetGroup,
    /// Neuere Version von GitHub installieren.
    Update,
    /// Hilfeseite öffnen.
    Help,
    Quit,
    /// Fensterauswahl: Ziel-Fensterindex.
    Place(usize),
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
pub const NEXT_WAITING: Entry = entry(
    'w',
    "Next waiting session",
    "Jump to the next session waiting for you (only when one waits)",
);
pub const TOGGLE_LIST: Entry = entry('s', "Hide session list", "Hide or show the session list");
pub const TOGGLE_STACKED: Entry = entry(
    'l',
    "Stack windows",
    "With exactly 2 windows: stack them or put them side by side",
);
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
pub const QUIT: Entry = entry('q', "Quit claude-tui", "Quit claude-tui; all sessions end");

/// Alle Punkte des Befehlsmenüs in Menüreihenfolge.
pub const ENTRIES: [&Entry; 13] = [
    &NEW_SESSION,
    &CLOSE_WINDOW,
    &DELETE,
    &GROUP,
    &DELETE_OLDER,
    &FOCUS,
    &NEXT_WAITING,
    &TOGGLE_LIST,
    &TOGGLE_STACKED,
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
    /// Laufende Session anzeigen.
    Show(SessionId),
    /// Gespeicherte Claude-Session fortsetzen (`--resume`).
    Resume(String),
    /// Claude-Session, deren Löschen bestätigt werden soll.
    Delete(String),
    /// Mehrere Claude-Sessions (markiert oder nach Alter), deren Löschen bestätigt werden soll.
    DeleteMany(Vec<String>),
    /// Claude-Session, die außerhalb von claude-tui läuft und trotzdem geöffnet werden soll; mit
    /// Zielfenster (`None` = danach Fensterauswahl).
    OpenElsewhere(String, Option<usize>),
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
    /// Sessionname je Fenster, in Fensterreihenfolge.
    pub windows: &'a [String],
    /// Die in der Liste ausgewählte Session hat ein Fenster.
    pub selected_has_window: bool,
    /// In der Liste ist eine Session ausgewählt.
    pub has_selection: bool,
    /// Version eines verfügbaren Updates (`None` = keins oder läuft schon).
    pub update: Option<&'a str>,
    /// Die Sessionliste ist ausgeblendet.
    pub list_hidden: bool,
    /// Zwei Fenster stehen übereinander.
    pub stacked: bool,
    /// Das Fenster mit Fokus ist gezoomt.
    pub zoomed: bool,
    /// Mindestens eine Session wartet auf dich.
    pub waiting: bool,
    /// Anzahl der in der Liste markierten Sessions.
    pub marked: usize,
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

        add(&NEW_SESSION, NEW_SESSION.label, Action::NewSession);
        if in_window || ctx.selected_has_window {
            add(&CLOSE_WINDOW, CLOSE_WINDOW.label, Action::CloseWindow);
        }
        // Markierte Sessions gelten nur mit Fokus auf der Liste; im Fenster meint x dessen Session.
        if !in_window && ctx.marked > 0 {
            let label = format!("Delete {} marked", ctx.marked);
            add(&DELETE, &label, Action::DeleteSession);
        } else if in_window || ctx.has_selection {
            add(&DELETE, DELETE.label, Action::DeleteSession);
        }
        if !in_window && ctx.marked > 0 {
            let label = format!("Group {} marked …", ctx.marked);
            add(&GROUP, &label, Action::SetGroup);
        } else if in_window || ctx.has_selection {
            add(&GROUP, GROUP.label, Action::SetGroup);
        }
        // Betrifft die ganze Liste, nicht das Fenster – deshalb nur mit Fokus auf der Liste.
        if !in_window {
            add(&DELETE_OLDER, DELETE_OLDER.label, Action::DeleteOlder);
        }
        for i in 0..ctx.windows.len() {
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
        if ctx.waiting {
            add(&NEXT_WAITING, NEXT_WAITING.label, Action::NextWaiting);
        }
        // Ausblenden nur, wenn ein Fenster den Platz übernimmt.
        if ctx.list_hidden {
            add(&TOGGLE_LIST, "Show session list", Action::ToggleList);
        } else if !ctx.windows.is_empty() {
            add(&TOGGLE_LIST, TOGGLE_LIST.label, Action::ToggleList);
        }
        // Die Anordnung ist nur bei genau zwei Fenstern wählbar.
        if ctx.windows.len() == 2 {
            let label = if ctx.stacked {
                "Windows side by side"
            } else {
                TOGGLE_STACKED.label
            };
            add(&TOGGLE_STACKED, label, Action::ToggleStacked);
        }
        // Zoomen lohnt erst ab zwei Fenstern; aufheben muss immer gehen.
        if ctx.zoomed {
            add(&TOGGLE_ZOOM, "Unzoom", Action::ToggleZoom);
        } else if in_window && ctx.windows.len() >= 2 {
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

    /// Fensterauswahl: belegte Fenster ersetzen oder (bei < 4) ein neues öffnen.
    pub fn pick_window(purpose: Purpose, title: String, open: usize) -> Self {
        let mut items: Vec<Item> = (0..open)
            .map(|i| Item {
                hotkey: digit(i + 1),
                label: format!("Replace Window {}", i + 1),
                action: Action::Place(i),
            })
            .collect();
        if open < MAX_WINDOWS {
            items.push(Item {
                hotkey: digit(open + 1),
                label: "New window".to_string(),
                action: Action::Place(open),
            });
        }
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
    pub fn confirm_open(title: &str, claude_id: String, slot: Option<usize>) -> Self {
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
            purpose: Some(Purpose::OpenElsewhere(claude_id, slot)),
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

    fn names(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("claude {i}")).collect()
    }

    #[test]
    fn window_context_offers_close_and_list_focus() {
        let windows = names(2);
        let menu = Menu::commands(&Context {
            focus: Focus::Window(0),
            windows: &windows,
            selected_has_window: true,
            has_selection: true,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 0,
        });
        let actions: Vec<Action> = menu.items.iter().map(|i| i.action).collect();
        assert!(actions.contains(&Action::CloseWindow));
        assert!(actions.contains(&Action::Focus(Focus::List)));
        assert!(actions.contains(&Action::Focus(Focus::Window(1))));
        assert!(!actions.contains(&Action::Focus(Focus::Window(0))));
    }

    #[test]
    fn window_context_title_names_menu_number() {
        let windows = names(2);
        let menu = Menu::commands(&Context {
            focus: Focus::Window(1),
            windows: &windows,
            selected_has_window: true,
            has_selection: true,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 0,
        });
        assert_eq!(menu.title, "Menue 2");
    }

    #[test]
    fn list_context_hides_close_without_window() {
        let windows = names(1);
        let menu = Menu::commands(&Context {
            focus: Focus::List,
            windows: &windows,
            selected_has_window: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 0,
        });
        assert!(menu.items.iter().all(|i| i.action != Action::CloseWindow));
    }

    fn toggle_label(windows: usize, list_hidden: bool) -> Option<String> {
        let windows = names(windows);
        let menu = Menu::commands(&Context {
            focus: Focus::List,
            windows: &windows,
            selected_has_window: false,
            has_selection: true,
            update: None,
            list_hidden,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 0,
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

    fn layout_label(windows: usize, stacked: bool) -> Option<String> {
        let windows = names(windows);
        let menu = Menu::commands(&Context {
            focus: Focus::Window(0),
            windows: &windows,
            selected_has_window: true,
            has_selection: true,
            update: None,
            list_hidden: false,
            stacked,
            zoomed: false,
            waiting: false,
            marked: 0,
        });
        menu.items
            .iter()
            .find(|i| i.action == Action::ToggleStacked)
            .map(|i| format!("{} {}", i.hotkey, i.label))
    }

    #[test]
    fn layout_toggle_only_with_two_windows() {
        assert_eq!(layout_label(2, false).as_deref(), Some("l Stack windows"));
        assert_eq!(
            layout_label(2, true).as_deref(),
            Some("l Windows side by side")
        );
        assert_eq!(layout_label(1, false), None);
        assert_eq!(layout_label(3, false), None);
    }

    fn zoom_label(focus: Focus, windows: usize, zoomed: bool) -> Option<String> {
        let windows = names(windows);
        let menu = Menu::commands(&Context {
            focus,
            windows: &windows,
            selected_has_window: true,
            has_selection: true,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed,
            waiting: false,
            marked: 0,
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
    fn next_waiting_only_when_a_session_waits() {
        let windows = names(1);
        let has_next = |waiting| {
            Menu::commands(&Context {
                focus: Focus::List,
                windows: &windows,
                selected_has_window: false,
                has_selection: true,
                update: None,
                list_hidden: false,
                stacked: false,
                zoomed: false,
                waiting,
                marked: 0,
            })
            .items
            .iter()
            .any(|i| i.hotkey == 'w' && i.action == Action::NextWaiting)
        };
        assert!(has_next(true));
        assert!(!has_next(false));
    }

    #[test]
    fn hotkey_and_arrows_select_items() {
        let mut menu = Menu::pick_window(Purpose::NewSession(PathBuf::from(".")), "Neu".into(), 2);
        assert_eq!(menu.items.len(), 3);
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('3'))),
            Outcome::Run(Action::Place(2))
        );
        assert_eq!(menu.on_key(&key(KeyCode::Up)), Outcome::Stay);
        assert_eq!(
            menu.on_key(&key(KeyCode::Enter)),
            Outcome::Run(Action::Place(2))
        );
        assert_eq!(menu.on_key(&key(KeyCode::Esc)), Outcome::Close);
    }

    #[test]
    fn prefix_again_closes_menu() {
        let mut menu = Menu::pick_window(Purpose::NewSession(PathBuf::from(".")), "Neu".into(), 1);
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(menu.on_key(&ctrl_a), Outcome::Close);
    }

    #[test]
    fn pick_window_without_new_window_when_full() {
        let menu = Menu::pick_window(Purpose::NewSession(PathBuf::from(".")), "Neu".into(), 4);
        let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Replace Window 1",
                "Replace Window 2",
                "Replace Window 3",
                "Replace Window 4"
            ]
        );
    }

    #[test]
    fn marked_sessions_change_delete_in_list_only() {
        let windows = names(1);
        let ctx = |focus| Context {
            focus,
            windows: &windows,
            selected_has_window: false,
            has_selection: true,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 3,
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
    fn group_item_in_window_and_list_with_marked_count() {
        let windows = names(1);
        let ctx = |focus, has_selection, marked| Context {
            focus,
            windows: &windows,
            selected_has_window: false,
            has_selection,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked,
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

        let windows = names(4);
        for (focus, marked, flag) in [
            (Focus::List, 0, false),
            (Focus::List, 2, true),
            (Focus::Window(1), 0, true),
        ] {
            let menu = Menu::commands(&Context {
                focus,
                windows: &windows[..if flag { 2 } else { 4 }],
                selected_has_window: true,
                has_selection: true,
                update: Some("9.9.9"),
                list_hidden: flag,
                stacked: flag,
                zoomed: flag,
                waiting: true,
                marked,
            });
            for item in menu.items {
                let known = item.hotkey.is_ascii_digit() || hotkeys.contains(&item.hotkey);
                assert!(known, "{} {}", item.hotkey, item.label);
            }
        }
    }

    #[test]
    fn help_item_opens_help() {
        let windows = names(1);
        let mut menu = Menu::commands(&Context {
            focus: Focus::List,
            windows: &windows,
            selected_has_window: false,
            has_selection: false,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 0,
        });
        assert_eq!(
            menu.on_key(&key(KeyCode::Char('h'))),
            Outcome::Run(Action::Help)
        );
    }

    #[test]
    fn delete_older_only_in_session_list() {
        let windows = names(1);
        let ctx = |focus| Context {
            focus,
            windows: &windows,
            selected_has_window: false,
            has_selection: false,
            update: None,
            list_hidden: false,
            stacked: false,
            zoomed: false,
            waiting: false,
            marked: 0,
        };
        let has_older = |menu: Menu| menu.items.iter().any(|i| i.action == Action::DeleteOlder);
        assert!(has_older(Menu::commands(&ctx(Focus::List))));
        assert!(!has_older(Menu::commands(&ctx(Focus::Window(0)))));
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
    fn open_elsewhere_needs_confirmation_and_enter_cancels() {
        let mut menu = Menu::confirm_open("Analyse", "abc".into(), Some(1));
        assert_eq!(menu.title, "Analyse runs elsewhere – open anyway?");
        assert_eq!(
            menu.purpose,
            Some(Purpose::OpenElsewhere("abc".into(), Some(1)))
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
