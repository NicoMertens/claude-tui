//! Zuletzt genutzte Startverzeichnisse, gespeichert als Textdatei (ein Pfad je Zeile) im
//! Konfigurationsordner: `%APPDATA%\claude-tui` bzw. `$XDG_CONFIG_HOME/claude-tui` (`~/.config`).

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::paths;

const MAX_ENTRIES: usize = 10;
const FILE_NAME: &str = "recent-dirs.txt";

fn file() -> Option<PathBuf> {
    paths::config_dir().map(|d| d.join(FILE_NAME))
}

/// Liest die Liste; fehlt die Datei, ist sie leer.
pub fn load() -> Vec<PathBuf> {
    file()
        .and_then(|f| fs::read_to_string(f).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

fn parse(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .take(MAX_ENTRIES)
        .collect()
}

pub fn save(list: &[PathBuf]) -> Result<()> {
    let file = file().context("kein Konfigurationsordner (APPDATA/HOME) gefunden")?;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("{} konnte nicht angelegt werden", dir.display()))?;
    }
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    fs::write(&file, text)
        .with_context(|| format!("{} konnte nicht geschrieben werden", file.display()))
}

/// Stellt `dir` an den Anfang, entfernt Doppelte und kürzt auf die Höchstzahl.
pub fn remember(list: &mut Vec<PathBuf>, dir: &Path) {
    list.retain(|p| !same(p, dir));
    list.insert(0, dir.to_path_buf());
    list.truncate(MAX_ENTRIES);
}

fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_moves_to_front_without_duplicates() {
        let mut list = vec![PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c")];
        remember(&mut list, Path::new("b"));
        assert_eq!(
            list,
            [PathBuf::from("b"), PathBuf::from("a"), PathBuf::from("c")]
        );
    }

    #[test]
    fn remember_keeps_at_most_ten() {
        let mut list: Vec<PathBuf> = (0..10).map(|i| PathBuf::from(i.to_string())).collect();
        remember(&mut list, Path::new("neu"));
        assert_eq!(list.len(), MAX_ENTRIES);
        assert_eq!(list[0], PathBuf::from("neu"));
        assert_eq!(list[9], PathBuf::from("8"));
    }

    #[test]
    fn parse_skips_empty_lines() {
        assert_eq!(
            parse("a\n\n  \nb\n"),
            [PathBuf::from("a"), PathBuf::from("b")]
        );
    }
}
