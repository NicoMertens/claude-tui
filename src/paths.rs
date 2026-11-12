//! Speicherorte: eigener Konfigurationsordner und der Datenordner von Claude Code — dazu das
//! Schreiben dorthin.
//!
//! Im Konfigurationsordner liegt u. a. `hooks-<PID>.json` mit dem Token des Hook-Servers. Unter
//! Unix ist er deshalb nur für den eigenen Benutzer zugänglich (`0700`); unter Windows sorgen die
//! Rechte von `%APPDATA%` dafür.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// `%APPDATA%\claude-tui` bzw. `$XDG_CONFIG_HOME/claude-tui` (Standard `~/.config/claude-tui`).
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))
    };
    base.map(|b| b.join("claude-tui"))
}

/// Ordner mit den Session-Protokollen von Claude Code: `$CLAUDE_CONFIG_DIR/projects`, sonst
/// `~/.claude/projects`.
pub fn claude_projects_dir() -> Option<PathBuf> {
    claude_dir().map(|d| d.join("projects"))
}

/// Ordner, in dem jeder laufende Claude-Code-Prozess `<PID>.json` mit seiner Session-ID ablegt.
pub fn claude_sessions_dir() -> Option<PathBuf> {
    claude_dir().map(|d| d.join("sessions"))
}

fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".claude")))
}

/// Legt `dir` samt fehlender Elternordner an — unter Unix nur für den eigenen Benutzer.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// Beschränkt einen schon bestehenden Konfigurationsordner auf den eigenen Benutzer; Versionen bis
/// 0.8 legten ihn mit den Standardrechten (meist `0755`) an.
pub fn restrict_config_dir() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Some(dir) = config_dir() else {
            return;
        };
        if let Ok(meta) = fs::metadata(&dir) {
            if meta.is_dir() && meta.permissions().mode() & 0o077 != 0 {
                let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
            }
        }
    }
}

/// Schreibt `file` atomar: erst eine eigene temporäre Datei daneben, dann umbenennen. Ein Abbruch
/// hinterlässt so nie eine halbe Datei, und wer liest, sieht den alten oder den neuen Stand. Der
/// Name der temporären Datei ist je Prozess und Aufruf eindeutig — der Hook-Server schreibt aus
/// mehreren Threads, mehrere claude-tui teilen sich den Ordner.
pub fn write_atomic(file: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        create_private_dir(dir)?;
    }
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = file.with_file_name(name);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, file).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("claude-tui-paths-{}", std::process::id()));
        let file = dir.join("neu").join("state.json");
        write_atomic(&file, "eins").expect("schreiben");
        write_atomic(&file, "zwei").expect("ersetzen");
        assert_eq!(fs::read_to_string(&file).expect("lesen"), "zwei");
        let names: Vec<_> = fs::read_dir(file.parent().expect("Ordner"))
            .expect("Ordner")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn created_folders_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("claude-tui-private-{}", std::process::id()));
        create_private_dir(&dir.join("a")).expect("anlegen");
        for d in [&dir, &dir.join("a")] {
            let mode = fs::metadata(d).expect("Ordner").permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", d.display());
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
