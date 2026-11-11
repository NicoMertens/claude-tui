//! Selbst-Update über die GitHub-Releases des Projekts.
//!
//! Ablauf: neuestes Release abfragen, Version vergleichen, das Archiv für die eigene Plattform
//! (`claude-tui-v<Version>-<Ziel>.zip|.tar.gz`) und `sha256.sum` laden, Prüfsumme vergleichen,
//! Binary entpacken und die laufende Datei ersetzen (`self-replace`; unter Windows über
//! Umbenennen, weil die laufende `.exe` gesperrt ist). Wirksam ab dem nächsten Start.

use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, SystemTime},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths;

const REPO: &str = "NicoMertens/claude-tui";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const USER_AGENT: &str = concat!("claude-tui/", env!("CARGO_PKG_VERSION"));
/// Höchstens so oft fragt claude-tui beim Start nach einer neuen Version.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CHECK_FILE: &str = "update-check.json";
/// Abschalten der automatischen Prüfung beim Start.
const OPT_OUT_ENV: &str = "CLAUDE_TUI_NO_UPDATE_CHECK";

/// Zieltripel, wie es in den Archivnamen des Release-Workflows steht.
const TARGET: Option<&str> = if cfg!(all(windows, target_arch = "x86_64")) {
    Some("x86_64-pc-windows-msvc")
} else if cfg!(all(windows, target_arch = "aarch64")) {
    Some("aarch64-pc-windows-msvc")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("x86_64-unknown-linux-gnu")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("aarch64-apple-darwin")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("x86_64-apple-darwin")
} else {
    None
};
const ARCHIVE_EXT: &str = if cfg!(windows) { "zip" } else { "tar.gz" };
const EXE_NAME: &str = if cfg!(windows) {
    "claude-tui.exe"
} else {
    "claude-tui"
};

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Eine neuere Version, die sich installieren lässt.
#[derive(Clone, Debug)]
pub struct Available {
    pub version: String,
    archive_name: String,
    archive_url: String,
    sums_url: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// Fragt das neueste Release ab; `None`, wenn es nicht neuer ist als die laufende Version.
pub fn check() -> Result<Option<Available>> {
    check_against(CURRENT)
}

fn check_against(current: &str) -> Result<Option<Available>> {
    let target = TARGET.context("no release builds for this platform")?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let text = agent()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("cannot reach GitHub")?
        .body_mut()
        .read_to_string()?;
    let release: Release = serde_json::from_str(&text).context("unexpected answer from GitHub")?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    if !is_newer(&version, current) {
        return Ok(None);
    }
    let archive_name = format!("claude-tui-{}-{target}.{ARCHIVE_EXT}", release.tag_name);
    let url_of = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
    };
    let archive_url = url_of(&archive_name)
        .with_context(|| format!("release {} has no {archive_name}", release.tag_name))?;
    let sums_url = url_of("sha256.sum")
        .with_context(|| format!("release {} has no sha256.sum", release.tag_name))?;
    Ok(Some(Available {
        version,
        archive_name,
        archive_url,
        sums_url,
    }))
}

/// Lädt, prüft und installiert die neue Version; gilt ab dem nächsten Start.
pub fn install(update: &Available) -> Result<()> {
    let binary = fetch_binary(update)?;
    let staged = std::env::temp_dir().join(format!("claude-tui-{}-{EXE_NAME}", update.version));
    fs::write(&staged, &binary).with_context(|| format!("cannot write {}", staged.display()))?;
    make_executable(&staged)?;
    let replaced = self_replace::self_replace(&staged).context("cannot replace claude-tui");
    let _ = fs::remove_file(&staged);
    replaced
}

/// Archiv und `sha256.sum` laden, Prüfsumme vergleichen, Binary entpacken.
fn fetch_binary(update: &Available) -> Result<Vec<u8>> {
    let agent = agent();
    let download = |url: &str| -> Result<Vec<u8>> {
        Ok(agent
            .get(url)
            .call()
            .with_context(|| format!("cannot download {url}"))?
            .body_mut()
            .read_to_vec()?)
    };
    let archive = download(&update.archive_url)?;
    let sums = String::from_utf8(download(&update.sums_url)?)?;
    let expected = expected_sum(&sums, &update.archive_name)
        .with_context(|| format!("sha256.sum lists no {}", update.archive_name))?;
    let actual = hex(&Sha256::digest(&archive));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!(
            "checksum mismatch for {} – update aborted",
            update.archive_name
        );
    }
    extract(&archive)
}

/// `claude-tui update` auf der Kommandozeile.
pub fn run_cli() -> Result<()> {
    println!("claude-tui {CURRENT}: checking for updates …");
    match check()? {
        None => println!("already up to date"),
        Some(update) => {
            println!("installing v{} …", update.version);
            install(&update)?;
            println!(
                "updated to v{} – takes effect on the next start",
                update.version
            );
        }
    }
    remember_check();
    Ok(())
}

/// Prüft beim Start im Hintergrund, höchstens einmal am Tag. Fehler (offline, Rate-Limit) bleiben
/// still — die Prüfung ist ein Hinweis, keine Pflicht.
pub fn spawn_check() -> Option<Receiver<Available>> {
    if std::env::var_os(OPT_OUT_ENV).is_some() || !check_due() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok(found) = check() {
            remember_check();
            if let Some(update) = found {
                let _ = tx.send(update);
            }
        }
    });
    Some(rx)
}

/// Installiert im Hintergrund; das Ergebnis kommt als Text für den Hinweis zurück.
pub fn spawn_install(update: Available) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = install(&update)
            .map(|()| format!("updated to v{} – restart claude-tui", update.version))
            .map_err(|e| format!("update failed: {e:#}"));
        let _ = tx.send(result);
    });
    rx
}

#[derive(Default, Serialize, Deserialize)]
struct CheckState {
    /// Sekunden seit 1970 der letzten erfolgreichen Prüfung.
    checked_at: u64,
}

fn check_file() -> Option<PathBuf> {
    paths::config_dir().map(|d| d.join(CHECK_FILE))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn check_due() -> bool {
    let last = check_file()
        .and_then(|f| fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<CheckState>(&t).ok())
        .map_or(0, |s| s.checked_at);
    now_secs().saturating_sub(last) >= CHECK_INTERVAL.as_secs()
}

fn remember_check() {
    let Some(file) = check_file() else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let state = CheckState {
        checked_at: now_secs(),
    };
    if let Ok(text) = serde_json::to_string(&state) {
        let _ = fs::write(file, text);
    }
}

/// `a.b.c` numerisch vergleichen; fehlende oder nicht numerische Teile zählen als 0.
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> [u64; 3] {
        let mut out = [0; 3];
        let core = v.split(['-', '+']).next().unwrap_or_default();
        for (slot, part) in out.iter_mut().zip(core.split('.')) {
            *slot = part.parse().unwrap_or(0);
        }
        out
    }
    parts(candidate) > parts(current)
}

/// Prüfsumme zu `name` aus einer `sha256sum`-Ausgabe (`<hex>  <name>`).
fn expected_sum<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then_some(hash.trim())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(windows)]
fn extract(archive: &[u8]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).context("damaged archive")?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let is_binary = file.name()?.rsplit('/').next() == Some(EXE_NAME);
        if is_binary {
            let mut binary = Vec::new();
            file.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("{EXE_NAME} not found in archive")
}

#[cfg(not(windows))]
fn extract(archive: &[u8]) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(archive)));
    for entry in tar.entries().context("damaged archive")? {
        let mut entry = entry?;
        if entry.path()?.file_name().is_some_and(|n| n == EXE_NAME) {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("{EXE_NAME} not found in archive")
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_is_numeric() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("0.2.0-beta", "0.2.0"));
    }

    #[test]
    fn checksum_line_is_found_by_file_name() {
        let sums = "aaa  claude-tui-v0.2.0-x86_64-pc-windows-msvc.zip\nbbb  claude-tui-v0.2.0-x86_64-unknown-linux-gnu.tar.gz\n";
        assert_eq!(
            expected_sum(sums, "claude-tui-v0.2.0-x86_64-unknown-linux-gnu.tar.gz"),
            Some("bbb")
        );
        assert_eq!(expected_sum(sums, "fehlt.zip"), None);
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
    }

    /// Gegen das echte GitHub-Release: abfragen, laden, Prüfsumme, entpacken — ohne zu ersetzen.
    /// Braucht Netz, daher nur auf Zuruf: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_and_verifies_published_release() {
        let update = check_against("0.0.0")
            .expect("Abfrage")
            .expect("Release vorhanden");
        let binary = fetch_binary(&update).expect("Download und Prüfsumme");
        assert!(
            binary.len() > 100_000,
            "Binary zu klein: {} Bytes",
            binary.len()
        );
        if cfg!(windows) {
            assert_eq!(&binary[..2], b"MZ", "keine Windows-Programmdatei");
        }
    }
}
