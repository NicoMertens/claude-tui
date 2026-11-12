//! Schnellwahl-Nummern 1–9 für Sessions: `Alt+Zahl` vergibt bzw. entfernt sie, `Ctrl+Zahl` zeigt
//! die Session. Jede Nummer gehört höchstens einer Session, jede Session hat höchstens eine Nummer.

use std::collections::BTreeMap;

/// Nummer → Claude-Session-ID.
pub type Tags = BTreeMap<u8, String>;

/// Was [`toggle`] getan hat (für den Hinweis unten in der Liste).
#[derive(Debug, PartialEq, Eq)]
pub enum Toggled {
    Set,
    Removed,
}

/// Hat die Session schon diese Nummer, wird sie entfernt. Sonst bekommt sie die Nummer — eine
/// andere Session verliert sie dabei, die bisherige Nummer der Session entfällt.
pub fn toggle(tags: &mut Tags, number: u8, claude_id: &str) -> Toggled {
    if tags.get(&number).is_some_and(|id| id == claude_id) {
        tags.remove(&number);
        return Toggled::Removed;
    }
    tags.retain(|_, id| id != claude_id);
    tags.insert(number, claude_id.to_string());
    Toggled::Set
}

pub fn number_of(tags: &Tags, claude_id: &str) -> Option<u8> {
    tags.iter()
        .find(|(_, id)| *id == claude_id)
        .map(|(&n, _)| n)
}

/// Gibt zurück, ob die Session eine Nummer hatte.
pub fn remove_session(tags: &mut Tags, claude_id: &str) -> bool {
    let before = tags.len();
    tags.retain(|_, id| id != claude_id);
    tags.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_number_twice_removes_tag() {
        let mut tags = Tags::new();
        assert_eq!(toggle(&mut tags, 3, "a"), Toggled::Set);
        assert_eq!(number_of(&tags, "a"), Some(3));
        assert_eq!(toggle(&mut tags, 3, "a"), Toggled::Removed);
        assert!(tags.is_empty());
    }

    #[test]
    fn number_moves_to_other_session() {
        let mut tags = Tags::new();
        toggle(&mut tags, 1, "a");
        toggle(&mut tags, 1, "b");
        assert_eq!(number_of(&tags, "a"), None);
        assert_eq!(tags.get(&1).map(String::as_str), Some("b"));
    }

    #[test]
    fn session_keeps_only_its_newest_number() {
        let mut tags = Tags::new();
        toggle(&mut tags, 1, "a");
        toggle(&mut tags, 2, "a");
        assert_eq!(tags.len(), 1);
        assert_eq!(number_of(&tags, "a"), Some(2));
    }

    #[test]
    fn removing_session_drops_its_tag() {
        let mut tags = Tags::new();
        toggle(&mut tags, 1, "a");
        toggle(&mut tags, 2, "b");
        assert!(remove_session(&mut tags, "a"));
        assert!(!remove_session(&mut tags, "a"));
        assert_eq!(number_of(&tags, "a"), None);
        assert_eq!(number_of(&tags, "b"), Some(2));
    }
}
