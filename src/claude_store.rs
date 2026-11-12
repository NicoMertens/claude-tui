//! Liest die gespeicherten Sessions von Claude Code (`~/.claude/projects/<Ordner>/<id>.jsonl`).
//!
//! Die Protokolle wachsen nur am Ende; je Datei wird deshalb nur das Neue seit dem letzten Lesen
//! ausgewertet. Gebraucht werden Arbeitsverzeichnis (`cwd`) und Titel: eigener Name
//! (`custom-title`, per `/rename`), sonst KI-Titel (`ai-title`), sonst der letzte Prompt.
//!
//! Der Lesestand je Datei (Offset und Ausgewertetes) liegt zusätzlich als `sessions-cache.json`
//! im Konfigurationsordner. Ein Neustart liest damit nur, was seit dem letzten Lauf dazukam,
//! statt jedes Mal alle Protokolle komplett.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

const CACHE_FILE: &str = "sessions-cache.json";
/// Bei einer Änderung am Format des Caches erhöhen; ein alter Cache wird dann verworfen.
const CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSession {
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    /// Sessionfarbe aus `/color` (z. B. `orange`); `None` = keine bzw. zurückgesetzt.
    pub color: Option<String>,
    pub modified: SystemTime,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Info {
    cwd: Option<PathBuf>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    last_prompt: Option<String>,
    color: Option<String>,
}

impl Info {
    fn title(&self) -> Option<&str> {
        self.custom_title
            .as_deref()
            .or(self.ai_title.as_deref())
            .or(self.last_prompt.as_deref())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FileState {
    id: String,
    /// Bis hierhin (Bytes) ist die Datei ausgewertet; danach höchstens eine unvollständige Zeile.
    offset: u64,
    modified: SystemTime,
    info: Info,
}

/// Nur die Felder, die hier gebraucht werden; alles andere ignoriert serde.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    cwd: Option<PathBuf>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    last_prompt: Option<String>,
    agent_color: Option<String>,
}

#[derive(Debug, Default)]
pub struct Store {
    root: Option<PathBuf>,
    files: HashMap<PathBuf, FileState>,
    /// Seit dem letzten Speichern des Caches hat sich etwas geändert.
    dirty: bool,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    root: Option<PathBuf>,
    files: Vec<(PathBuf, FileState)>,
}

impl Store {
    /// Leerer Store mit dem Stand aus dem Cache; noch ohne Blick auf die Dateien (`refresh`).
    pub fn from_cache() -> Self {
        let mut store = Self::with_root(paths::claude_projects_dir());
        if let Some(cache) = cache_file()
            .and_then(|f| fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok())
            .filter(|c| c.version == CACHE_VERSION && c.root == store.root)
        {
            store.files = cache.files.into_iter().collect();
        }
        store
    }

    fn with_root(root: Option<PathBuf>) -> Self {
        Self {
            root,
            files: HashMap::new(),
            dirty: false,
        }
    }

    /// Speichert den Lesestand, falls sich seit dem letzten Mal etwas geändert hat. Fehler zählen
    /// nicht: ohne Cache dauert nur der nächste Start länger.
    pub fn save_cache(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(file) = cache_file() else {
            return;
        };
        let cache = CacheRef {
            version: CACHE_VERSION,
            root: self.root.as_deref(),
            files: self.files.iter().collect(),
        };
        let Ok(bytes) = serde_json::to_vec(&cache) else {
            return;
        };
        // Erst temporär schreiben, dann umbenennen: ein Abbruch hinterlässt keinen halben Cache.
        let tmp = file.with_extension("json.tmp");
        if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, &file).is_ok() {
            self.dirty = false;
        }
    }

    /// Sucht neue und gewachsene Protokolle und wertet das Neue aus. Fehler beim Lesen einzelner
    /// Dateien werden übergangen: die Liste ist eine Anzeige, kein Muss.
    pub fn refresh(&mut self) {
        let Some(root) = self.root.as_deref() else {
            return;
        };
        let Ok(projects) = fs::read_dir(root) else {
            return;
        };
        let mut seen = Vec::new();
        for project in projects.flatten() {
            let Ok(files) = fs::read_dir(project.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    if let Ok(meta) = file.metadata() {
                        let changed =
                            update_file(&mut self.files, &path, meta.len(), meta.modified().ok());
                        self.dirty |= changed;
                        seen.push(path);
                    }
                }
            }
        }
        let seen: std::collections::HashSet<PathBuf> = seen.into_iter().collect();
        let before = self.files.len();
        self.files.retain(|path, _| seen.contains(path));
        self.dirty |= self.files.len() != before;
    }

    /// Sessions mit Arbeitsverzeichnis und Titel (leere Sessions ohne Prompt fallen weg).
    pub fn sessions(&self) -> Vec<StoredSession> {
        self.files.values().filter_map(stored).collect()
    }

    pub fn get(&self, id: &str) -> Option<StoredSession> {
        self.files.values().find(|f| f.id == id).and_then(stored)
    }

    /// Protokolldatei einer Session.
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        self.files
            .iter()
            .find(|(_, f)| f.id == id)
            .map(|(path, _)| path.clone())
    }
}

/// Löscht eine Session endgültig: die Protokolldatei `<id>.jsonl` und den gleichnamigen Ordner
/// daneben (Subagent-Protokolle u. Ä.). Unter Windows kann die Datei kurz nach dem Beenden des
/// Prozesses noch gesperrt sein, daher einige Versuche.
pub fn delete(jsonl: &Path) -> Result<()> {
    let mut attempt = 0;
    loop {
        match fs::remove_file(jsonl) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) if attempt < DELETE_ATTEMPTS => {
                attempt += 1;
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e).with_context(|| format!("cannot delete {}", jsonl.display())),
        }
    }
    let dir = jsonl.with_extension("");
    if dir.is_dir() {
        fs::remove_dir_all(&dir).with_context(|| format!("cannot delete {}", dir.display()))?;
    }
    Ok(())
}

const DELETE_ATTEMPTS: u32 = 10;

fn stored(file: &FileState) -> Option<StoredSession> {
    Some(StoredSession {
        id: file.id.clone(),
        cwd: file.info.cwd.clone()?,
        title: one_line(file.info.title()?),
        color: file.info.color.clone(),
        modified: file.modified,
    })
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wertet das Neue einer Datei aus; `true`, wenn sich am gespeicherten Stand etwas geändert hat.
fn update_file(
    files: &mut HashMap<PathBuf, FileState>,
    path: &Path,
    len: u64,
    modified: Option<SystemTime>,
) -> bool {
    let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let mut changed = false;
    let state = files.entry(path.to_path_buf()).or_insert_with(|| {
        changed = true;
        FileState {
            id: id.to_string(),
            offset: 0,
            modified: SystemTime::UNIX_EPOCH,
            info: Info::default(),
        }
    });
    if let Some(modified) = modified.filter(|m| *m != state.modified) {
        state.modified = modified;
        changed = true;
    }
    if len < state.offset {
        // Datei wurde neu geschrieben: von vorn.
        state.offset = 0;
        state.info = Info::default();
        changed = true;
    }
    if len == state.offset {
        return changed;
    }
    if let Ok(read) = read_from(path, state.offset) {
        let used = apply(&mut state.info, &read) as u64;
        state.offset += used;
        changed |= used > 0;
    }
    changed
}

/// Wie [`Cache`], nur geliehen — zum Schreiben ohne Kopie.
#[derive(Serialize)]
struct CacheRef<'a> {
    version: u32,
    root: Option<&'a Path>,
    files: Vec<(&'a PathBuf, &'a FileState)>,
}

fn cache_file() -> Option<PathBuf> {
    let dir = paths::config_dir()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join(CACHE_FILE))
}

fn read_from(path: &Path, offset: u64) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Wertet alle vollständigen Zeilen aus und liefert, wie viele Bytes verbraucht sind.
fn apply(info: &mut Info, bytes: &[u8]) -> usize {
    let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else {
        return 0;
    };
    for raw in bytes[..end].split(|&b| b == b'\n') {
        let Ok(text) = std::str::from_utf8(raw) else {
            continue;
        };
        // Billiger Vorfilter: die meisten Zeilen sind Nachrichten, die hier nichts beitragen.
        let wanted = (info.cwd.is_none() && text.contains("\"cwd\""))
            || text.contains("\"custom-title\"")
            || text.contains("\"ai-title\"")
            || text.contains("\"last-prompt\"")
            || text.contains("\"agent-color\"");
        if !wanted {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Line>(text) else {
            continue;
        };
        if info.cwd.is_none() {
            info.cwd = line.cwd;
        }
        match line.kind.as_deref() {
            Some("custom-title") => {
                info.custom_title = line.custom_title.or(info.custom_title.take())
            }
            Some("ai-title") => info.ai_title = line.ai_title.or(info.ai_title.take()),
            Some("last-prompt") => info.last_prompt = line.last_prompt.or(info.last_prompt.take()),
            // Die jeweils letzte Zeile gilt; „default“ setzt die Farbe zurück.
            Some("agent-color") => {
                info.color = line.agent_color.filter(|c| c != "default" && !c.is_empty())
            }
            _ => {}
        }
    }
    end + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_title_wins_over_ai_title_and_prompt() {
        let mut info = Info::default();
        let text = concat!(
            r#"{"type":"user","cwd":"S:\\private","message":"hallo"}"#,
            "\n",
            r#"{"type":"last-prompt","lastPrompt":"mach was"}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"KI-Titel"}"#,
            "\n",
        );
        apply(&mut info, text.as_bytes());
        assert_eq!(info.cwd, Some(PathBuf::from("S:\\private")));
        assert_eq!(info.title(), Some("KI-Titel"));

        apply(
            &mut info,
            b"{\"type\":\"custom-title\",\"customTitle\":\"eigener Name\"}\n",
        );
        assert_eq!(info.title(), Some("eigener Name"));
    }

    #[test]
    fn last_agent_color_wins_and_default_resets() {
        let mut info = Info::default();
        apply(
            &mut info,
            b"{\"type\":\"agent-color\",\"agentColor\":\"orange\"}\n",
        );
        assert_eq!(info.color.as_deref(), Some("orange"));
        apply(
            &mut info,
            b"{\"type\":\"agent-color\",\"agentColor\":\"blue\"}\n",
        );
        assert_eq!(info.color.as_deref(), Some("blue"));
        apply(
            &mut info,
            b"{\"type\":\"agent-color\",\"agentColor\":\"default\"}\n",
        );
        assert_eq!(info.color, None);
    }

    #[test]
    fn incomplete_last_line_is_kept_for_later() {
        let mut info = Info::default();
        let text = b"{\"type\":\"ai-title\",\"aiTitle\":\"A\"}\n{\"type\":\"ai-ti";
        let used = apply(&mut info, text);
        assert_eq!(used, text.iter().position(|&b| b == b'\n').unwrap_or(0) + 1);
        assert_eq!(info.ai_title.as_deref(), Some("A"));
    }

    #[test]
    fn store_reads_files_incrementally() {
        let root = std::env::temp_dir().join(format!("claude-tui-store-{}", std::process::id()));
        let project = root.join("S--demo");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).expect("Testordner");
        let file = project.join("abc.jsonl");
        fs::write(&file, "{\"type\":\"user\",\"cwd\":\"/demo\"}\n").expect("schreiben");

        let mut store = Store::with_root(Some(root.clone()));
        store.refresh();
        assert!(
            store.sessions().is_empty(),
            "ohne Titel/Prompt ausgeblendet"
        );

        let mut content = fs::read_to_string(&file).expect("lesen");
        content.push_str("{\"type\":\"last-prompt\",\"lastPrompt\":\"zeile\\neins\"}\n");
        fs::write(&file, content).expect("schreiben");
        store.refresh();
        let session = store.get("abc").expect("Session");
        assert_eq!(session.cwd, PathBuf::from("/demo"));
        assert_eq!(session.title, "zeile eins");

        let subdir = project.join("abc");
        fs::create_dir_all(subdir.join("subagents")).expect("Unterordner");
        let path = store.path_of("abc").expect("Pfad");
        delete(&path).expect("löschen");
        assert!(!file.exists() && !subdir.exists());
        store.refresh();
        assert!(store.get("abc").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cached_state_survives_serialization_and_reads_only_new_lines() {
        let root = std::env::temp_dir().join(format!("claude-tui-cache-{}", std::process::id()));
        let project = root.join("S--demo");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).expect("Testordner");
        let file = project.join("abc.jsonl");
        let first =
            "{\"type\":\"user\",\"cwd\":\"/demo\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"Alt\"}\n";
        fs::write(&file, first).expect("schreiben");

        let mut store = Store::with_root(Some(root.clone()));
        store.refresh();
        assert!(store.dirty);
        let cache = Cache {
            version: CACHE_VERSION,
            root: store.root.clone(),
            files: store.files.drain().collect(),
        };
        let json = serde_json::to_vec(&cache).expect("json");
        let cache: Cache = serde_json::from_slice(&json).expect("json");

        let mut again = Store::with_root(Some(root.clone()));
        again.files = cache.files.into_iter().collect();
        assert_eq!(again.get("abc").map(|s| s.title).as_deref(), Some("Alt"));
        // Unverändert: nichts zu tun, nichts zu speichern.
        again.refresh();
        assert!(!again.dirty);

        let mut content = fs::read_to_string(&file).expect("lesen");
        content.push_str("{\"type\":\"custom-title\",\"customTitle\":\"Neu\"}\n");
        fs::write(&file, content).expect("schreiben");
        again.refresh();
        assert!(again.dirty);
        assert_eq!(again.get("abc").map(|s| s.title).as_deref(), Some("Neu"));
        let _ = fs::remove_dir_all(root);
    }
}
