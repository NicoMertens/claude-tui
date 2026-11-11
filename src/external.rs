//! Erkennt Claude-Sessions, die außerhalb von claude-tui laufen (z. B. in einem anderen Terminal).
//!
//! Hauptquelle: Claude Code legt je laufendem Prozess `~/.claude/sessions/<PID>.json` mit `pid`
//! und `sessionId` an und entfernt die Datei beim Beenden. Nach einem Absturz kann sie liegen
//! bleiben; deshalb zählt sie nur, solange der Prozess mit dieser PID noch existiert.
//! Rückfall für Versionen ohne diese Datei: ein Protokoll, das gerade geschrieben wurde, ohne dass
//! claude-tui die Session selbst ausführt.
//!
//! Hintergrund-Sessions (`claude --bg`, `"kind":"bg"`) tragen zusätzlich eine `jobId`: Solange sie
//! laufen, lehnt `claude --resume` sie ab; geöffnet werden sie mit `claude attach <jobId>`.
//!
//! Das Lesen der Dateien und die Prozessabfrage (`tasklist` braucht unter Windows fast eine
//! Sekunde) laufen in einem Hintergrund-Thread, nicht im Zeichentakt.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant, SystemTime},
};

use serde::Deserialize;

use crate::paths;

/// So oft liest der Hintergrund-Thread `~/.claude/sessions`.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Die Prozessliste wird neu abgefragt, wenn sich die PIDs geändert haben, sonst in diesem Abstand
/// (für Dateien, die ein abgestürzter Prozess hinterlassen hat).
const ALIVE_RECHECK: Duration = Duration::from_secs(20);
/// Rückfall-Heuristik: so jung muss die letzte Änderung am Protokoll sein.
pub const RECENT_WRITE: Duration = Duration::from_secs(10);

/// Ein laufender Claude-Prozess laut `~/.claude/sessions/<PID>.json`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Claim {
    pid: u32,
    session_id: String,
    /// Nur bei Hintergrund-Sessions.
    #[serde(default)]
    job_id: Option<String>,
}

/// Session-IDs laufender Claude-Prozesse, je mit der Job-ID, falls es eine Hintergrund-Session ist.
pub type Claimed = HashMap<String, Option<String>>;

fn parse_claim(text: &str) -> Option<Claim> {
    serde_json::from_str(text).ok()
}

fn read_claims(dir: &Path) -> Vec<Claim> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| fs::read_to_string(p).ok())
        .filter_map(|text| parse_claim(&text))
        .collect()
}

/// Sessions der Prozesse, die noch laufen. `alive == None` (Prozessliste nicht abfragbar):
/// jede Datei zählt — lieber einmal zu oft nachfragen als zweimal öffnen.
fn claimed_ids(claims: &[Claim], alive: Option<&HashSet<u32>>) -> Claimed {
    claims
        .iter()
        .filter(|c| alive.is_none_or(|a| a.contains(&c.pid)))
        .map(|c| (c.session_id.clone(), c.job_id.clone()))
        .collect()
}

/// Einmalige, blockierende Abfrage für `ids` (beim Wiederherstellen des Fensterlayouts). Die
/// Prozessliste wird nur abgefragt, wenn eine der Sessions überhaupt in einer Datei steht.
pub fn claimed_now(ids: &[String]) -> Claimed {
    let Some(dir) = paths::claude_sessions_dir() else {
        return Claimed::new();
    };
    let claims: Vec<Claim> = read_claims(&dir)
        .into_iter()
        .filter(|c| ids.contains(&c.session_id))
        .collect();
    if claims.is_empty() {
        return Claimed::new();
    }
    let pids = claims.iter().map(|c| c.pid).collect();
    claimed_ids(&claims, alive_pids(&pids).as_ref())
}

/// Startet den Hintergrund-Thread; er liefert alle `SCAN_INTERVAL` die Sessions laufender
/// Claude-Prozesse und endet, sobald der Empfänger fallen gelassen wird.
pub fn spawn_watcher() -> Option<Receiver<Claimed>> {
    let dir = paths::claude_sessions_dir()?;
    let (tx, rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("external-sessions".into())
        .spawn(move || watch(&dir, &tx));
    spawned.ok().map(|_| rx)
}

fn watch(dir: &Path, tx: &mpsc::Sender<Claimed>) {
    let mut last_pids = BTreeSet::new();
    let mut alive: Option<HashSet<u32>> = None;
    let mut checked: Option<Instant> = None;
    loop {
        let claims = read_claims(dir);
        let pids: BTreeSet<u32> = claims.iter().map(|c| c.pid).collect();
        let stale = checked.is_none_or(|t| t.elapsed() >= ALIVE_RECHECK);
        if !pids.is_empty() && (pids != last_pids || stale) {
            alive = alive_pids(&pids);
            checked = Some(Instant::now());
        }
        last_pids = pids;
        if tx.send(claimed_ids(&claims, alive.as_ref())).is_err() {
            return;
        }
        thread::sleep(SCAN_INTERVAL);
    }
}

/// Welche der `pids` noch laufen; `None`, wenn sich das nicht feststellen lässt.
fn alive_pids(pids: &BTreeSet<u32>) -> Option<HashSet<u32>> {
    let running = if cfg!(windows) {
        let output = Command::new(tasklist_program())
            .args(["/FO", "CSV", "/NH"])
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        parse_tasklist(&String::from_utf8_lossy(&output.stdout))
    } else if Path::new("/proc/self").exists() {
        return Some(
            pids.iter()
                .copied()
                .filter(|pid| Path::new(&format!("/proc/{pid}")).exists())
                .collect(),
        );
    } else {
        let output = Command::new("ps")
            .args(["-A", "-o", "pid="])
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        parse_pid_lines(&String::from_utf8_lossy(&output.stdout))
    };
    Some(
        pids.iter()
            .copied()
            .filter(|p| running.contains(p))
            .collect(),
    )
}

/// Fester Pfad wie bei `curl` (siehe `http.rs`): kein fremdes `tasklist.exe` aus PATH oder
/// aktuellem Ordner.
fn tasklist_program() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("tasklist.exe")
}

/// `"claude.exe","8072","Console","1","406,204 K"` je Zeile: die PID ist das zweite Feld.
fn parse_tasklist(text: &str) -> HashSet<u32> {
    text.lines()
        .filter_map(|line| line.split("\",\"").nth(1))
        .filter_map(|pid| pid.trim_matches('"').parse().ok())
        .collect()
}

fn parse_pid_lines(text: &str) -> HashSet<u32> {
    text.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

/// Gespeicherte Sessions, die anderswo laufen: von einem lebenden Prozess beansprucht
/// (`claimed`) oder seit `since` (Start von claude-tui) vor höchstens [`RECENT_WRITE`]
/// geschrieben. `own` sind Sessions, die in claude-tui laufen oder dort gerade beendet wurden —
/// ihre Datei bzw. letzte Schreibzugriffe stammen von claude-tui selbst.
pub fn running_elsewhere<'a>(
    stored: impl IntoIterator<Item = (&'a str, SystemTime)>,
    claimed: &Claimed,
    own: &HashSet<&str>,
    since: SystemTime,
    now: SystemTime,
) -> HashSet<String> {
    stored
        .into_iter()
        .filter(|(id, _)| !own.contains(id))
        .filter(|&(id, modified)| {
            let recent = modified >= since
                && now.duration_since(modified).unwrap_or_default() <= RECENT_WRITE;
            claimed.contains_key(id) || recent
        })
        .map(|(id, _)| id.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(pid: u32, id: &str) -> Claim {
        Claim {
            pid,
            session_id: id.to_string(),
            job_id: None,
        }
    }

    #[test]
    fn claim_file_yields_pid_and_session_id() {
        let text = r#"{"pid":8072,"sessionId":"befc32e4","cwd":"S:\\private","kind":"bg"}"#;
        assert_eq!(parse_claim(text), Some(claim(8072, "befc32e4")));
        assert_eq!(parse_claim(r#"{"pid":1}"#), None, "ohne Session-ID");
    }

    #[test]
    fn background_claim_carries_job_id() {
        let text = r#"{"pid":8072,"sessionId":"befc32e4-a6ea","kind":"bg","jobId":"befc32e4"}"#;
        let claim = parse_claim(text).expect("Claim");
        assert_eq!(claim.job_id.as_deref(), Some("befc32e4"));
        let claimed = claimed_ids(&[claim], None);
        assert_eq!(claimed["befc32e4-a6ea"].as_deref(), Some("befc32e4"));
    }

    #[test]
    fn only_live_processes_count() {
        let claims = [claim(1, "a"), claim(2, "b")];
        let alive = HashSet::from([2]);
        assert_eq!(
            claimed_ids(&claims, Some(&alive)),
            Claimed::from([("b".to_string(), None)])
        );
        assert_eq!(claimed_ids(&claims, None).len(), 2, "unbekannt = alle");
    }

    #[test]
    fn tasklist_csv_and_ps_output_are_parsed() {
        let csv = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
                   \"claude.exe\",\"8072\",\"Console\",\"1\",\"406,204 K\"\r\n";
        assert_eq!(parse_tasklist(csv), HashSet::from([0, 8072]));
        assert_eq!(parse_pid_lines("  1\n 42\n"), HashSet::from([1, 42]));
    }

    #[test]
    fn own_process_is_alive() {
        let me = std::process::id();
        let alive = alive_pids(&BTreeSet::from([me, u32::MAX - 1])).expect("Prozessliste");
        assert_eq!(alive, HashSet::from([me]));
    }

    #[test]
    fn claimed_or_freshly_written_sessions_run_elsewhere_unless_own() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let now = start + Duration::from_secs(100);
        let stored = [
            ("claimed", start - Duration::from_secs(500)),
            ("fresh", now - Duration::from_secs(3)),
            ("old", now - RECENT_WRITE - Duration::from_secs(1)),
            ("before-start", start - Duration::from_secs(1)),
            ("own", now),
            ("own-claimed", now),
        ];
        let claimed = Claimed::from([
            ("claimed".to_string(), None),
            ("own-claimed".to_string(), None),
        ]);
        let own = HashSet::from(["own", "own-claimed"]);
        let found = running_elsewhere(stored, &claimed, &own, start, now);
        let mut found: Vec<_> = found.into_iter().collect();
        found.sort();
        assert_eq!(found, ["claimed", "fresh"]);
    }

    #[test]
    fn write_before_start_does_not_count_even_if_recent() {
        // claude-tui gerade neu gestartet: die letzten Schreibzugriffe stammen vom alten Lauf.
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let now = start + Duration::from_secs(2);
        let stored = [("restored", start - Duration::from_secs(1))];
        let found = running_elsewhere(stored, &Claimed::new(), &HashSet::new(), start, now);
        assert!(found.is_empty());
    }
}
