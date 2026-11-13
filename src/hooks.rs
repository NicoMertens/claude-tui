//! Zustandsmeldung über Claude-Code-Hooks: claude-tui gibt jedem gestarteten Claude-Prozess per
//! `--settings` Hooks mit. Normalerweise HTTP-Hooks an den eigenen Server (`hook_server.rs`), sonst
//! als Rückfall Befehls-Hooks, die `claude-tui hook <Event>` aufrufen. Der Server legt den Zustand
//! im Speicher ab ([`receive`]); die Befehls-Hooks laufen als eigene Prozesse und schreiben ihn nach
//! `status/<Claude-ID>` im Konfigurationsordner, wo die App ihn liest. Die
//! `~/.claude/settings.json` des Nutzers bleibt unberührt.

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard, OnceLock, PoisonError,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{hook_server, paths, session::Status};

/// URL des Hook-Servers dieses Laufs; `None`, wenn er nicht starten konnte.
static SERVER_URL: OnceLock<Option<String>> = OnceLock::new();

/// Startet den Hook-Server (einmal je Lauf, vor der ersten Session).
pub fn start_server() {
    SERVER_URL.get_or_init(hook_server::start);
}

/// Umgebungsvariable mit der Claude-ID, unter der claude-tui den Prozess gestartet hat. Sie bleibt
/// gleich, wenn Claude im selben Prozess die Session wechselt (`/clear`, `/resume`); die Meldungen
/// laufen weiter unter dieser Start-ID, die aktuelle `session_id` steht im Event-JSON (siehe
/// [`take_switch`]).
pub const SESSION_ENV: &str = "CLAUDE_TUI_SESSION";
const STATUS_DIR: &str = "status";
const SETTINGS_PREFIX: &str = "hooks-";
const SETTINGS_SUFFIX: &str = ".json";
/// Gemeinsame Datei bis v0.7; wird nur noch aufgeräumt.
const LEGACY_SETTINGS_FILE: &str = "hooks.json";
/// Obergrenze je Befehls-Hook in Sekunden. Der Aufruf selbst dauert Millisekunden, aber ein
/// Virenscanner kann den Start der unsignierten Binary um 10 s und mehr verzögern.
const HOOK_TIMEOUT: u64 = 30;
/// Obergrenze je HTTP-Hook in Sekunden. HTTP-Hooks laufen synchron; der Server antwortet in
/// Millisekunden, läuft claude-tui nicht mehr, scheitert die Verbindung sofort.
const HTTP_TIMEOUT: u64 = 5;
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
    let Some(key) = key else {
        return;
    };
    crate::trace::mark(format_args!("hook {event} for {key}"));
    apply(event, &input, &key);
}

/// Wirkung eines Hook-Events auf die Statusdatei der Session `key` (Befehls-Hooks).
fn apply(event: &str, input: &Value, key: &str) {
    let Some(dir) = status_dir() else {
        return;
    };
    match effect(event, input) {
        Effect::Set(status) => {
            let _ = write_report(&dir, key, status, SystemTime::now());
        }
        Effect::Remove => remove_in(&dir, key),
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
        Status::Starting | Status::Exited => return None,
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

fn write_report(dir: &Path, key: &str, status: Status, at: SystemTime) -> Result<()> {
    anyhow::ensure!(valid_key(key), "invalid session key");
    let text = format_report(status, at).context("no hook state for exited")?;
    // Atomar: die App liest nie eine halbe Zeile.
    Ok(paths::write_atomic(&dir.join(key), text)?)
}

fn read_in(dir: &Path, key: &str) -> Option<Report> {
    if !valid_key(key) {
        return None;
    }
    parse_report(&fs::read_to_string(dir.join(key)).ok()?)
}

fn remove_in(dir: &Path, key: &str) {
    if valid_key(key) {
        let _ = fs::remove_file(dir.join(key));
    }
}

/// Meldungen, die der Hook-Server dieses Prozesses empfangen hat. Server und App laufen im selben
/// Prozess — der Umweg über Statusdateien (schreiben, alle 250 ms je Session lesen) entfällt.
static RECEIVED: Mutex<BTreeMap<String, Report>> = Mutex::new(BTreeMap::new());

fn received_map() -> MutexGuard<'static, BTreeMap<String, Report>> {
    RECEIVED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wirkung eines Hook-Events, das der eigene Hook-Server empfangen hat.
pub fn receive(event: &str, input: &Value, key: &str) {
    if !valid_key(key) {
        return;
    }
    note_switch(event, input, key);
    let mut reports = received_map();
    match effect(event, input) {
        Effect::Set(status) => {
            reports.insert(
                key.to_string(),
                Report {
                    status,
                    at: SystemTime::now(),
                },
            );
        }
        Effect::Remove => {
            reports.remove(key);
        }
        Effect::Keep => {}
    }
}

/// Sessionwechsel im selben Prozess: Claude hat mit `/resume` bzw. `/clear` eine andere Session
/// geladen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    /// Die jetzt geladene Session.
    pub session_id: String,
    /// `/clear`: dieselbe Arbeit unter neuer ID, Nummer und Gruppe wandern mit. Bei `/resume`
    /// bleiben sie bei der alten Session.
    pub carry: bool,
}

/// Je Start-ID die zuletzt beendete Session (`SessionEnd` mit `clear`/`resume`) und ob es `/clear`
/// war. Erst ein späteres Event mit anderer `session_id` gilt als Wechsel: Ohne diese Sperre würden
/// auch die Hooks einer von Claude verlegten Session (Hintergrund-Job, gleiche Umgebung) als
/// Wechsel zählen.
static ENDED: Mutex<BTreeMap<String, (String, bool)>> = Mutex::new(BTreeMap::new());
/// Erkannte, noch nicht abgeholte Wechsel je Start-ID.
static SWITCHED: Mutex<BTreeMap<String, Switch>> = Mutex::new(BTreeMap::new());

fn lock<T>(map: &'static Mutex<T>) -> MutexGuard<'static, T> {
    map.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Merkt sich das Ende einer Session und erkennt am nächsten Event den Wechsel.
fn note_switch(event: &str, input: &Value, key: &str) {
    let Some(session_id) = input
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| crate::session::valid_claude_id(id))
    else {
        return;
    };
    // `SessionStart` (über `curl` weitergeleitet) nennt die geladene Session und den Anlass direkt:
    // `/resume`, `/clear` bzw. `/branch` (Quelle `fork`). Gleiche ID wie bisher (etwa `--resume`
    // beim Start) verwirft die App.
    if event == "SessionStart" {
        let source = input.get("source").and_then(Value::as_str);
        if let Some(source @ ("resume" | "clear" | "fork")) = source {
            lock(&ENDED).remove(key);
            crate::trace::mark(format_args!("hook {key}: {source} -> {session_id}"));
            let switch = Switch {
                session_id: session_id.to_string(),
                carry: source == "clear",
            };
            lock(&SWITCHED).insert(key.to_string(), switch);
        }
        return;
    }
    if event == "SessionEnd" {
        let reason = input.get("reason").and_then(Value::as_str);
        if let Some(reason @ ("clear" | "resume")) = reason {
            let clear = reason == "clear";
            lock(&ENDED).insert(key.to_string(), (session_id.to_string(), clear));
        }
        return;
    }
    let Some((old, clear)) = lock(&ENDED).remove(key) else {
        return;
    };
    if session_id == old {
        return;
    }
    crate::trace::mark(format_args!("hook {key}: switched {old} -> {session_id}"));
    let switch = Switch {
        session_id: session_id.to_string(),
        carry: clear,
    };
    lock(&SWITCHED).insert(key.to_string(), switch);
}

/// Holt einen erkannten Sessionwechsel des Prozesses mit dieser Start-ID ab.
pub fn take_switch(hook_id: &str) -> Option<Switch> {
    lock(&SWITCHED).remove(hook_id)
}

/// Letzte Meldung einer Session an den eigenen Hook-Server (billig, ohne Dateizugriff).
pub fn received(claude_id: &str) -> Option<Report> {
    received_map().get(claude_id).copied()
}

/// Letzte Hook-Meldung einer Session aus ihrer Statusdatei (Befehls-Hooks); `None`, solange keine
/// vorliegt.
pub fn read(claude_id: &str) -> Option<Report> {
    read_in(&status_dir()?, claude_id)
}

/// Meldungen einer Session verwerfen (vor dem Start, nach dem Ende).
pub fn clear(claude_id: &str) {
    received_map().remove(claude_id);
    lock(&ENDED).remove(claude_id);
    lock(&SWITCHED).remove(claude_id);
    if let Some(dir) = status_dir() {
        remove_in(&dir, claude_id);
    }
}

/// Entfernt Statusdateien und Hook-Einstellungen abgebrochener Läufe (lange nicht geändert).
/// Trifft es die Einstellungen eines noch laufenden claude-tui, schreibt der sie beim nächsten
/// Session-Start neu.
pub fn purge_stale() {
    let Some(config) = paths::config_dir() else {
        return;
    };
    purge_old_in(&config.join(STATUS_DIR), |_| true);
    purge_old_in(&config, is_stale_settings);
}

fn purge_old_in(dir: &Path, wanted: impl Fn(&str) -> bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_str().is_some_and(&wanted) {
            continue;
        }
        let old = entry
            .metadata()
            .ok()
            .filter(fs::Metadata::is_file)
            .and_then(|m| m.modified().ok())
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Wohin die Hooks melden.
enum Target<'a> {
    /// HTTP-Hook an den eigenen Server.
    Http(&'a str),
    /// Befehls-Hook `claude-tui hook <Event>` (Rückfall ohne Server).
    Command(&'a str),
}

/// Inhalt der `--settings`-Datei: je Event ein Hook.
///
/// HTTP: Claude schickt das Event-JSON per `POST`; die Session-ID kommt über den Header aus
/// `CLAUDE_TUI_SESSION` (nur freigegebene Variablen werden eingesetzt).
///
/// Befehl: Exec-Form (`command` + `args`), Claude startet die Binary direkt ohne Shell. `async`,
/// weil ein langsamer Start sonst jeden Prompt und jedes Werkzeug blockiert; dafür kann eine
/// verspätete Meldung kurz eine neuere überschreiben, bis das nächste Event kommt.
///
/// `disableAgentView`: Mit Agent View verlegt Claude Code eine Session bei `/resume` oder `←` in
/// einen Hintergrund-Job und hängt das Fenster nur noch daran; welche Session es dann zeigt, lässt
/// sich von außen nicht verlässlich feststellen. Ohne Agent View wechselt `/resume` im selben
/// Prozess, und die Hooks melden den Wechsel (siehe [`take_switch`]).
fn settings(target: &Target) -> Value {
    let hooks: serde_json::Map<String, Value> = EVENTS
        .iter()
        .map(|&event| {
            let handler = match target {
                // `SessionStart` unterstützt nur Befehls-Hooks (Claude-Code-Doku; mit 2.1.322
                // geprüft). `curl` des Systems reicht das Event an den eigenen Server weiter: ein
                // signiertes Programm, das kein Virenscanner bei jedem Start ausbremst wie die
                // unsignierte claude-tui-Binary.
                Target::Http(url) if event == "SessionStart" => json!({
                    "type": "command",
                    "command": crate::http::curl_program().to_string_lossy(),
                    "args": forward_args(url),
                    "async": true,
                    "timeout": HTTP_TIMEOUT,
                }),
                Target::Http(url) => json!({
                    "type": "http",
                    "url": url,
                    "headers": {
                        hook_server::SESSION_HEADER: format!("${{{SESSION_ENV}}}"),
                    },
                    "allowedEnvVars": [SESSION_ENV],
                    "timeout": HTTP_TIMEOUT,
                }),
                Target::Command(exe) => json!({
                    "type": "command",
                    "command": exe,
                    "args": ["hook", event],
                    "async": true,
                    "timeout": HOOK_TIMEOUT,
                }),
            };
            (event.to_string(), json!([{ "hooks": [handler] }]))
        })
        .collect();
    json!({ "hooks": hooks, "disableAgentView": true })
}

/// `curl`-Argumente: Event-JSON von stdin an den Hook-Server, Session-ID als Header aus
/// `CLAUDE_TUI_SESSION` (`--variable`/`--expand-header`, ab curl 8.3; Windows 11 hat 8.x). Still,
/// damit nichts in Claudes Kontext landet.
fn forward_args(url: &str) -> Vec<String> {
    [
        "-s",
        "-o",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
        "-m",
        "3",
        "--data-binary",
        "@-",
        "-H",
        "Content-Type: application/json",
        "--variable",
        &format!("%{SESSION_ENV}"),
        "--expand-header",
        &format!("{}: {{{{{SESSION_ENV}}}}}", hook_server::SESSION_HEADER),
        url,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `hooks-<PID>.json`: je laufender claude-tui eine eigene Datei — sie enthält die URL des eigenen
/// Hook-Servers, die eine zweite Instanz sonst überschriebe.
fn settings_file(dir: &Path) -> PathBuf {
    dir.join(format!(
        "{SETTINGS_PREFIX}{}{SETTINGS_SUFFIX}",
        std::process::id()
    ))
}

/// In diesem Lauf schon geschrieben.
static SETTINGS_WRITTEN: AtomicBool = AtomicBool::new(false);

/// Liefert den Pfad der Hook-Einstellungen für `--settings`. Der Inhalt ändert sich während eines
/// Laufs nicht: geschrieben wird beim ersten Aufruf und danach nur, wenn die Datei fehlt
/// (`purge_stale` einer anderen Instanz).
pub fn write_settings() -> Result<PathBuf> {
    let dir = paths::config_dir().context("no config directory (APPDATA/HOME) found")?;
    let file = settings_file(&dir);
    ensure_settings(&file, &SETTINGS_WRITTEN, || {
        let exe = std::env::current_exe().context("own executable not found")?;
        let exe = exe.to_string_lossy();
        let target = match SERVER_URL.get().and_then(Option::as_deref) {
            Some(url) => Target::Http(url),
            None => Target::Command(&exe),
        };
        Ok(serde_json::to_string_pretty(&settings(&target))?)
    })?;
    Ok(file)
}

/// Schreibt `file`, wenn es in diesem Lauf (`written`) noch nicht geschah oder die Datei fehlt.
/// Der erste Aufruf schreibt immer: eine liegengebliebene Datei eines abgestürzten Laufs mit
/// derselben (wiederverwendeten) PID enthielte Port und Token eines toten Servers. Geschrieben wird
/// atomar, damit ein gerade startendes Claude nie eine halb geschriebene Datei liest.
fn ensure_settings(
    file: &Path,
    written: &AtomicBool,
    text: impl FnOnce() -> Result<String>,
) -> Result<()> {
    if written.load(Ordering::Acquire) && file.is_file() {
        return Ok(());
    }
    paths::write_atomic(file, text()?)
        .with_context(|| format!("cannot write {}", file.display()))?;
    written.store(true, Ordering::Release);
    Ok(())
}

/// Entfernt die Hook-Einstellungen dieses Laufs (beim Beenden).
pub fn remove_settings() {
    if let Some(dir) = paths::config_dir() {
        let _ = fs::remove_file(settings_file(&dir));
    }
}

/// Hook-Einstellungen abgebrochener Läufe (lange nicht geändert) und die gemeinsame `hooks.json`
/// älterer Versionen.
fn is_stale_settings(name: &str) -> bool {
    name == LEGACY_SETTINGS_FILE
        || (name.starts_with(SETTINGS_PREFIX)
            && (name.ends_with(SETTINGS_SUFFIX) || name.ends_with(".tmp")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(kind: &str) -> Value {
        json!({ "hook_event_name": "Notification", "notification_type": kind })
    }

    fn event(name: &str, session_id: &str, reason: Option<&str>) -> Value {
        let mut value = json!({"hook_event_name": name, "session_id": session_id});
        if let Some(reason) = reason {
            value["reason"] = json!(reason);
        }
        value
    }

    const OLD: &str = "0b6f3c1e-1d2a-4c5b-9e8f-1234567890ab";
    const NEW: &str = "1c7a4d2f-2e3b-4d6c-8f9a-234567890abc";

    #[test]
    fn resume_switches_on_the_next_event_and_keeps_tags() {
        let key = "switch-resume";
        receive("SessionEnd", &event("SessionEnd", OLD, Some("resume")), key);
        assert_eq!(take_switch(key), None);
        receive(
            "UserPromptSubmit",
            &event("UserPromptSubmit", NEW, None),
            key,
        );
        let expected = Switch {
            session_id: NEW.to_string(),
            carry: false,
        };
        assert_eq!(take_switch(key), Some(expected));
        assert_eq!(take_switch(key), None);
    }

    #[test]
    fn clear_switch_carries_tags() {
        let key = "switch-clear";
        receive("SessionEnd", &event("SessionEnd", OLD, Some("clear")), key);
        receive("Stop", &event("Stop", NEW, None), key);
        assert!(take_switch(key).is_some_and(|s| s.carry));
    }

    #[test]
    fn other_session_ids_without_session_end_are_no_switch() {
        // So melden sich z. B. die Hooks einer von Claude verlegten Session.
        let key = "switch-none";
        receive("Stop", &event("Stop", NEW, None), key);
        receive("SessionEnd", &event("SessionEnd", OLD, Some("other")), key);
        receive("Stop", &event("Stop", NEW, None), key);
        assert_eq!(take_switch(key), None);
    }

    #[test]
    fn same_or_invalid_session_id_is_no_switch() {
        let key = "switch-same";
        receive("SessionEnd", &event("SessionEnd", OLD, Some("resume")), key);
        receive("Stop", &event("Stop", OLD, None), key);
        receive("Stop", &event("Stop", NEW, None), key);
        assert_eq!(take_switch(key), None);

        let key = "switch-invalid";
        receive("SessionEnd", &event("SessionEnd", OLD, Some("resume")), key);
        receive("Stop", &event("Stop", "$(calc)", None), key);
        assert_eq!(take_switch(key), None);
    }

    #[test]
    fn session_start_reports_resume_clear_and_branch() {
        let start = |source: &str, id: &str| {
            let mut value = event("SessionStart", id, None);
            value["source"] = json!(source);
            value
        };
        let key = "switch-start";
        receive("SessionStart", &start("startup", OLD), key);
        assert_eq!(take_switch(key), None);
        receive("SessionStart", &start("fork", NEW), key);
        let expected = Switch {
            session_id: NEW.to_string(),
            carry: false,
        };
        assert_eq!(take_switch(key), Some(expected));
        receive("SessionStart", &start("clear", OLD), key);
        assert!(take_switch(key).is_some_and(|s| s.carry));
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
    fn settings_files_are_per_process_and_purgeable() {
        let file = settings_file(Path::new("cfg"));
        let name = file.file_name().and_then(|n| n.to_str()).expect("Name");
        assert_eq!(name, format!("hooks-{}.json", std::process::id()));
        assert!(is_stale_settings(name));
        assert!(is_stale_settings("hooks.json"));
        assert!(!is_stale_settings("state.json"));
        assert!(is_stale_settings("hooks-1.json.4711.0.tmp"));
        assert!(!is_stale_settings("hooks-notes.txt"));
    }

    #[test]
    fn leftover_settings_are_replaced_once_then_kept() {
        let dir = std::env::temp_dir().join(format!("claude-tui-settings-{}", std::process::id()));
        let file = dir.join("hooks-1.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&file, "alter Lauf").unwrap();
        let written = AtomicBool::new(false);
        ensure_settings(&file, &written, || Ok("neu".into())).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "neu", "Rest ersetzt");
        ensure_settings(&file, &written, || Ok("nochmal".into())).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "neu",
            "nicht neu geschrieben"
        );
        fs::remove_file(&file).unwrap();
        ensure_settings(&file, &written, || Ok("wieder da".into())).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "wieder da", "fehlte");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn server_reports_live_in_memory_until_cleared() {
        let key = "7f0c2a9e-0000-4000-8000-00000000abcd";
        receive("PermissionRequest", &Value::Null, key);
        assert_eq!(received(key).map(|r| r.status), Some(Status::Waiting));
        receive("Stop", &Value::Null, key);
        assert_eq!(received(key).map(|r| r.status), Some(Status::Idle));
        receive("SessionEnd", &Value::Null, key);
        assert_eq!(received(key), None);
        receive("UserPromptSubmit", &Value::Null, key);
        clear(key);
        assert_eq!(received(key), None);
        receive("Stop", &Value::Null, "../böse");
        assert_eq!(received("../böse"), None);
    }

    #[test]
    fn garbage_is_no_report() {
        assert_eq!(parse_report(""), None);
        assert_eq!(parse_report("working"), None);
        assert_eq!(parse_report("busy 12"), None);
    }

    #[test]
    fn http_settings_pass_session_header() {
        let value = settings(&Target::Http("http://127.0.0.1:4711/hook/t"));
        let stop = &value["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(stop["type"], "http");
        assert_eq!(stop["url"], "http://127.0.0.1:4711/hook/t");
        assert_eq!(
            stop["headers"]["X-Claude-Tui-Session"],
            "${CLAUDE_TUI_SESSION}"
        );
        assert_eq!(stop["allowedEnvVars"], json!(["CLAUDE_TUI_SESSION"]));
        assert_eq!(value["hooks"].as_object().unwrap().len(), EVENTS.len());
        assert_eq!(value["disableAgentView"], true);
    }

    #[test]
    fn session_start_is_forwarded_by_curl_with_session_header() {
        let value = settings(&Target::Http("http://127.0.0.1:4711/hook/t"));
        let start = &value["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(start["type"], "command");
        assert!(start["command"].as_str().unwrap().contains("curl"));
        let args: Vec<&str> = start["args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(args.contains(&"%CLAUDE_TUI_SESSION"));
        assert!(args.contains(&"X-Claude-Tui-Session: {{CLAUDE_TUI_SESSION}}"));
        assert_eq!(args.last(), Some(&"http://127.0.0.1:4711/hook/t"));
    }

    #[test]
    fn settings_use_exec_form_for_every_event() {
        let value = settings(&Target::Command(
            r"C:\Program Files\claude-tui\claude-tui.exe",
        ));
        let stop = &value["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(
            stop["command"],
            r"C:\Program Files\claude-tui\claude-tui.exe"
        );
        assert_eq!(stop["args"], json!(["hook", "Stop"]));
        assert_eq!(stop["async"], json!(true));
        assert_eq!(value["hooks"].as_object().unwrap().len(), EVENTS.len());
    }
}
