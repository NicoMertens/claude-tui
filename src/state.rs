//! Fensterlayout über einen Neustart hinweg: welche Claude-Session in welchem Fenster lag und wo
//! der Fokus war, dazu die Schnellwahl-Nummern, die eigenen Gruppen und die ungelesenen Sessions.
//! Gespeichert als `state.json` im Konfigurationsordner.

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
    /// Claude-Session-IDs in Fensterreihenfolge.
    pub windows: Vec<String>,
    /// Fensterindex mit Fokus; `None` = Sessionliste.
    pub focus: Option<usize>,
    /// Sessionliste ausgeblendet (fehlt in älteren Dateien = eingeblendet).
    #[serde(default)]
    pub list_hidden: bool,
    /// Zwei Fenster übereinander statt nebeneinander.
    #[serde(default)]
    pub stacked: bool,
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
}

/// Fehlt die Datei oder ist sie unlesbar, gibt es nichts wiederherzustellen.
pub fn load() -> State {
    paths::config_dir()
        .and_then(|d| fs::read_to_string(d.join(FILE_NAME)).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Entfernt Claude-IDs, die nicht unter `known` sind; `true`, wenn sich etwas geändert hat.
pub fn forget_unknown<'a>(
    ids: &mut BTreeSet<String>,
    known: impl Iterator<Item = &'a str>,
) -> bool {
    let known: HashSet<&str> = known.collect();
    let before = ids.len();
    ids.retain(|id| known.contains(id.as_str()));
    ids.len() != before
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
            windows: vec!["a".into(), "b".into()],
            focus: Some(1),
            list_hidden: true,
            stacked: true,
            tags: Tags::from([(3, "b".to_string())]),
            groups: Assignments::from([("a".to_string(), "G".to_string())]),
            recent_groups: vec!["G".into()],
            superseded: BTreeMap::from([("alt".to_string(), "b".to_string())]),
            collapsed: BTreeSet::from(["dir:/x".to_string()]),
            unread: BTreeSet::from(["a".to_string()]),
        };
        let text = serde_json::to_string(&state).expect("json");
        assert_eq!(serde_json::from_str::<State>(&text).expect("json"), state);
    }

    #[test]
    fn forget_unknown_drops_deleted_sessions() {
        let mut ids = BTreeSet::from(["a".to_string(), "gone".to_string()]);
        assert!(forget_unknown(&mut ids, ["a", "b"].into_iter()));
        assert_eq!(ids, BTreeSet::from(["a".to_string()]));
        assert!(!forget_unknown(&mut ids, ["a"].into_iter()));
    }

    #[test]
    fn older_file_without_tags_still_loads() {
        let state: State = serde_json::from_str(r#"{"windows":["a"],"focus":0}"#).expect("json");
        assert!(state.tags.is_empty());
        assert!(state.groups.is_empty());
        assert!(state.recent_groups.is_empty());
        assert!(state.unread.is_empty());
        assert_eq!(state.windows, ["a"]);
    }
}
