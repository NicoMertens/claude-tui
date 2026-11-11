//! HTTPS-Abrufe für den Updater, ohne eigenen TLS-Stack in der Binary.
//!
//! 1. `curl` des Betriebssystems — unter Windows fest `%SystemRoot%\System32\curl.exe` (seit
//!    Windows 10 1803 vorhanden), damit kein fremdes `curl.exe` aus dem PATH oder dem aktuellen
//!    Ordner untergeschoben werden kann; sonst `curl` aus dem PATH. Nutzt Zertifikate und
//!    Proxy-Einstellungen des Systems.
//! 2. Fehlt `curl`: unter Windows und macOS `ureq` mit dem TLS des Betriebssystems (SChannel bzw.
//!    Security-Framework). Unter Linux gibt es keinen Rückfall — der würde die Binary an OpenSSL
//!    binden, und `curl` ist dort praktisch immer installiert.
//!
//! Die Echtheit der Dateien sichert ohnehin die SHA-256-Prüfung im Updater.

use std::{io::ErrorKind, path::PathBuf, process::Command};

use anyhow::{bail, Result};

const USER_AGENT: &str = concat!("claude-tui/", env!("CARGO_PKG_VERSION"));
/// Obergrenze je Abruf in Sekunden.
const TIMEOUT_SECS: u64 = 60;

/// Was abgerufen wird: GitHub-API (JSON) oder eine Datei.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Api,
    File,
}

pub fn get(url: &str, kind: Kind) -> Result<Vec<u8>> {
    match curl(url, kind) {
        Ok(body) => Ok(body),
        Err(CurlError::Missing) => fallback(url, kind),
        Err(CurlError::Failed(message)) => bail!("cannot download {url}: {message}"),
    }
}

enum CurlError {
    /// `curl` ist nicht installiert.
    Missing,
    Failed(String),
}

fn curl_program() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("curl.exe")
    } else {
        PathBuf::from("curl")
    }
}

fn curl(url: &str, kind: Kind) -> Result<Vec<u8>, CurlError> {
    let mut cmd = Command::new(curl_program());
    // -f: HTTP-Fehler als Fehler, -sS: still außer Fehlern, -L: Weiterleitungen (Release-Assets).
    cmd.args(["-fsSL", "--proto", "=https", "--max-time"])
        .arg(TIMEOUT_SECS.to_string())
        .args(["-A", USER_AGENT]);
    if kind == Kind::Api {
        cmd.args(["-H", "Accept: application/vnd.github+json"]);
    }
    cmd.arg(url);
    let output = match cmd.output() {
        Ok(output) => output,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(CurlError::Missing),
        Err(e) => return Err(CurlError::Failed(e.to_string())),
    };
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(CurlError::Failed(if stderr.is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr
        }))
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn fallback(url: &str, kind: Kind) -> Result<Vec<u8>> {
    use std::time::Duration;

    use anyhow::Context;
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    let tls = TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(TIMEOUT_SECS)))
        .user_agent(USER_AGENT)
        .build()
        .into();
    let mut request = agent.get(url);
    if kind == Kind::Api {
        request = request.header("Accept", "application/vnd.github+json");
    }
    let body = request
        .call()
        .with_context(|| format!("cannot download {url}"))?
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()?;
    Ok(body)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn fallback(_url: &str, _kind: Kind) -> Result<Vec<u8>> {
    bail!("curl is not installed – install it to use claude-tui update")
}

#[cfg(test)]
mod tests {
    use super::*;

    const API: &str = "https://api.github.com/repos/NicoMertens/claude-tui/releases/latest";

    /// Braucht Netz: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn curl_reads_github_api() {
        let body = curl(API, Kind::Api).unwrap_or_else(|_| panic!("curl fehlgeschlagen"));
        assert!(String::from_utf8_lossy(&body).contains("tag_name"));
    }

    /// Rückfall ohne `curl` (System-TLS); braucht Netz.
    #[test]
    #[ignore]
    #[cfg(any(windows, target_os = "macos"))]
    fn native_tls_fallback_reads_github_api() {
        let body = fallback(API, Kind::Api).expect("System-TLS");
        assert!(String::from_utf8_lossy(&body).contains("tag_name"));
    }

    #[test]
    fn windows_curl_is_taken_from_system32() {
        if cfg!(windows) {
            let program = curl_program();
            assert!(
                program.ends_with("System32\\curl.exe"),
                "{}",
                program.display()
            );
        }
    }
}
