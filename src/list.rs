//! Sessionliste: gespeicherte Claude-Sessions und laufende Prozesse, oben eigene Gruppen
//! (alphabetisch), darunter nach Verzeichnis (ebenfalls alphabetisch).

use std::{
    collections::{hash_map::Entry as MapEntry, BTreeSet, HashMap, HashSet},
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

impl Heading {
    /// Dauerhafter Schlüssel (eingeklappte Abschnitte in `state.json`, Cursor auf der Überschrift).
    pub fn key(&self) -> String {
        match self {
            Self::Group(name) => format!("group:{name}"),
            Self::Dir(path) => format!("dir:{}", dir_key(path).display()),
        }
    }
}

/// Position des Cursors in der Liste: auf einer Session, einer Überschrift oder dem Notizblock
/// (eigene Zeile ganz oben, gehört zu keinem Abschnitt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// Claude-ID der Session.
    Session(String),
    /// Schlüssel der Überschrift (siehe [`Heading::key`]).
    Heading(String),
    Notepad,
}

/// Eine sichtbare Zeile der Liste.
#[derive(Clone, Copy, Debug)]
pub enum Row<'a> {
    Heading(&'a Group),
    Entry(&'a Entry),
}

impl Row<'_> {
    pub fn cursor(&self) -> Cursor {
        match self {
            Self::Heading(group) => Cursor::Heading(group.heading.key()),
            Self::Entry(entry) => Cursor::Session(entry.claude_id.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub heading: Heading,
    pub entries: Vec<Entry>,
}

pub const NEW_SESSION_TITLE: &str = "new session";

/// Baut die Abschnitte. Reihenfolge: eigene Gruppen (`assigned`) alphabetisch, danach die
/// Verzeichnisse alphabetisch; darin laufende Sessions zuerst (in Startreihenfolge), dann nach
/// letzter Aktivität. `external`: Claude-IDs, die außerhalb von claude-tui laufen.
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
        group.entries.sort_by(order);
    }
    groups.sort_by(|a, b| match (&a.heading, &b.heading) {
        (Heading::Group(x), Heading::Group(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Heading::Group(_), Heading::Dir(_)) => std::cmp::Ordering::Less,
        (Heading::Dir(_), Heading::Group(_)) => std::cmp::Ordering::Greater,
        (Heading::Dir(x), Heading::Dir(y)) => dir_key(x).cmp(&dir_key(y)),
    });
    groups
}

/// Reihenfolge im Abschnitt: laufende zuerst, untereinander in Startreihenfolge (stabil, sonst
/// springen sie bei jeder Ausgabe); danach gespeicherte, jüngste Aktivität zuerst.
fn order(a: &Entry, b: &Entry) -> std::cmp::Ordering {
    match (a.running, b.running) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.modified.cmp(&a.modified),
    }
}

/// Vergleichsschlüssel eines Verzeichnisses; unter Windows ohne Groß-/Kleinschreibung.
fn dir_key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// Sichtbare Zeilen: jede Überschrift, darunter ihre Sessions, sofern der Abschnitt nicht
/// eingeklappt ist (`collapsed`: Schlüssel nach [`Heading::key`]).
pub fn rows<'a>(groups: &'a [Group], collapsed: &BTreeSet<String>) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    for group in groups {
        rows.push(Row::Heading(group));
        if !collapsed.contains(&group.heading.key()) {
            rows.extend(group.entries.iter().map(Row::Entry));
        }
    }
    rows
}

/// Abschnitt, in dem die Session steht.
pub fn group_with<'a>(groups: &'a [Group], claude_id: &str) -> Option<&'a Group> {
    groups
        .iter()
        .find(|g| g.entries.iter().any(|e| e.claude_id == claude_id))
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
            version: None,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs),
        }
    }

    fn dir(path: &str) -> Heading {
        Heading::Dir(PathBuf::from(path))
    }

    #[test]
    fn directories_alphabetical_sessions_newest_first() {
        let groups = build(
            vec![
                stored("a", "/y", 50),
                stored("b", "/x", 100),
                stored("c", "/y", 5),
            ],
            &[],
            &HashSet::new(),
            &Assignments::new(),
        );
        let dirs: Vec<_> = groups.iter().map(|g| g.heading.clone()).collect();
        assert_eq!(dirs, [dir("/x"), dir("/y")]);
        let ids: Vec<_> = flatten(&groups).map(|e| e.claude_id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }

    #[test]
    fn running_sessions_keep_start_order_regardless_of_activity() {
        let running = [
            Running {
                id: 1,
                claude_id: "alt".into(),
                cwd: "/x".into(),
            },
            Running {
                id: 2,
                claude_id: "jung".into(),
                cwd: "/x".into(),
            },
        ];
        let ids = |alt_age, jung_age| {
            let groups = build(
                vec![
                    stored("alt", "/x", alt_age),
                    stored("jung", "/x", jung_age),
                    stored("ruht", "/x", 0),
                ],
                &running,
                &HashSet::new(),
                &Assignments::new(),
            );
            flatten(&groups)
                .map(|e| e.claude_id.clone())
                .collect::<Vec<_>>()
        };
        // Wer zuletzt geschrieben hat, ändert nichts an der Reihenfolge der laufenden.
        assert_eq!(ids(5, 50), ["alt", "jung", "ruht"]);
        assert_eq!(ids(50, 5), ["alt", "jung", "ruht"]);
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
        // Verzeichnisse alphabetisch, auch wenn darin Sessions laufen.
        let dirs: Vec<_> = groups.iter().map(|g| g.heading.clone()).collect();
        assert_eq!(dirs, [dir("/x"), dir("/y"), dir("/z")]);
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
    fn collapsed_sections_show_only_their_heading() {
        let groups = build(
            vec![stored("a", "/x", 50), stored("b", "/y", 10)],
            &[],
            &HashSet::new(),
            &Assignments::new(),
        );
        let collapsed = BTreeSet::from([dir("/y").key()]);
        let cursors: Vec<_> = rows(&groups, &collapsed).iter().map(Row::cursor).collect();
        assert_eq!(
            cursors,
            [
                Cursor::Heading(dir("/x").key()),
                Cursor::Session("a".into()),
                Cursor::Heading(dir("/y").key()),
            ]
        );
        assert_eq!(
            group_with(&groups, "b").map(|g| &g.heading),
            Some(&dir("/y"))
        );
    }

    #[test]
    fn heading_keys_separate_groups_from_directories() {
        assert_ne!(Heading::Group("x".into()).key(), dir("x").key());
        if cfg!(windows) {
            assert_eq!(dir(r"C:\Dev").key(), dir(r"c:\dev").key());
        }
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
