//! Fensterlayout über einen Neustart hinweg: welche Claude-Session in welchem Fenster lag und wo
//! der Fokus war, dazu die Schnellwahl-Nummern, die eigenen Gruppen, die ungelesenen Sessions und
//! die Notizen.
//! Gespeichert als `state.json` im Konfigurationsordner. Einträge zu Sessions, deren Protokoll es
//! nicht mehr gibt, fallen weg ([`PerSession::forget_unknown`]).

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{groups::Assignments, paths, tags::Tags};

const FILE_NAME: &str = "state.json";

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Claude-Session-ID je Fensterplatz, `""` = Fenster nicht offen. Bewusst kein `null`: ältere
    /// Versionen lesen die Datei sonst nicht mehr und verlieren beim nächsten Speichern alles.
    pub windows: Vec<String>,
    /// Fensterindex mit Fokus; `None` = Sessionliste.
    pub focus: Option<usize>,
    /// Sessionliste ausgeblendet (fehlt in älteren Dateien = eingeblendet).
    #[serde(default)]
    pub list_hidden: bool,
    /// Schnellwahl-Nummer → Claude-Session-ID (Alt+Zahl / Ctrl+Zahl).
    #[serde(default)]
    pub tags: Tags,
    /// Claude-Session-ID → eigene Gruppe.
    #[serde(default)]
    pub groups: Assignments,
    /// Zuletzt verwendete Gruppennamen, neueste zuerst.
    #[serde(default)]
    pub recent_groups: Vec<String>,
    /// Von Claude Code als Hintergrund-Job weitergeführte Sessions: alte Claude-ID → neue. Die
    /// alte wird in der Liste nicht mehr gezeigt.
    #[serde(default)]
    pub superseded: BTreeMap<String, String>,
    /// Eingeklappte Abschnitte der Sessionliste (Schlüssel nach `list::Heading::key`).
    #[serde(default)]
    pub collapsed: BTreeSet<String>,
    /// Sessions mit ungelesener fertiger Antwort (Claude-IDs).
    #[serde(default)]
    pub unread: BTreeSet<String>,
    /// Claude-Session-ID → Notiz (Ctrl+A t).
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
    /// Fensterindex des Notizblocks (sein Platz in `windows` bleibt `""`).
    #[serde(default)]
    pub notepad_window: Option<usize>,
    /// Fensterindex der Todo-Liste (ihr Platz in `windows` bleibt `""`).
    #[serde(default)]
    pub todo_window: Option<usize>,
    /// Fensterindex der Notizübersicht (ihr Platz in `windows` bleibt `""`).
    #[serde(default)]
    pub notes_window: Option<usize>,
}

/// Fehlt die Datei oder ist sie unlesbar, gibt es nichts wiederherzustellen.
pub fn load() -> State {
    paths::config_dir()
        .and_then(|d| fs::read_to_string(d.join(FILE_NAME)).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Was an einzelnen Claude-Sessions hängt: Schnellwahl-Nummern, Gruppen, ungelesene Antworten,
/// Notizen.
pub struct PerSession<'a> {
    pub tags: &'a mut Tags,
    pub groups: &'a mut Assignments,
    pub unread: &'a mut BTreeSet<String>,
    pub notes: &'a mut BTreeMap<String, String>,
}

impl PerSession<'_> {
    /// Entfernt Einträge zu Claude-IDs, die nicht unter `known` sind; `true`, wenn sich etwas
    /// geändert hat.
    pub fn forget_unknown<'k>(self, known: impl Iterator<Item = &'k str>) -> bool {
        let known: HashSet<&str> = known.collect();
        let counts = |s: &Self| (s.tags.len(), s.groups.len(), s.unread.len(), s.notes.len());
        let before = counts(&self);
        self.tags.retain(|_, id| known.contains(id.as_str()));
        self.groups.retain(|id, _| known.contains(id.as_str()));
        self.unread.retain(|id| known.contains(id.as_str()));
        self.notes.retain(|id, _| known.contains(id.as_str()));
        counts(&self) != before
    }
}

pub fn save(state: &State) -> Result<()> {
    let dir = paths::config_dir().context("no config directory (APPDATA/HOME) found")?;
    let file = dir.join(FILE_NAME);
    let text = serde_json::to_string_pretty(state)?;
    paths::write_atomic(&file, text).with_context(|| format!("cannot write {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let state = State {
            windows: vec!["".into(), "a".into(), "".into(), "b".into()],
            focus: Some(1),
            list_hidden: true,
            tags: Tags::from([(3, "b".to_string())]),
            groups: Assignments::from([("a".to_string(), "G".to_string())]),
            recent_groups: vec!["G".into()],
            superseded: BTreeMap::from([("alt".to_string(), "b".to_string())]),
            collapsed: BTreeSet::from(["dir:/x".to_string()]),
            unread: BTreeSet::from(["a".to_string()]),
            notes: BTreeMap::from([("b".to_string(), "x\ny".to_string())]),
            notepad_window: Some(0),
            todo_window: Some(2),
            notes_window: Some(3),
        };
        let text = serde_json::to_string(&state).expect("json");
        assert_eq!(serde_json::from_str::<State>(&text).expect("json"), state);
    }

    #[test]
    fn forget_unknown_drops_deleted_sessions() {
        let mut tags = Tags::from([(1, "a".to_string()), (2, "gone".to_string())]);
        let mut groups = Assignments::from([
            ("a".to_string(), "G".to_string()),
            ("gone".to_string(), "G".to_string()),
        ]);
        let mut unread = BTreeSet::from(["a".to_string(), "gone".to_string()]);
        let mut notes = BTreeMap::from([("gone".to_string(), "n".to_string())]);
        let mut forget = |known: &[&str]| {
            PerSession {
                tags: &mut tags,
                groups: &mut groups,
                unread: &mut unread,
                notes: &mut notes,
            }
            .forget_unknown(known.iter().copied())
        };
        assert!(forget(&["a", "b"]));
        assert!(!forget(&["a"]));
        assert_eq!(tags, Tags::from([(1, "a".to_string())]));
        assert_eq!(
            groups,
            Assignments::from([("a".to_string(), "G".to_string())])
        );
        assert_eq!(unread, BTreeSet::from(["a".to_string()]));
        assert!(notes.is_empty());
    }

    #[test]
    fn older_file_with_stacked_windows_still_loads() {
        let text = r#"{"windows":["a","b"],"focus":1,"stacked":true}"#;
        let state: State = serde_json::from_str(text).expect("json");
        assert_eq!(state.windows, ["a", "b"]);
        assert_eq!(state.focus, Some(1));
    }

    #[test]
    fn older_file_without_tags_still_loads() {
        let state: State = serde_json::from_str(r#"{"windows":["a"],"focus":0}"#).expect("json");
        assert!(state.tags.is_empty());
        assert!(state.groups.is_empty());
        assert!(state.recent_groups.is_empty());
        assert!(state.unread.is_empty());
        assert!(state.notes.is_empty());
        assert_eq!(state.windows, ["a"]);
    }
}
