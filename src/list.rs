//! Sessionliste: gespeicherte Claude-Sessions und laufende Prozesse, gruppiert nach Verzeichnis.

use std::{path::PathBuf, time::SystemTime};

use crate::{claude_store::StoredSession, windows::SessionId};

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
    /// Letzte Aktivität laut Protokoll; `None` für eine neue Session ohne Protokoll.
    pub modified: Option<SystemTime>,
    /// Laufender Prozess, falls die Session gerade in claude-tui läuft.
    pub running: Option<SessionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
}

pub const NEW_SESSION_TITLE: &str = "neue Session";

/// Baut die Gruppen. Reihenfolge: Gruppen nach jüngster Aktivität (laufend zählt als jetzt),
/// darin laufende Sessions zuerst, dann nach letzter Aktivität.
pub fn build(stored: Vec<StoredSession>, running: &[Running]) -> Vec<Group> {
    let mut entries: Vec<(PathBuf, Entry)> = stored
        .into_iter()
        .map(|s| {
            let running = running.iter().find(|r| r.claude_id == s.id).map(|r| r.id);
            let entry = Entry {
                claude_id: s.id,
                title: s.title,
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
                    modified: None,
                    running: Some(r.id),
                },
            ));
        }
    }

    let mut groups: Vec<Group> = Vec::new();
    for (cwd, entry) in entries {
        match groups.iter_mut().find(|g| same_dir(&g.cwd, &cwd)) {
            Some(group) => group.entries.push(entry),
            None => groups.push(Group {
                cwd,
                entries: vec![entry],
            }),
        }
    }
    for group in &mut groups {
        group.entries.sort_by_key(|e| std::cmp::Reverse(rank(e)));
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.entries.first().map(rank)));
    groups
}

/// Sortierschlüssel: laufend vor gespeichert, dann jüngste Aktivität zuerst.
fn rank(entry: &Entry) -> (bool, Option<SystemTime>) {
    (entry.running.is_some(), entry.modified)
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// Alle Einträge in Anzeigereihenfolge (ohne Gruppenüberschriften).
pub fn flatten(groups: &[Group]) -> impl Iterator<Item = &Entry> {
    groups.iter().flat_map(|g| g.entries.iter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn stored(id: &str, cwd: &str, age_secs: u64) -> StoredSession {
        StoredSession {
            id: id.to_string(),
            cwd: PathBuf::from(cwd),
            title: id.to_uppercase(),
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age_secs),
        }
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
        );
        let dirs: Vec<_> = groups.iter().map(|g| g.cwd.clone()).collect();
        assert_eq!(dirs, [PathBuf::from("/x"), PathBuf::from("/y")]);
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
        let groups = build(vec![stored("a", "/x", 500), stored("b", "/y", 1)], &running);
        // Gruppen mit laufenden Sessions stehen vor rein gespeicherten.
        assert_eq!(
            groups.last().map(|g| g.cwd.clone()),
            Some(PathBuf::from("/y"))
        );
        let neu = flatten(&groups)
            .find(|e| e.claude_id == "neu")
            .expect("neu");
        assert_eq!(neu.title, NEW_SESSION_TITLE);
        assert_eq!(neu.running, Some(2));
        let a = flatten(&groups).find(|e| e.claude_id == "a").expect("a");
        assert_eq!(a.running, Some(1));
    }
}
