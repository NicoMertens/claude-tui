//! Fensterlayout über einen Neustart hinweg: welche Claude-Session in welchem Fenster lag und wo
//! der Fokus war, dazu die Schnellwahl-Nummern und die eigenen Gruppen. Gespeichert als `state.json` im
//! Konfigurationsordner.

use std::fs;

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
}

/// Fehlt die Datei oder ist sie unlesbar, gibt es nichts wiederherzustellen.
pub fn load() -> State {
    paths::config_dir()
        .and_then(|d| fs::read_to_string(d.join(FILE_NAME)).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
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
        };
        let text = serde_json::to_string(&state).expect("json");
        assert_eq!(serde_json::from_str::<State>(&text).expect("json"), state);
    }

    #[test]
    fn older_file_without_tags_still_loads() {
        let state: State = serde_json::from_str(r#"{"windows":["a"],"focus":0}"#).expect("json");
        assert!(state.tags.is_empty());
        assert!(state.groups.is_empty());
        assert!(state.recent_groups.is_empty());
        assert_eq!(state.windows, ["a"]);
    }
}
