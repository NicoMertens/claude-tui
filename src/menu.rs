//! Das Ctrl+A-Menü unten in der Sessionliste und die Fensterauswahl.

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
    Cancel,
    Focus(Focus),
    SendPrefix,
    /// Neuere Version von GitHub installieren.
    Update,
    Quit,
    /// Fensterauswahl: Ziel-Fensterindex.
    Place(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Char(char),
    Prefix,
}

impl Hotkey {
    pub fn label(self) -> String {
        match self {
            Hotkey::Char(c) => c.to_string(),
            Hotkey::Prefix => "^A".to_string(),
        }
    }
}

#[derive(Debug)]
pub struct Item {
    pub hotkey: Hotkey,
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

        add(Hotkey::Char('n'), "New session", Action::NewSession);
        if in_window || ctx.selected_has_window {
            add(Hotkey::Char('c'), "Close window", Action::CloseWindow);
        }
        if in_window || ctx.has_selection {
            add(Hotkey::Char('x'), "Delete session", Action::DeleteSession);
        }
        for i in 0..ctx.windows.len() {
            if ctx.focus != Focus::Window(i) {
                let label = format!("Focus window {}", i + 1);
                add(
                    Hotkey::Char(digit(i + 1)),
                    &label,
                    Action::Focus(Focus::Window(i)),
                );
            }
        }
        if in_window {
            add(
                Hotkey::Char('0'),
                "Focus session list",
                Action::Focus(Focus::List),
            );
            add(Hotkey::Prefix, "Send Ctrl+A to Claude", Action::SendPrefix);
        }
        if let Some(version) = ctx.update {
            let label = format!("Update to v{version}");
            add(Hotkey::Char('u'), &label, Action::Update);
        }
        add(Hotkey::Char('q'), "Quit claude-tui", Action::Quit);

        let title = match ctx.focus {
            Focus::Window(i) => format!("Window {}", i + 1),
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
    pub fn pick_window(purpose: Purpose, title: String, windows: &[String]) -> Self {
        let mut items: Vec<Item> = windows
            .iter()
            .enumerate()
            .map(|(i, name)| Item {
                hotkey: Hotkey::Char(digit(i + 1)),
                label: format!("Replace {name}"),
                action: Action::Place(i),
            })
            .collect();
        if windows.len() < MAX_WINDOWS {
            items.push(Item {
                hotkey: Hotkey::Char(digit(windows.len() + 1)),
                label: "New window".to_string(),
                action: Action::Place(windows.len()),
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
            hotkey: Hotkey::Char(c),
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
            _ => {
                let hotkey = if keys::is_prefix(key) {
                    Some(Hotkey::Prefix)
                } else if let KeyCode::Char(c) = key.code {
                    Some(Hotkey::Char(c))
                } else {
                    None
                };
                self.items
                    .iter()
                    .find(|item| Some(item.hotkey) == hotkey)
                    .map_or(Outcome::Stay, |item| Outcome::Run(item.action))
            }
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
        });
        let actions: Vec<Action> = menu.items.iter().map(|i| i.action).collect();
        assert!(actions.contains(&Action::CloseWindow));
        assert!(actions.contains(&Action::Focus(Focus::List)));
        assert!(actions.contains(&Action::Focus(Focus::Window(1))));
        assert!(!actions.contains(&Action::Focus(Focus::Window(0))));
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
        });
        assert!(menu.items.iter().all(|i| i.action != Action::CloseWindow));
        assert!(menu.items.iter().all(|i| i.action != Action::SendPrefix));
    }

    #[test]
    fn hotkey_and_arrows_select_items() {
        let mut menu = Menu::pick_window(
            Purpose::NewSession(PathBuf::from(".")),
            "Neu".into(),
            &names(2),
        );
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
    fn pick_window_without_new_window_when_full() {
        let menu = Menu::pick_window(
            Purpose::NewSession(PathBuf::from(".")),
            "Neu".into(),
            &names(4),
        );
        assert_eq!(menu.items.len(), 4);
        assert!(menu.items.iter().all(|i| i.label.starts_with("Replace")));
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
}
