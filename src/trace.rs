//! Zeitmessung auf Zuruf: Mit `CLAUDE_TUI_TRACE=1` schreibt claude-tui Zeitstempel nach
//! `<Konfigurationsordner>/trace.log` — Start (Protokolle, Wiederherstellen, erstes Bild), je
//! Session Prozessstart, erste Ausgabe und erste Hook-Meldung, dazu jeden Hook-Aufruf.
//!
//! Jede Zeile trägt Unix-Millisekunden und die Prozess-ID, damit sich die Zeilen der Hook-Prozesse
//! (eigene Prozesse, von Claude gestartet) mit denen der App vergleichen lassen. Ohne die Variable
//! kostet ein Aufruf nur eine Abfrage eines `OnceLock`.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::{Mutex, OnceLock},
    time::{Instant, SystemTime},
};

use crate::paths;

pub const ENV: &str = "CLAUDE_TUI_TRACE";
const FILE_NAME: &str = "trace.log";

struct Trace {
    start: Instant,
    file: Mutex<File>,
}

static TRACE: OnceLock<Option<Trace>> = OnceLock::new();

fn trace() -> Option<&'static Trace> {
    TRACE.get_or_init(open).as_ref()
}

fn open() -> Option<Trace> {
    let value = std::env::var(ENV).ok()?;
    if value.is_empty() || value == "0" {
        return None;
    }
    let dir = paths::config_dir()?;
    paths::create_private_dir(&dir).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(FILE_NAME))
        .ok()?;
    Some(Trace {
        start: Instant::now(),
        file: Mutex::new(file),
    })
}

/// Schreibt `<Unix-ms> <PID> +<ms seit Prozessstart> <Text>`.
pub fn mark(text: impl std::fmt::Display) {
    let Some(trace) = trace() else {
        return;
    };
    let unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let since = trace.start.elapsed().as_secs_f64() * 1000.0;
    let line = format!(
        "{unix_ms} {:>6} +{since:>9.1}ms {text}\n",
        std::process::id()
    );
    if let Ok(mut file) = trace.file.lock() {
        let _ = file.write_all(line.as_bytes());
    }
}
