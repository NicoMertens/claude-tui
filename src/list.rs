//! Sessionliste: gespeicherte Claude-Sessions und laufende Prozesse, oben eigene Gruppen
//! (alphabetisch), darunter nach Verzeichnis.

use std::{
    collections::{hash_map::Entry as MapEntry, HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{claude_store::StoredSession, groups::Assignments, windows::SessionId};

/// Ein laufender Claude-Prozess in claude-tui.
#[derive(Clone, Debug)]
pub struct Running {
    pub id: SessionId,
    pub claude_id: String,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub claude_id: String,
    pub title: String,
    /// Sessionfarbe aus `/color`.
    pub color: Option<String>,
    /// Letzte Aktivität laut Protokoll; `None` für eine neue Session ohne Protokoll.
    pub modified: Option<SystemTime>,
    /// Laufender Prozess, falls die Session gerade in claude-tui läuft.
    pub running: Option<SessionId>,
    /// Läuft außerhalb von claude-tui (siehe `external.rs`); Öffnen fragt erst nach.
    pub external: bool,
}

/// Überschrift eines Listenabschnitts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Heading {
    /// Eigene Gruppe (siehe `groups.rs`).
    Group(String),
    /// Arbeitsverzeichnis der nicht gruppierten Sessions.
    Dir(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub heading: Heading,
    pub entries: Vec<Entry>,
}

pub const NEW_SESSION_TITLE: &str = "new session";

/// Baut die Abschnitte. Reihenfolge: eigene Gruppen (`assigned`) alphabetisch, danach die
/// Verzeichnisse nach jüngster Aktivität (laufend zählt als jetzt); darin laufende Sessions
/// zuerst, dann nach letzter Aktivität. `external`: Claude-IDs, die außerhalb von claude-tui
/// laufen.
pub fn build(
    stored: Vec<StoredSession>,
    running: &[Running],
    external: &HashSet<String>,
    assigned: &Assignments,
) -> Vec<Group> {
    let mut entries: Vec<(PathBuf, Entry)> = stored
        .into_iter()
        .map(|s| {
            let running = running.iter().find(|r| r.claude_id == s.id).map(|r| r.id);
            let entry = Entry {
                external: running.is_none() && external.contains(&s.id),
                claude_id: s.id,
                title: s.title,
                color: s.color,
                modified: Some(s.modified),
                running,
            };
            (s.cwd, entry)
        })
        .collect();
    for r in running {
        if !entries.iter().any(|(_, e)| e.claude_id == r.claude_id) {
            entries.push((
                r.cwd.clone(),
                Entry {
                    claude_id: r.claude_id.clone(),
                    title: NEW_SESSION_TITLE.to_string(),
                    color: None,
                    modified: None,
                    running: Some(r.id),
                    external: false,
                },
            ));
        }
    }

    // Schlüssel je Verzeichnis einmal berechnen: die Liste wird oft neu gebaut, und unter Windows
    // kostete jeder Vergleich zwei kleingeschriebene Kopien. Gruppennamen sind schon bereinigt
    // (eine Schreibweise je Gruppe), Verzeichnisse und Gruppen haben getrennte Schlüssel.
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<Heading, usize> = HashMap::new();
    for (cwd, entry) in entries {
        let (key, heading) = match assigned.get(&entry.claude_id) {
            Some(name) => (Heading::Group(name.clone()), Heading::Group(name.clone())),
            None => (Heading::Dir(dir_key(&cwd)), Heading::Dir(cwd)),
        };
        match index.entry(key) {
            MapEntry::Occupied(slot) => groups[*slot.get()].entries.push(entry),
            MapEntry::Vacant(slot) => {
                slot.insert(groups.len());
                groups.push(Group {
                    heading,
                    entries: vec![entry],
                });
            }
        }
    }
    for group in &mut groups {
        group.entries.sort_by_key(|e| std::cmp::Reverse(rank(e)));
    }
    groups.sort_by(|a, b| match (&a.heading, &b.heading) {
        (Heading::Group(x), Heading::Group(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Heading::Group(_), Heading::Dir(_)) => std::cmp::Ordering::Less,
        (Heading::Dir(_), Heading::Group(_)) => std::cmp::Ordering::Greater,
        (Heading::Dir(_), Heading::Dir(_)) => {
            let newest = |g: &Group| g.entries.first().map(rank);
            newest(b).cmp(&newest(a))
        }
    });
    groups
}

/// Sortierschlüssel: laufend vor gespeichert, dann jüngste Aktivität zuerst.
fn rank(entry: &Entry) -> (bool, Option<SystemTime>) {
    (entry.running.is_some(), entry.modified)
}

/// Vergleichsschlüssel eines Verzeichnisses; unter Windows ohne Groß-/Kleinschreibung.
fn dir_key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// Alle Einträge in Anzeigereihenfolge (ohne Gruppenüberschriften).
pub fn flatten(groups: &[Group]) -> impl Iterator<Item = &Entry> {
    groups.iter().flat_map(|g| g.entries.iter())
}

/// Nachfolger für einen verschwundenen Eintrag `gone`: der nächste noch vorhandene Eintrag
/// darunter in der alten Reihenfolge `before`, sonst der nächste darüber.
pub fn successor<'a>(before: &[&'a str], gone: &str, still: &HashSet<&str>) -> Option<&'a str> {
    let index = before.iter().position(|&id| id == gone)?;
    let (above, below) = (&before[..index], &before[index + 1..]);
    below
        .iter()
        .chain(above.iter().rev())
        .find(|id| still.contains(*id))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn successor_prefers_entry_below_then_above() {
        let before = ["a", "b", "c", "d"];
        let still: HashSet<&str> = ["a", "b", "d"].into();
        assert_eq!(successor(&before, "c", &still), Some("d"));
        let still: HashSet<&str> = ["a", "b"].into();
        assert_eq!(successor(&before, "c", &still), Some("b"));
        assert_eq!(successor(&before, "x", &still), None);
        assert_eq!(successor(&before, "c", &HashSet::new()), None);
    }

    fn stored(id: &str, cwd: &str, age_secs: u64) -> StoredSession {
        StoredSession {
            id: id.to_string(),
            cwd: PathBuf::from(cwd),
            title: id.to_uppercase(),
            color: None,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs),
        }
    }

    fn dir(path: &str) -> Heading {
        Heading::Dir(PathBuf::from(path))
    }

    #[test]
    fn groups_by_directory_newest_first() {
        let groups = build(
            vec![
                stored("a", "/x", 50),
                stored("b", "/y", 10),
                stored("c", "/x", 5),
            ],
            &[],
            &HashSet::new(),
            &Assignments::new(),
        );
        let dirs: Vec<_> = groups.iter().map(|g| g.heading.clone()).collect();
        assert_eq!(dirs, [dir("/x"), dir("/y")]);
        let ids: Vec<_> = flatten(&groups).map(|e| e.claude_id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b"]);
    }

    #[test]
    fn running_sessions_come_first_and_new_ones_appear() {
        let running = [
            Running {
                id: 1,
                claude_id: "a".into(),
                cwd: "/x".into(),
            },
            Running {
                id: 2,
                claude_id: "neu".into(),
                cwd: "/z".into(),
            },
        ];
        let external = HashSet::from(["a".to_string(), "b".to_string()]);
        let groups = build(
            vec![stored("a", "/x", 500), stored("b", "/y", 1)],
            &running,
            &external,
            &Assignments::new(),
        );
        // Gruppen mit laufenden Sessions stehen vor rein gespeicherten.
        assert_eq!(groups.last().map(|g| g.heading.clone()), Some(dir("/y")));
        let neu = flatten(&groups)
            .find(|e| e.claude_id == "neu")
            .expect("neu");
        assert_eq!(neu.title, NEW_SESSION_TITLE);
        assert_eq!(neu.running, Some(2));
        let a = flatten(&groups).find(|e| e.claude_id == "a").expect("a");
        assert_eq!(a.running, Some(1));
        assert!(!a.external, "läuft in claude-tui, also nicht extern");
        let b = flatten(&groups).find(|e| e.claude_id == "b").expect("b");
        assert!(b.external);
    }

    #[test]
    fn own_groups_come_first_alphabetically_and_leave_their_directory() {
        let assigned = Assignments::from([
            ("b".to_string(), "zeta".to_string()),
            ("c".to_string(), "Alpha".to_string()),
            ("d".to_string(), "Alpha".to_string()),
        ]);
        let groups = build(
            vec![
                stored("a", "/x", 50),
                stored("b", "/x", 1),
                stored("c", "/y", 30),
                stored("d", "/z", 10),
            ],
            &[],
            &HashSet::new(),
            &assigned,
        );
        let headings: Vec<_> = groups.iter().map(|g| g.heading.clone()).collect();
        // /y und /z sind leer geworden und verschwinden; /x behält nur a.
        assert_eq!(
            headings,
            [
                Heading::Group("Alpha".into()),
                Heading::Group("zeta".into()),
                dir("/x")
            ]
        );
        let ids: Vec<_> = flatten(&groups).map(|e| e.claude_id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "b", "a"]);
    }

    #[test]
    fn new_running_session_can_be_grouped() {
        let running = [Running {
            id: 7,
            claude_id: "neu".into(),
            cwd: "/x".into(),
        }];
        let assigned = Assignments::from([("neu".to_string(), "G".to_string())]);
        let groups = build(Vec::new(), &running, &HashSet::new(), &assigned);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].heading, Heading::Group("G".into()));
    }
}
