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
    /// Bestätigung: Session löschen (Ziel steht in [Purpose::Delete]).
    ConfirmDelete,
    /// Bestätigung: anderswo laufende Session trotzdem öffnen (Ziel in [Purpose::OpenElsewhere]).
    ConfirmOpen,
    Cancel,
    Focus(Focus),
    /// Sessionliste ein- bzw. ausblenden.
    ToggleList,
    /// Zwei Fenster übereinander bzw. nebeneinander anordnen.
    ToggleStacked,
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
        if in_window || ctx.has_selection {
            add('x', "Delete session", Action::DeleteSession);
        }
        for i in 0..ctx.windows.len() {
            if ctx.focus != Focus::Window(i) {
                let label = format!("Focus window {}", i + 1);
                add(digit(i + 1), &label, Action::Focus(Focus::Window(i)));
            }
        }
        if in_window {
            add('0', "Focus session list", Action::Focus(Focus::List));
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
