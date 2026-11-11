//! `claude-tui install` / `uninstall`: Binary an einen festen Ort kopieren und in den PATH bringen.
//!
//! - Windows: `%LOCALAPPDATA%\Programs\claude-tui`, Eintrag im **Benutzer**-PATH (keine Adminrechte).
//! - Linux/macOS: `~/.local/bin`; steht das nicht im PATH, gibt es einen Hinweis — Shell-Profile
//!   werden nicht angefasst.
//!
//! Einstellungen (`state.json`, zuletzt genutzte Verzeichnisse) bleiben beim Deinstallieren erhalten.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const EXE_NAME: &str = if cfg!(windows) {
    "claude-tui.exe"
} else {
    "claude-tui"
};

fn install_dir() -> Result<PathBuf> {
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
        Ok(PathBuf::from(base).join("Programs").join("claude-tui"))
    } else {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home).join(".local").join("bin"))
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub fn install() -> Result<()> {
    let dir = install_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let target = dir.join(EXE_NAME);
    let current = std::env::current_exe().context("cannot locate the running claude-tui")?;
    if !same_file(&current, &target) {
        fs::copy(&current, &target).with_context(|| {
            format!(
                "cannot copy to {} (is an installed claude-tui still running?)",
                target.display()
            )
        })?;
    }
    println!("installed: {}", target.display());
    path::add(&dir)?;
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let dir = install_dir()?;
    let target = dir.join(EXE_NAME);
    let current = std::env::current_exe().context("cannot locate the running claude-tui")?;
    if same_file(&current, &target) {
        // Die laufende Datei lässt sich unter Windows nicht direkt löschen.
        self_replace::self_delete().context("cannot remove the running claude-tui")?;
    } else if target.exists() {
        fs::remove_file(&target).with_context(|| format!("cannot remove {}", target.display()))?;
    }
    println!("removed: {}", target.display());
    #[cfg(windows)]
    {
        // Der Ordner gehört nur claude-tui; ist er leer, darf er weg.
        let _ = fs::remove_dir(&dir);
        path::remove(&dir)?;
    }
    println!("settings in the config directory were kept");
    Ok(())
}

/// Ordner in eine `;`-getrennte PATH-Liste aufnehmen bzw. daraus entfernen. `None` = nichts zu tun.
/// Vergleich ohne Groß-/Kleinschreibung und ohne abschließenden Backslash, wie Windows es sieht.
#[cfg_attr(not(windows), allow(dead_code))]
fn edit_path_list(list: &str, dir: &str, add: bool) -> Option<String> {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    let wanted = norm(dir);
    let parts: Vec<&str> = list.split(';').filter(|p| !p.trim().is_empty()).collect();
    let present = parts.iter().any(|p| norm(p) == wanted);
    match (add, present) {
        (true, false) => {
            let mut parts = parts;
            parts.push(dir);
            Some(parts.join(";"))
        }
        (false, true) => Some(
            parts
                .into_iter()
                .filter(|p| norm(p) != wanted)
                .collect::<Vec<_>>()
                .join(";"),
        ),
        _ => None,
    }
}

#[cfg(windows)]
mod path {
    //! Benutzer-PATH über PowerShell lesen und schreiben. Gelesen wird roh aus der Registry
    //! (`DoNotExpandEnvironmentNames`), geschrieben als `REG_EXPAND_SZ` — so bleiben Einträge wie
    //! `%USERPROFILE%\…` erhalten. Das kurze Setzen einer Hilfsvariable über .NET verschickt danach
    //! `WM_SETTINGCHANGE`, damit neu gestartete Terminals den PATH sofort sehen.

    use std::{path::Path, process::Command};

    use anyhow::{bail, Context, Result};

    const READ: &str = "[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')";
    const WRITE: &str =
        "$k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true); \
        $k.SetValue('Path', $env:CLAUDE_TUI_PATH, 'ExpandString'); \
        [Environment]::SetEnvironmentVariable('CLAUDE_TUI_REFRESH', '1', 'User'); \
        [Environment]::SetEnvironmentVariable('CLAUDE_TUI_REFRESH', $null, 'User')";

    fn powershell(script: &str, value: Option<&str>) -> Result<String> {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        if let Some(value) = value {
            // Über eine Umgebungsvariable, damit der Pfad nicht als PowerShell-Code gelesen wird.
            cmd.env("CLAUDE_TUI_PATH", value);
        }
        let out = cmd.output().context("cannot run powershell.exe")?;
        if !out.status.success() {
            bail!(
                "powershell failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn update(dir: &Path, add: bool) -> Result<bool> {
        let dir = dir.to_string_lossy();
        let current = powershell(READ, None)?;
        match super::edit_path_list(&current, &dir, add) {
            Some(new) => {
                powershell(WRITE, Some(&new))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn add(dir: &Path) -> Result<()> {
        if update(dir, true)? {
            println!("added to your PATH – open a new terminal, then run: claude-tui");
        } else {
            println!("already in your PATH");
        }
        Ok(())
    }

    pub fn remove(dir: &Path) -> Result<()> {
        if update(dir, false)? {
            println!("removed from your PATH");
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod path {
    use std::path::Path;

    use anyhow::Result;

    pub fn add(dir: &Path) -> Result<()> {
        let in_path = std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|entry| entry == dir));
        if in_path {
            println!("{} is in your PATH – run: claude-tui", dir.display());
        } else {
            println!(
                "{} is not in your PATH; add this line to your shell profile:\n  export PATH=\"{}:$PATH\"",
                dir.display(),
                dir.display()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_once_and_keeps_other_entries() {
        let list = r"C:\Windows;%USERPROFILE%\bin";
        let added = edit_path_list(list, r"C:\Apps\claude-tui", true).expect("neu");
        assert_eq!(added, r"C:\Windows;%USERPROFILE%\bin;C:\Apps\claude-tui");
        assert_eq!(edit_path_list(&added, r"c:\apps\CLAUDE-TUI\", true), None);
    }

    #[test]
    fn removes_matching_entry_only() {
        let list = r"C:\Windows;C:\Apps\claude-tui\;C:\Tools";
        assert_eq!(
            edit_path_list(list, r"C:\Apps\claude-tui", false).as_deref(),
            Some(r"C:\Windows;C:\Tools")
        );
        assert_eq!(edit_path_list(list, r"C:\Other", false), None);
    }

    #[test]
    fn empty_path_gets_single_entry() {
        assert_eq!(
            edit_path_list("", r"C:\Apps\claude-tui", true).as_deref(),
            Some(r"C:\Apps\claude-tui")
        );
    }
}
