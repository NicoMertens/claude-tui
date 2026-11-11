//! Das Ctrl+A-Menü unten in der Sessionliste und die Fensterauswahl.

use crossterm::event::{KeyCode, KeyEvent};

use crate::{
    keys,
    windows::{Focus, SessionId, MAX_WINDOWS},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewSession,
    CloseWindow,
    KillSession,
    Focus(Focus),
    SendPrefix,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    NewSession,
    Show(SessionId),
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
    pub has_sessions: bool,
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

        add(Hotkey::Char('n'), "Neue Session", Action::NewSession);
        if in_window || ctx.selected_has_window {
            add(Hotkey::Char('c'), "Fenster schließen", Action::CloseWindow);
        }
        if in_window || ctx.has_sessions {
            add(Hotkey::Char('x'), "Session beenden", Action::KillSession);
        }
        for i in 0..ctx.windows.len() {
            if ctx.focus != Focus::Window(i) {
                let label = format!("Fokus Fenster {}", i + 1);
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
                "Fokus Sessionliste",
                Action::Focus(Focus::List),
            );
            add(Hotkey::Prefix, "Ctrl+A an Claude", Action::SendPrefix);
        }
        add(Hotkey::Char('q'), "claude-tui beenden", Action::Quit);

        let title = match ctx.focus {
            Focus::Window(i) => format!("Fenster {}", i + 1),
            Focus::List => "Sessionliste".to_string(),
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
                label: format!("{name} ersetzen"),
                action: Action::Place(i),
            })
            .collect();
        if windows.len() < MAX_WINDOWS {
            items.push(Item {
                hotkey: Hotkey::Char(digit(windows.len() + 1)),
                label: "neues Fenster".to_string(),
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
            has_sessions: true,
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
            has_sessions: true,
        });
        assert!(menu.items.iter().all(|i| i.action != Action::CloseWindow));
        assert!(menu.items.iter().all(|i| i.action != Action::SendPrefix));
    }

    #[test]
    fn hotkey_and_arrows_select_items() {
        let mut menu = Menu::pick_window(Purpose::NewSession, "Neu".into(), &names(2));
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
        let menu = Menu::pick_window(Purpose::NewSession, "Neu".into(), &names(4));
        assert_eq!(menu.items.len(), 4);
        assert!(menu.items.iter().all(|i| i.label.ends_with("ersetzen")));
    }
}
