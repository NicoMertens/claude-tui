//! Speicherorte: eigener Konfigurationsordner und der Datenordner von Claude Code.

use std::path::PathBuf;

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
