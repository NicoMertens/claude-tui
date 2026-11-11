//! Verzeichnisauswahl für eine neue Session: Eingabefeld mit Tab-Ergänzung, darunter die zuletzt
//! genutzten Verzeichnisse (↑/↓ übernimmt einen Eintrag ins Eingabefeld).

use std::{
    fs,
    path::{is_separator, Path, PathBuf, MAIN_SEPARATOR},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Wohin die Session nach der Auswahl kommt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Direkt in dieses Fenster (Ctrl+A n aus einem Fenster).
    Window(usize),
    /// Danach die Fensterauswahl (Ctrl+A n aus der Liste).
    PickWindow,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Cancel,
    Chosen(PathBuf),
}

#[derive(Debug)]
pub struct DirPicker {
    pub input: String,
    pub recent: Vec<PathBuf>,
    /// Mit ↑/↓ angesteuerter Eintrag aus „zuletzt“.
    pub selected: Option<usize>,
    pub error: Option<&'static str>,
    pub target: Target,
    /// Treffer der letzten Tab-Ergänzung, solange weiteres Tab durch sie blättert.
    cycle: Option<(Vec<String>, usize)>,
}

impl DirPicker {
    pub fn new(target: Target, recent: Vec<PathBuf>, default: &Path) -> Self {
        Self {
            input: default.display().to_string(),
            recent,
            selected: None,
            error: None,
            target,
            cycle: None,
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Enter => return self.choose(),
            KeyCode::Tab => {
                self.complete();
                return Outcome::Stay;
            }
            KeyCode::Up => self.select(self.selected.and_then(|i| i.checked_sub(1))),
            KeyCode::Down => match self.selected {
                None if !self.recent.is_empty() => self.select(Some(0)),
                Some(i) if i + 1 < self.recent.len() => self.select(Some(i + 1)),
                _ => {}
            },
            KeyCode::Backspace => {
                self.input.pop();
                self.edited();
            }
            // Ctrl+Alt ist unter Windows AltGr (\ auf deutscher Tastatur).
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

    fn select(&mut self, selected: Option<usize>) {
        self.selected = selected;
        if let Some(path) = selected.and_then(|i| self.recent.get(i)) {
            self.input = path.display().to_string();
        }
        self.error = None;
        self.cycle = None;
    }

    fn edited(&mut self) {
        self.selected = None;
        self.error = None;
        self.cycle = None;
    }

    fn choose(&mut self) -> Outcome {
        match resolve(&self.input) {
            Some(path) if path.is_dir() => Outcome::Chosen(path),
            _ => {
                self.error = Some("directory not found");
                Outcome::Stay
            }
        }
    }

    fn complete(&mut self) {
        if let Some((matches, index)) = self.cycle.as_mut() {
            *index = (*index + 1) % matches.len();
            self.input = matches[*index].clone();
            return;
        }
        let matches = completions(&self.input);
        match matches.as_slice() {
            [] => self.error = Some("no matching folder"),
            [only] => self.input = only.clone(),
            _ => {
                let common = common_prefix(&matches);
                if common.chars().count() > self.input.chars().count() {
                    self.input = common;
                } else {
                    self.input = matches[0].clone();
                    self.cycle = Some((matches, 0));
                }
            }
        }
    }
}

/// Eingabe → Pfad: Anführungszeichen weg, relativ zum aktuellen Verzeichnis, ohne Schluss-Trenner.
fn resolve(input: &str) -> Option<PathBuf> {
    let text = input.trim().trim_matches('"');
    if text.is_empty() {
        return None;
    }
    let path = PathBuf::from(text);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    Some(path.components().collect())
}

/// Unterordner des Elternpfads, deren Name mit dem letzten Pfadteil beginnt (ohne Beachtung der
/// Groß-/Kleinschreibung), jeweils mit Trenner am Ende, damit das nächste Tab weiter hineingeht.
fn completions(input: &str) -> Vec<String> {
    let Some(split) = input.rfind(is_separator) else {
        return Vec::new();
    };
    let (parent, prefix) = (&input[..=split], &input[split + 1..]);
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let prefix = prefix.to_lowercase();
    let mut matches: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.to_lowercase().starts_with(&prefix))
        .filter(|name| !name.starts_with('.') || prefix.starts_with('.'))
        .map(|name| format!("{parent}{name}{MAIN_SEPARATOR}"))
        .collect();
    matches.sort_by_key(|m| m.to_lowercase());
    matches
}

fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut len = first.len();
    for item in &items[1..] {
        len = first
            .char_indices()
            .zip(item.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8())
            .min(len);
    }
    first[..len].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Legt einen Testordner mit Unterordnern an.
    fn sandbox(name: &str, dirs: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("claude-tui-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("Testordner");
        for d in dirs {
            fs::create_dir_all(root.join(d)).expect("Testordner");
        }
        root
    }

    fn with_sep(path: &Path) -> String {
        format!("{}{MAIN_SEPARATOR}", path.display())
    }

    #[test]
    fn tab_completes_single_match() {
        let root = sandbox("single", &["alpha", "beta"]);
        let mut picker = DirPicker::new(Target::PickWindow, Vec::new(), &root);
        picker.input = format!("{}al", with_sep(&root));
        picker.on_key(&key(KeyCode::Tab));
        assert_eq!(picker.input, with_sep(&root.join("alpha")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tab_extends_common_prefix_then_cycles() {
        let root = sandbox("cycle", &["projekt-a", "projekt-b"]);
        let mut picker = DirPicker::new(Target::PickWindow, Vec::new(), &root);
        picker.input = format!("{}pro", with_sep(&root));
        picker.on_key(&key(KeyCode::Tab));
        assert_eq!(picker.input, format!("{}projekt-", with_sep(&root)));
        picker.on_key(&key(KeyCode::Tab));
        assert_eq!(picker.input, with_sep(&root.join("projekt-a")));
        picker.on_key(&key(KeyCode::Tab));
        assert_eq!(picker.input, with_sep(&root.join("projekt-b")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn arrows_copy_recent_entry_into_input() {
        let recent = vec![PathBuf::from("eins"), PathBuf::from("zwei")];
        let mut picker = DirPicker::new(Target::PickWindow, recent, Path::new("start"));
        picker.on_key(&key(KeyCode::Down));
        picker.on_key(&key(KeyCode::Down));
        assert_eq!(picker.input, "zwei");
        picker.on_key(&key(KeyCode::Up));
        assert_eq!(picker.input, "eins");
        assert_eq!(picker.selected, Some(0));
    }

    #[test]
    fn enter_rejects_missing_directory() {
        let mut picker = DirPicker::new(
            Target::PickWindow,
            Vec::new(),
            Path::new("gibt-es-sicher-nicht-42"),
        );
        assert_eq!(picker.on_key(&key(KeyCode::Enter)), Outcome::Stay);
        assert!(picker.error.is_some());
    }

    #[test]
    fn enter_accepts_existing_directory_without_trailing_separator() {
        let root = sandbox("enter", &[]);
        let mut picker = DirPicker::new(Target::PickWindow, Vec::new(), &root);
        picker.input = with_sep(&root);
        assert_eq!(
            picker.on_key(&key(KeyCode::Enter)),
            Outcome::Chosen(root.clone())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn altgr_backslash_is_typed() {
        let mut picker = DirPicker::new(Target::PickWindow, Vec::new(), Path::new("S:"));
        let altgr = KeyEvent::new(
            KeyCode::Char('\\'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        picker.on_key(&altgr);
        assert_eq!(picker.input, "S:\\");
    }
}
