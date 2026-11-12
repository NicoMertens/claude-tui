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
    /// Neuere Version von GitHub installieren.
    Update,
    Quit,
    /// Fensterauswahl: Ziel-Fensterindex.
    Place(usize),
}

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
        let mut add = |hotkey, label: &str, action| {
            items.push(Item {
                hotkey,
                label: label.to_string(),
                action,
            })
        };
        let in_window = matches!(ctx.focus, Focus::Window(_));

        add('n', "New session", Action::NewSession);
        if in_window || ctx.selected_has_window {
            add('c', "Close window", Action::CloseWindow);
        }
        // Markierte Sessions gelten nur mit Fokus auf der Liste; im Fenster meint x dessen Session.
        if !in_window && ctx.marked > 0 {
            let label = format!("Delete {} marked", ctx.marked);
            add('x', &label, Action::DeleteSession);
        } else if in_window || ctx.has_selection {
            add('x', "Delete session", Action::DeleteSession);
        }
        add('o', "Delete sessions older than …", Action::DeleteOlder);
        for i in 0..ctx.windows.len() {
            if ctx.focus != Focus::Window(i) {
                let label = format!("Focus window {}", i + 1);
                add(digit(i + 1), &label, Action::Focus(Focus::Window(i)));
            }
        }
        if in_window {
            add('0', "Focus session list", Action::Focus(Focus::List));
        }
        if ctx.waiting {
            add('w', "Next waiting session", Action::NextWaiting);
        }
        // Ausblenden nur, wenn ein Fenster den Platz übernimmt.
        if ctx.list_hidden {
            add('s', "Show session list", Action::ToggleList);
        } else if !ctx.windows.is_empty() {
            add('s', "Hide session list", Action::ToggleList);
        }
        // Die Anordnung ist nur bei genau zwei Fenstern wählbar.
        if ctx.windows.len() == 2 {
            let label = if ctx.stacked {
                "Windows side by side"
            } else {
                "Stack windows"
            };
            add('l', label, Action::ToggleStacked);
        }
        // Zoomen lohnt erst ab zwei Fenstern; aufheben muss immer gehen.
        if ctx.zoomed {
            add('z', "Unzoom", Action::ToggleZoom);
        } else if in_window && ctx.windows.len() >= 2 {
            add('z', "Zoom window", Action::ToggleZoom);
        }
        if let Some(version) = ctx.update {
            let label = format!("Update to v{version}");
            add('u', &label, Action::Update);
        }
        add('q', "Quit claude-tui", Action::Quit);

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
        const TITLE_MAX: usize = 30;
        let title = if title.chars().count() > TITLE_MAX {
            let keep: String = title.chars().take(TITLE_MAX - 1).collect();
            format!("{keep}…")
        } else {
            title.to_string()
        };
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
