//! Fensterlayout über einen Neustart hinweg: welche Claude-Session in welchem Fenster lag und wo
//! der Fokus war. Gespeichert als `state.json` im Konfigurationsordner.

use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

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
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let file = dir.join(FILE_NAME);
    let text = serde_json::to_string_pretty(state)?;
    fs::write(&file, text).with_context(|| format!("cannot write {}", file.display()))
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
        };
        let text = serde_json::to_string(&state).expect("json");
        assert_eq!(serde_json::from_str::<State>(&text).expect("json"), state);
    }
}
