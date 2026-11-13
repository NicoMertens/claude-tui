//! Eigene Gruppen für Sessions: eine Session gehört höchstens einer Gruppe. Gruppierte Sessions
//! stehen in der Sessionliste über den Verzeichnissen, die Gruppen alphabetisch.

use std::collections::BTreeMap;

/// Claude-Session-ID → Gruppenname.
pub type Assignments = BTreeMap<String, String>;

/// So viele zuletzt verwendete Gruppen bleiben gespeichert.
const MAX_RECENT: usize = 10;

/// Bereinigter Name (ohne Rand-Leerzeichen); leer = kein Name. Gibt es die Gruppe schon in anderer
/// Schreibweise, gilt deren Schreibweise — „Foo“ und „foo“ sollen nie zwei Gruppen werden.
pub fn normalize<'a>(name: &str, known: impl IntoIterator<Item = &'a String>) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let existing = known
        .into_iter()
        .find(|k| k.to_lowercase() == name.to_lowercase());
    Some(existing.map_or_else(|| name.to_string(), Clone::clone))
}

/// Ordnet die Sessions der Gruppe zu (`None` = aus ihrer Gruppe entfernen).
pub fn assign(assignments: &mut Assignments, claude_ids: &[String], group: Option<&str>) {
    for id in claude_ids {
        match group {
            Some(name) => assignments.insert(id.clone(), name.to_string()),
            None => assignments.remove(id),
        };
    }
}

/// Alle Gruppen, denen gerade eine Session angehört, alphabetisch ohne Groß-/Kleinschreibung.
pub fn existing(assignments: &Assignments) -> Vec<String> {
    let mut names: Vec<String> = assignments.values().cloned().collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// Stellt `name` an den Anfang der zuletzt verwendeten, ohne Doppelte, gekürzt auf die Höchstzahl.
pub fn remember(recent: &mut Vec<String>, name: &str) {
    recent.retain(|r| r.to_lowercase() != name.to_lowercase());
    recent.insert(0, name.to_string());
    recent.truncate(MAX_RECENT);
}

/// Auswahl für den Gruppen-Dialog: zuletzt verwendete zuerst, danach die übrigen vorhandenen
/// alphabetisch.
pub fn choices(assignments: &Assignments, recent: &[String]) -> Vec<String> {
    let mut list: Vec<String> = recent.to_vec();
    for name in existing(assignments) {
        if !list.iter().any(|r| r.to_lowercase() == name.to_lowercase()) {
            list.push(name);
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalize_trims_and_reuses_existing_spelling() {
        let known = ids(&["Backend"]);
        assert_eq!(normalize("  backend ", &known).as_deref(), Some("Backend"));
        assert_eq!(normalize("Neu", &known).as_deref(), Some("Neu"));
        assert_eq!(normalize("   ", &known), None);
    }

    #[test]
    fn assign_and_remove() {
        let mut a = Assignments::new();
        assign(&mut a, &ids(&["x", "y"]), Some("G"));
        assert_eq!(a.get("x").map(String::as_str), Some("G"));
        assign(&mut a, &ids(&["x"]), None);
        assert!(!a.contains_key("x"));
        assert_eq!(existing(&a), ["G"]);
    }

    #[test]
    fn remember_moves_to_front_case_insensitive() {
        let mut recent = ids(&["a", "B", "c"]);
        remember(&mut recent, "b");
        assert_eq!(recent, ["b", "a", "c"]);
    }

    #[test]
    fn choices_put_recent_first_then_rest_alphabetical() {
        let mut a = Assignments::new();
        assign(&mut a, &ids(&["1"]), Some("zeta"));
        assign(&mut a, &ids(&["2"]), Some("Alpha"));
        assign(&mut a, &ids(&["3"]), Some("mid"));
        let recent = ids(&["mid", "alt"]);
        assert_eq!(choices(&a, &recent), ["mid", "alt", "Alpha", "zeta"]);
    }
}
