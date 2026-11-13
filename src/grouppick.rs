//! Gruppen-Dialog: Eingabefeld für einen neuen Namen, darunter „None“ (aus der Gruppe nehmen) und
//! die zuletzt verwendeten bzw. vorhandenen Gruppen. Tippen filtert die Liste, ↑/↓ wählt daraus.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Eintrag der Auswahlliste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Aus der Gruppe nehmen.
    None,
    Group(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Cancel,
    /// Gewählte Gruppe (`None` = aus der Gruppe nehmen); getippte Namen noch unbereinigt.
    Chosen(Option<String>),
}

#[derive(Debug)]
pub struct GroupPicker {
    pub title: String,
    pub input: String,
    /// Mit ↑/↓ angesteuerter Eintrag aus [`Self::visible`]; hat Vorrang vor dem Eingabefeld.
    pub selected: Option<usize>,
    pub error: Option<&'static str>,
    /// Claude-IDs der Sessions, die zugeordnet werden (beim Öffnen festgelegt).
    pub targets: Vec<String>,
    choices: Vec<String>,
    /// „None“ nur, wenn eine der Sessions schon in einer Gruppe ist.
    offer_none: bool,
}

impl GroupPicker {
    pub fn new(
        title: String,
        targets: Vec<String>,
        choices: Vec<String>,
        offer_none: bool,
    ) -> Self {
        Self {
            title,
            input: String::new(),
            selected: None,
            error: None,
            targets,
            choices,
            offer_none,
        }
    }

    /// Einträge, die zur Eingabe passen (Teilstring ohne Groß-/Kleinschreibung). „None“ nur bei
    /// leerer Eingabe — sonst nähme ↓ + Enter nach dem Tippen versehentlich die Gruppe weg.
    pub fn visible(&self) -> Vec<Choice> {
        let filter = self.input.trim().to_lowercase();
        let groups = self
            .choices
            .iter()
            .filter(|c| c.to_lowercase().contains(&filter))
            .map(|c| Choice::Group(c.clone()));
        let none = (self.offer_none && filter.is_empty()).then_some(Choice::None);
        none.into_iter().chain(groups).collect()
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let count = self.visible().len();
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter => return self.choose(),
            KeyCode::Up => {
                self.selected = self.selected.and_then(|i| i.checked_sub(1));
                self.error = None;
            }
            KeyCode::Down => {
                match self.selected {
                    None if count > 0 => self.selected = Some(0),
                    Some(i) if i + 1 < count => self.selected = Some(i + 1),
                    _ => {}
                }
                self.error = None;
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.edited();
            }
            // Ctrl+Alt ist unter Windows AltGr (@, {, [ auf deutscher Tastatur).
            KeyCode::Char(c) if !ctrl || alt => {
                self.input.push(c);
                self.edited();
            }
            _ => {}
        }
        Outcome::Stay
    }

    pub fn paste(&mut self, text: &str) {
        self.input
            .push_str(text.lines().next().unwrap_or_default().trim());
        self.edited();
    }

    fn edited(&mut self) {
        self.selected = None;
        self.error = None;
    }

    fn choose(&mut self) -> Outcome {
        if let Some(choice) = self
            .selected
            .and_then(|i| self.visible().into_iter().nth(i))
        {
            return Outcome::Chosen(match choice {
                Choice::None => None,
                Choice::Group(name) => Some(name),
            });
        }
        let name = self.input.trim();
        if name.is_empty() {
            self.error = Some("type a name or pick an entry");
            return Outcome::Stay;
        }
        // „None“ steht für „keine Gruppe“; als Gruppenname wäre es mehrdeutig.
        if name.eq_ignore_ascii_case("none") {
            self.error = Some("\"None\" is reserved");
            return Outcome::Stay;
        }
        Outcome::Chosen(Some(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn picker(offer_none: bool) -> GroupPicker {
        GroupPicker::new(
            "Group".into(),
            vec!["id".into()],
            vec!["Backend".into(), "Frontend".into()],
            offer_none,
        )
    }

    fn type_text(picker: &mut GroupPicker, text: &str) {
        for c in text.chars() {
            picker.on_key(&key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typed_name_is_chosen() {
        let mut p = picker(false);
        type_text(&mut p, " Neu ");
        assert_eq!(
            p.on_key(&key(KeyCode::Enter)),
            Outcome::Chosen(Some("Neu".into()))
        );
    }

    #[test]
    fn arrow_selection_wins_over_input_and_typing_filters() {
        let mut p = picker(false);
        type_text(&mut p, "end");
        assert_eq!(p.visible().len(), 2);
        type_text(&mut p, "f");
        // „endf“ passt auf nichts mehr.
        assert!(p.visible().is_empty());
        p.on_key(&key(KeyCode::Backspace));
        p.on_key(&key(KeyCode::Backspace));
        p.on_key(&key(KeyCode::Backspace));
        p.on_key(&key(KeyCode::Backspace));
        type_text(&mut p, "front");
        p.on_key(&key(KeyCode::Down));
        assert_eq!(
            p.on_key(&key(KeyCode::Enter)),
            Outcome::Chosen(Some("Frontend".into()))
        );
    }

    #[test]
    fn none_entry_removes_and_only_when_offered() {
        let mut p = picker(true);
        assert_eq!(p.visible().first(), Some(&Choice::None));
        p.on_key(&key(KeyCode::Down));
        assert_eq!(p.on_key(&key(KeyCode::Enter)), Outcome::Chosen(None));
        assert!(!picker(false).visible().contains(&Choice::None));
    }

    #[test]
    fn typing_hides_none_so_arrow_picks_a_group() {
        let mut p = picker(true);
        type_text(&mut p, "front");
        assert!(!p.visible().contains(&Choice::None));
        p.on_key(&key(KeyCode::Down));
        assert_eq!(
            p.on_key(&key(KeyCode::Enter)),
            Outcome::Chosen(Some("Frontend".into()))
        );
    }

    #[test]
    fn empty_or_reserved_name_stays_open() {
        let mut p = picker(true);
        assert_eq!(p.on_key(&key(KeyCode::Enter)), Outcome::Stay);
        assert!(p.error.is_some());
        type_text(&mut p, "none");
        assert_eq!(p.on_key(&key(KeyCode::Enter)), Outcome::Stay);
        assert!(p.error.is_some());
        assert_eq!(p.on_key(&key(KeyCode::Esc)), Outcome::Cancel);
    }

    #[test]
    fn altgr_character_is_typed() {
        let mut p = picker(false);
        let altgr = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        p.on_key(&altgr);
        assert_eq!(p.input, "@");
    }
}
