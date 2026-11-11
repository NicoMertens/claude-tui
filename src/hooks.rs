//! Zustandsmeldung über Claude-Code-Hooks: claude-tui gibt jedem gestarteten Claude-Prozess per
//! `--settings` Hooks mit, die `claude-tui hook <Event>` aufrufen. Der Aufruf schreibt den Zustand
//! in `status/<Claude-ID>` im Konfigurationsordner; die App liest ihn dort. Die
//! `~/.claude/settings.json` des Nutzers bleibt unberührt.

use std::{
    fs,
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{paths, session::Status};

/// Umgebungsvariable mit der Claude-ID, unter der claude-tui den Prozess führt. Sie bleibt auch
/// nach `/clear` gleich, wenn Claude intern eine neue `session_id` vergibt.
pub const SESSION_ENV: &str = "CLAUDE_TUI_SESSION";
const STATUS_DIR: &str = "status";
const SETTINGS_FILE: &str = "hooks.json";
/// Obergrenze je Hook-Aufruf in Sekunden; der Aufruf selbst dauert nur Millisekunden.
const HOOK_TIMEOUT: u64 = 5;
/// Statusdateien, die so lange unverändert sind, stammen aus abgebrochenen Läufen.
const STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Events, für die claude-tui einen Hook einträgt.
const EVENTS: [&str; 11] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "PreCompact",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

/// Was ein Hook-Event für die Statusdatei bedeutet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Set(Status),
    /// Session beendet: Statusdatei entfernen.
    Remove,
    /// Event ändert den Zustand nicht.
    Keep,
}

/// Letzte Meldung eines Hooks: Zustand und Zeitpunkt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub status: Status,
    pub at: SystemTime,
}

/// Event → Zustand. `PermissionRequest` kommt sofort, wenn Claude eine Freigabe braucht;
/// `Notification` mit `permission_prompt` erst nach einigen Sekunden ohne Eingabe und dient als
/// Rückfall. `idle_prompt` meldet Claude rund 60 s nach dem Ende einer Antwort.
pub fn effect(event: &str, input: &Value) -> Effect {
    match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "PreCompact" => {
            Effect::Set(Status::Working)
        }
        "PermissionRequest" => Effect::Set(Status::Waiting),
        "Notification" => match input.get("notification_type").and_then(Value::as_str) {
            Some("permission_prompt" | "elicitation_dialog") => Effect::Set(Status::Waiting),
            Some("idle_prompt") => Effect::Set(Status::Idle),
            _ => Effect::Keep,
        },
        "SessionStart" | "Stop" | "StopFailure" => Effect::Set(Status::Idle),
        "SessionEnd" => Effect::Remove,
        _ => Effect::Keep,
    }
}

/// `claude-tui hook <Event>`: liest das Hook-JSON von stdin und schreibt den Zustand. Gibt nie
/// etwas aus und scheitert nie sichtbar — Claude soll weder blockiert werden noch Meldungen sehen
/// (Ausgabe bei `SessionStart`/`UserPromptSubmit` landete sonst im Kontext).
pub fn run_cli(event: Option<&str>) {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let input: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let event = event
        .or_else(|| input.get("hook_event_name").and_then(Value::as_str))
        .unwrap_or_default();
    let key = std::env::var(SESSION_ENV).ok().or_else(|| {
        input
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let (Some(key), Some(dir)) = (key, status_dir()) else {
        return;
    };
    match effect(event, &input) {
        Effect::Set(status) => {
            let _ = write_report(&dir, &key, status, SystemTime::now());
        }
        Effect::Remove => remove_in(&dir, &key),
        Effect::Keep => {}
    }
}

fn status_dir() -> Option<PathBuf> {
    paths::config_dir().map(|d| d.join(STATUS_DIR))
}

/// Nur UUID-artige Namen werden zu Dateinamen — kein Pfad aus fremder Eingabe.
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn format_report(status: Status, at: SystemTime) -> Option<String> {
    let name = match status {
        Status::Working => "working",
        Status::Waiting => "waiting",
        Status::Idle => "idle",
        Status::Exited => return None,
    };
    let millis = at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Some(format!("{name} {millis}"))
}

fn parse_report(text: &str) -> Option<Report> {
    let (name, millis) = text.trim().split_once(' ')?;
    let status = match name {
        "working" => Status::Working,
        "waiting" => Status::Waiting,
        "idle" => Status::Idle,
        _ => return None,
    };
    let at = UNIX_EPOCH + Duration::from_millis(millis.parse().ok()?);
    Some(Report { status, at })
}

fn write_report(dir: &std::path::Path, key: &str, status: Status, at: SystemTime) -> Result<()> {
    anyhow::ensure!(valid_key(key), "invalid session key");
    let text = format_report(status, at).context("no hook state for exited")?;
    fs::create_dir_all(dir)?;
    // Erst eine eigene Datei schreiben, dann umbenennen: die App liest nie eine halbe Zeile.
    let tmp = dir.join(format!("{key}.{}.tmp", std::process::id()));
    fs::write(&tmp, text)?;
    let result = fs::rename(&tmp, dir.join(key));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

fn read_in(dir: &std::path::Path, key: &str) -> Option<Report> {
    if !valid_key(key) {
        return None;
    }
    parse_report(&fs::read_to_string(dir.join(key)).ok()?)
}

fn remove_in(dir: &std::path::Path, key: &str) {
    if valid_key(key) {
        let _ = fs::remove_file(dir.join(key));
    }
}

/// Letzte Hook-Meldung einer Session; `None`, solange keine vorliegt.
pub fn read(claude_id: &str) -> Option<Report> {
    read_in(&status_dir()?, claude_id)
}

/// Statusdatei einer Session entfernen (vor dem Start, nach dem Ende).
pub fn clear(claude_id: &str) {
    if let Some(dir) = status_dir() {
        remove_in(&dir, claude_id);
    }
}

/// Entfernt Statusdateien abgebrochener Läufe (lange nicht geändert).
pub fn purge_stale() {
    let Some(entries) = status_dir().and_then(|d| fs::read_dir(d).ok()) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Inhalt der `--settings`-Datei: je Event ein Hook in Exec-Form (`command` + `args`). Claude
/// startet die Binary dann direkt ohne Shell, Pfade mit Leerzeichen brauchen kein Quoting.
fn settings(exe: &str) -> Value {
    let hooks: serde_json::Map<String, Value> = EVENTS
        .iter()
        .map(|event| {
            let handler = json!({
                "type": "command",
                "command": exe,
                "args": ["hook", event],
                "timeout": HOOK_TIMEOUT,
            });
            (event.to_string(), json!([{ "hooks": [handler] }]))
        })
        .collect();
    json!({ "hooks": hooks })
}

/// Schreibt die Hook-Einstellungen in den Konfigurationsordner und liefert den Pfad für
/// `--settings`.
pub fn write_settings() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("own executable not found")?;
    let dir = paths::config_dir().context("no config directory (APPDATA/HOME) found")?;
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let file = dir.join(SETTINGS_FILE);
    let text = serde_json::to_string_pretty(&settings(&exe.to_string_lossy()))?;
    fs::write(&file, text).with_context(|| format!("cannot write {}", file.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(kind: &str) -> Value {
        json!({ "hook_event_name": "Notification", "notification_type": kind })
    }

    #[test]
    fn events_map_to_states() {
        let none = Value::Null;
        assert_eq!(
            effect("UserPromptSubmit", &none),
            Effect::Set(Status::Working)
        );
        assert_eq!(effect("PreToolUse", &none), Effect::Set(Status::Working));
        assert_eq!(
            effect("PermissionRequest", &none),
            Effect::Set(Status::Waiting)
        );
        assert_eq!(effect("Stop", &none), Effect::Set(Status::Idle));
        assert_eq!(effect("SessionStart", &none), Effect::Set(Status::Idle));
        assert_eq!(effect("SessionEnd", &none), Effect::Remove);
        assert_eq!(effect("SubagentStop", &none), Effect::Keep);
    }

    #[test]
    fn notification_type_decides() {
        assert_eq!(
            effect("Notification", &notification("permission_prompt")),
            Effect::Set(Status::Waiting)
        );
        assert_eq!(
            effect("Notification", &notification("idle_prompt")),
            Effect::Set(Status::Idle)
        );
        assert_eq!(
            effect("Notification", &notification("auth_success")),
            Effect::Keep
        );
    }

    #[test]
    fn report_roundtrip_through_file() {
        let dir = std::env::temp_dir().join(format!("claude-tui-hooks-{}", std::process::id()));
        let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        write_report(&dir, "abc-123", Status::Waiting, at).unwrap();
        assert_eq!(
            read_in(&dir, "abc-123"),
            Some(Report {
                status: Status::Waiting,
                at
            })
        );
        remove_in(&dir, "abc-123");
        assert_eq!(read_in(&dir, "abc-123"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_cannot_escape_the_status_folder() {
        assert!(valid_key("0b6f3c1e-1d2a-4c5b-9e8f-1234567890ab"));
        assert!(!valid_key("../state.json"));
        assert!(!valid_key(""));
        let dir = std::env::temp_dir();
        assert!(write_report(&dir, "..\\x", Status::Idle, SystemTime::now()).is_err());
    }

    #[test]
    fn garbage_is_no_report() {
        assert_eq!(parse_report(""), None);
        assert_eq!(parse_report("working"), None);
        assert_eq!(parse_report("busy 12"), None);
    }

    #[test]
    fn settings_use_exec_form_for_every_event() {
        let value = settings(r"C:\Program Files\claude-tui\claude-tui.exe");
        let stop = &value["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(
            stop["command"],
            r"C:\Program Files\claude-tui\claude-tui.exe"
        );
        assert_eq!(stop["args"], json!(["hook", "Stop"]));
        assert_eq!(value["hooks"].as_object().unwrap().len(), EVENTS.len());
    }
}
