//! Eine Datei im Konfigurationsordner, die mehrere claude-tui-Instanzen teilen (Notizblock,
//! Todo-Liste): gespeichert wird spätestens [`SAVE_DELAY`] nach der ersten Änderung; ändert eine
//! andere Instanz die Datei, wird sie neu gelesen, solange hier nichts ungespeichert ist — sonst
//! gewinnt, wer zuletzt speichert. Lässt sich die Datei nicht lesen (sie fehlt nicht nur), wird sie
//! nie überschrieben, sonst wäre ihr Inhalt verloren.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};

use crate::paths;

/// So lange nach der ersten ungespeicherten Änderung wird gespeichert (Tippen schreibt nicht bei
/// jeder Taste); ebenso oft wird geprüft, ob eine andere Instanz die Datei geändert hat.
pub const SAVE_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct SyncedFile {
    name: &'static str,
    /// Bezeichnung in Fehlermeldungen, z. B. `notepad`.
    label: &'static str,
    /// Zeitpunkt der ersten noch nicht gespeicherten Änderung.
    unsaved_since: Option<Instant>,
    /// Änderungszeit der Datei beim letzten Lesen bzw. Schreiben.
    disk: Option<SystemTime>,
    last_check: Option<Instant>,
    blocked: bool,
}

impl SyncedFile {
    pub const fn new(name: &'static str, label: &'static str) -> Self {
        Self {
            name,
            label,
            unsaved_since: None,
            disk: None,
            last_check: None,
            blocked: false,
        }
    }

    /// Liest die Datei; `Ok(None)`, wenn es sie (oder den Konfigurationsordner) nicht gibt. Bei
    /// einem Fehler wird nie gespeichert — die Meldung sagt das dem Benutzer.
    pub fn load(&mut self) -> Result<Option<String>> {
        let Some(file) = self.path() else {
            return Ok(None);
        };
        match read(&file) {
            Ok(Some((text, modified))) => {
                self.disk = modified;
                Ok(Some(text))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                self.blocked = true;
                Err(e.context(format!("{} not loaded, changes are not saved", self.label)))
            }
        }
    }

    /// Den Inhalt nicht mehr speichern (er ließ sich nicht verstehen, z. B. kaputtes JSON).
    pub fn block(&mut self) {
        self.blocked = true;
    }

    /// Merkt eine Änderung vor; gespeichert wird mit [`Self::save_if_due`].
    pub fn touch(&mut self) {
        self.unsaved_since.get_or_insert_with(Instant::now);
    }

    pub fn is_unsaved(&self) -> bool {
        self.unsaved_since.is_some()
    }

    /// Neuer Inhalt, falls eine andere Instanz die Datei geändert hat. Prüft höchstens alle
    /// [`SAVE_DELAY`] und nur, wenn hier nichts ungespeichert ist.
    pub fn poll(&mut self) -> Result<Option<String>> {
        if self.blocked
            || self.unsaved_since.is_some()
            || self.last_check.is_some_and(|at| at.elapsed() < SAVE_DELAY)
        {
            return Ok(None);
        }
        self.last_check = Some(Instant::now());
        let Some(file) = self.path() else {
            return Ok(None);
        };
        let modified = modified(&file);
        if modified.is_none() || modified == self.disk {
            return Ok(None);
        }
        self.disk = modified;
        match read(&file) {
            Ok(Some((text, modified))) => {
                self.disk = modified;
                Ok(Some(text))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                self.blocked = true;
                Err(e.context(format!(
                    "{} changed elsewhere, changes are no longer saved",
                    self.label
                )))
            }
        }
    }

    /// Speichert `content()`, wenn sich seit [`SAVE_DELAY`] nichts mehr geändert hat (`force`:
    /// sofort).
    pub fn save_if_due(&mut self, force: bool, content: impl FnOnce() -> String) -> Result<()> {
        let Some(since) = self.unsaved_since else {
            return Ok(());
        };
        if self.blocked {
            self.unsaved_since = None;
            return Ok(());
        }
        if !force && since.elapsed() < SAVE_DELAY {
            return Ok(());
        }
        let file = self.path().context("no config directory")?;
        let saved =
            paths::write_atomic(&file, content()).with_context(|| format!("saving {}", self.label));
        // Fehlgeschlagen: nach SAVE_DELAY erneut versuchen, nicht in jedem Durchlauf.
        self.unsaved_since = saved.is_err().then(Instant::now);
        if saved.is_ok() {
            self.disk = modified(&file);
        }
        saved
    }

    fn path(&self) -> Option<PathBuf> {
        paths::config_dir().map(|d| d.join(self.name))
    }
}

fn modified(file: &Path) -> Option<SystemTime> {
    fs::metadata(file).and_then(|m| m.modified()).ok()
}

/// Text (ohne BOM) und Änderungszeit der Datei; `None`, wenn es sie nicht gibt.
fn read(file: &Path) -> Result<Option<(String, Option<SystemTime>)>> {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not UTF-8", file.display()))?;
    let text = match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    };
    Ok(Some((text, modified(file))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_file_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("claude-tui-filesync-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("Ordner");
        let file = dir.join("x.txt");
        fs::write(&file, [0xff, 0xfe, b'a']).expect("schreiben");
        assert!(read(&file).is_err(), "kein UTF-8");
        assert_eq!(read(&dir.join("fehlt.txt")).expect("fehlt"), None);
        fs::write(&file, "\u{feff}ab").expect("schreiben");
        assert_eq!(read(&file).expect("BOM").map(|(t, _)| t), Some("ab".into()));
        let mut f = SyncedFile::new("x.txt", "x");
        f.block();
        f.touch();
        f.save_if_due(true, || unreachable!("blockiert: nie schreiben"))
            .expect("nichts zu tun");
        assert!(!f.is_unsaved());
        let _ = fs::remove_dir_all(&dir);
    }
}
