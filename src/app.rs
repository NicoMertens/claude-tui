use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context as _, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Margin, Position, Rect};

use crate::{
    claude_store::{self, Store},
    dirpick::{DirPicker, Outcome as DirOutcome, Target},
    external,
    grouppick::{GroupPicker, Outcome as GroupOutcome},
    groups::{self, Assignments},
    hooks,
    keys::{self, TagKey},
    layout::{self, Direction},
    list::{self, Entry, Group, Running, NEW_SESSION_TITLE},
    menu::{self, Action, Context, Menu, Outcome, Purpose},
    mouse, recent,
    selection::{self, Cell, Selection},
    session::{self, Session, Status},
    state::{self, State},
    tags::{self, Tags, Toggled},
    trace, ui,
    update::{self, Available},
    waiting,
    windows::{Focus, SessionId, Windows, MAX_WINDOWS},
};

/// So oft wird `~/.claude/projects` nach neuen Sessions und Titeln durchsucht.
const STORE_INTERVAL: Duration = Duration::from_secs(2);
/// Höchstens so oft wird der Lesestand der Protokolle gespeichert (außerdem beim Beenden).
const CACHE_INTERVAL: Duration = Duration::from_secs(60);
/// Beim Wiederherstellen startet die nächste Session, sobald die vorige etwas anzeigt, spätestens
/// nach dieser Zeit — mehrere Claude-Prozesse gleichzeitig bremsen sich gegenseitig aus.
const STAGGER_MAX: Duration = Duration::from_secs(3);
/// Zweiter Klick auf dieselbe Zelle innerhalb dieser Zeit = Doppelklick (Wort markieren).
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// So lange bleiben befristete Hinweise (z. B. „copied N characters“) stehen.
const FLASH_DURATION: Duration = Duration::from_secs(5);
/// So lange nach dem Ende eines eigenen Prozesses gilt seine Session noch als eigene: Datei in
/// `~/.claude/sessions` und letzte Schreibzugriffe stammen noch von ihm.
const ENDED_GRACE: Duration = Duration::from_secs(30);
/// Längere Sessiontitel im Kopf des Gruppen-Dialogs werden gekürzt.
const GROUP_TITLE_MAX: usize = 30;
/// Ohne Anlass wird die Sessionliste höchstens so oft neu gebaut.
const LIST_INTERVAL: Duration = Duration::from_secs(1);

/// Sichtbare Zeilen der Sessionliste aus dem letzten Zeichnen: Fläche und (Bildschirmzeile,
/// Claude-ID) je Session. Gruppenüberschriften fehlen absichtlich.
#[derive(Debug, Default)]
pub struct ListHits {
    pub area: Rect,
    pub rows: Vec<(u16, String)>,
}

impl ListHits {
    fn at(&self, position: Position) -> Option<&str> {
        if !self.area.contains(position) {
            return None;
        }
        self.rows
            .iter()
            .find(|(y, _)| *y == position.y)
            .map(|(_, id)| id.as_str())
    }
}

/// Wie ein Claude-Prozess gestartet wird.
enum Launch {
    /// Neue Session mit vorgegebener ID (`--session-id`), damit sie sich später fortsetzen lässt.
    New,
    /// Gespeicherte Session fortsetzen (`--resume`).
    Resume(String),
    /// Laufende Hintergrund-Session (Claude-ID, Job-ID) öffnen (`claude attach`); `--resume`
    /// lehnt sie ab, solange sie läuft.
    Attach(String, String),
}

pub struct App {
    /// Laufende Claude-Prozesse.
    pub sessions: Vec<Session>,
    pub windows: Windows,
    /// Sessionliste: gespeicherte und laufende Sessions, gruppiert nach Verzeichnis.
    pub groups: Vec<Group>,
    /// Claude-ID der in der Liste ausgewählten Session.
    pub cursor: Option<String>,
    /// In der Liste markierte Sessions (Leertaste); `x` löscht sie gemeinsam.
    pub marked: HashSet<String>,
    /// Schnellwahl-Nummern (Alt+Zahl vergibt, Ctrl+Zahl zeigt); bleiben über `state.json` erhalten.
    pub tags: Tags,
    /// Eigene Gruppen je Claude-ID; bleiben über `state.json` erhalten.
    pub group_of: Assignments,
    /// Zuletzt verwendete Gruppennamen, neueste zuerst.
    recent_groups: Vec<String>,
    /// Gruppen-Dialog (Ctrl+A g).
    pub group_picker: Option<GroupPicker>,
    pub menu: Option<Menu>,
    /// Verzeichnisauswahl vor dem Start einer neuen Session.
    pub dir_picker: Option<DirPicker>,
    /// Hinweis unten in der Sessionliste (z. B. Startfehler); verschwindet mit der nächsten Taste,
    /// befristete Hinweise zusätzlich nach [`FLASH_DURATION`].
    pub notice: Option<String>,
    /// Befristeter Hinweis samt Ablaufzeit; `tick` entfernt ihn danach, sofern `notice` noch
    /// denselben Text zeigt.
    notice_expiry: Option<(String, Instant)>,
    /// Mausmarkierung in einem Fenster; bleibt sichtbar bis zur nächsten Taste oder zum nächsten
    /// Klick.
    pub selection: Option<Selection>,
    dragging: bool,
    /// Laufender Klick/Zug, der an das Programm der Session geht (es hat die Maus eingeschaltet);
    /// bleibt bis zum Loslassen an diese Session gebunden.
    forwarding: Option<(SessionId, mouse::Button)>,
    last_click: Option<(Instant, SessionId, Cell)>,
    /// Letzter Klick in die Sessionliste (für Doppelklick = Enter).
    last_list_click: Option<(Instant, String)>,
    /// Beim Zeichnen gemerkt: welche Bildschirmzeile der Liste welche Session zeigt.
    pub list_hits: RefCell<ListHits>,
    /// Beim Zeichnen gemerkt: Fläche des Menü-Overlays (`None`, wenn keins offen ist).
    pub overlay: std::cell::Cell<Option<Rect>>,
    /// Neuere Version auf GitHub, sobald die Prüfung beim Start eine gefunden hat.
    pub update: Option<Available>,
    update_check: Option<Receiver<Available>>,
    update_install: Option<Receiver<Result<String, String>>>,
    pub should_quit: bool,
    /// Eine Session wartet neu auf dich: einmal die Terminal-Glocke läuten (`main.rs`).
    pub ring_bell: bool,
    command: String,
    /// Verzeichnis, aus dem claude-tui gestartet wurde.
    start_dir: PathBuf,
    /// Zuletzt genutzte Startverzeichnisse, neueste zuerst.
    recent: Vec<PathBuf>,
    store: Store,
    /// Hintergrund-Thread, der die Protokolle beim Start einliest; bis dahin gilt der Cache.
    store_load: Option<Receiver<Store>>,
    /// Fensterlayout, das erst nach dem Einlesen wiederhergestellt werden kann (Sessions fehlen
    /// im Cache).
    pending_restore: Option<State>,
    last_scan: Instant,
    last_cache_save: Instant,
    /// Die Sessionliste muss neu gebaut werden (Sessions kamen oder gingen, Protokolle änderten
    /// sich); sonst nur alle [`LIST_INTERVAL`].
    list_dirty: bool,
    last_list: Instant,
    /// Wiederhergestellte Sessions, deren Prozess noch nicht läuft, in Startreihenfolge.
    start_queue: VecDeque<SessionId>,
    /// Zuletzt gestartete Session aus der Reihe und Startzeit (siehe [STAGGER_MAX]).
    starting: Option<(SessionId, Instant)>,
    /// Laufende Claude-Prozesse laut `~/.claude/sessions` (eigene eingeschlossen).
    claimed: external::Claimed,
    claimed_watch: Option<Receiver<external::Snapshot>>,
    /// Eigene Sessions, die Claude Code als Hintergrund-Job mit neuer ID weiterführt: alte ID →
    /// neue. Das alte Protokoll bleibt aus der Liste (siehe [`Self::follow_parked`]); gespeichert
    /// in `state.json`, sonst käme es nach einem Neustart als Doublette zurück.
    superseded: BTreeMap<String, String>,
    /// Zuletzt in claude-tui beendete Sessions mit Zeitpunkt (siehe [ENDED_GRACE]).
    ended: Vec<(String, Instant)>,
    /// Start von claude-tui; ältere Schreibzugriffe zählen nicht als fremde Aktivität.
    started: SystemTime,
    /// Ganzer Bildschirm; die Fensterfläche ergibt sich daraus und aus `list_hidden`.
    screen: Rect,
    /// Sessionliste ausgeblendet: die Fenster bekommen die volle Breite. Nur mit mindestens einem
    /// Fenster und nie bei Fokus auf der Liste.
    pub list_hidden: bool,
    /// Zwei Fenster übereinander statt nebeneinander (gilt nur bei genau zwei Fenstern).
    stacked: bool,
    next_id: SessionId,
}

impl App {
    pub fn new(command: String, screen: Rect) -> Result<Self> {
        trace::mark("app: start");
        let store = Store::from_cache();
        trace::mark("app: cache loaded");
        let store_load = spawn_store_load();
        hooks::start_server();
        crate::paths::restrict_config_dir();
        hooks::purge_stale();
        let mut app = Self {
            sessions: Vec::new(),
            windows: Windows::default(),
            groups: Vec::new(),
            cursor: None,
            marked: HashSet::new(),
            tags: Tags::new(),
            group_of: Assignments::new(),
            recent_groups: Vec::new(),
            group_picker: None,
            menu: None,
            dir_picker: None,
            notice: None,
            notice_expiry: None,
            selection: None,
            dragging: false,
            forwarding: None,
            last_click: None,
            last_list_click: None,
            list_hits: RefCell::new(ListHits::default()),
            overlay: std::cell::Cell::new(None),
            update: None,
            update_check: update::spawn_check(),
            update_install: None,
            should_quit: false,
            ring_bell: false,
            command,
            start_dir: std::env::current_dir().context("current directory not readable")?,
            recent: recent::load(),
            store,
            store_load,
            pending_restore: None,
            last_scan: Instant::now(),
            last_cache_save: Instant::now(),
            list_dirty: true,
            last_list: Instant::now(),
            start_queue: VecDeque::new(),
            starting: None,
            claimed: external::Claimed::new(),
            claimed_watch: external::spawn_watcher(),
            superseded: BTreeMap::new(),
            ended: Vec::new(),
            started: SystemTime::now(),
            screen,
            list_hidden: false,
            stacked: false,
            next_id: 0,
        };
        let mut saved = state::load();
        // Vor dem Wiederherstellen übernehmen: jedes `place` speichert den Zustand neu.
        app.tags = std::mem::take(&mut saved.tags);
        app.group_of = std::mem::take(&mut saved.groups);
        app.recent_groups = std::mem::take(&mut saved.recent_groups);
        app.superseded = std::mem::take(&mut saved.superseded);
        // Fehlt eine der Sessions im Cache (erster Start, neue Session), erst nach dem Einlesen.
        let known = saved.windows.iter().all(|id| app.store.get(id).is_some());
        if known || app.store_load.is_none() {
            app.restore(saved)?;
        } else {
            trace::mark("app: restore waits for sessions");
            app.pending_restore = Some(saved);
        }
        app.rebuild_list();
        trace::mark("app: ready");
        Ok(app)
    }

    /// Stellt das Fensterlayout des letzten Laufs wieder her; jede Session darin wird fortgesetzt.
    /// Gibt es nichts wiederherzustellen, startet eine Session im Startverzeichnis.
    fn restore(&mut self, saved: State) -> Result<()> {
        self.restore_layout(saved);
        if self.windows.len() == 0 {
            let start_dir = self.start_dir.clone();
            let id = self.spawn(&start_dir, Launch::New)?;
            self.place(id, 0);
        }
        Ok(())
    }

    /// Fenster anlegen; die Prozesse starten gestaffelt (siehe [`Self::advance_starts`]), das
    /// Fenster mit Fokus zuerst.
    fn restore_layout(&mut self, saved: State) {
        trace::mark(format_args!("app: restore {} windows", saved.windows.len()));
        self.stacked = saved.stacked;
        let now = external::claimed_now(&saved.windows);
        let elsewhere = &now.claimed;
        for claude_id in saved.windows {
            // Inzwischen von Claude Code als Hintergrund-Job weitergeführt: den Job öffnen.
            let claude_id = match now.parked.get(&claude_id) {
                Some(new) => {
                    self.superseded.insert(claude_id, new.clone());
                    new.clone()
                }
                None => claude_id,
            };
            let Some(stored) = self.store.get(&claude_id) else {
                continue;
            };
            let launch = match elsewhere.get(&claude_id) {
                None => Launch::Resume(claude_id),
                Some(Some(job)) => Launch::Attach(claude_id, job.clone()),
                Some(None) => {
                    self.notice = Some(format!(
                        "\"{}\" runs elsewhere – not reopened",
                        stored.title
                    ));
                    continue;
                }
            };
            match self.spawn_with(&stored.cwd, launch, false) {
                Ok(id) => self.place(id, self.windows.len()),
                Err(e) => self.notice = Some(format!("{e:#}")),
            }
        }
        let focus = match saved.focus {
            Some(slot) if slot < self.windows.len() => Focus::Window(slot),
            _ if self.windows.len() > 0 => Focus::Window(0),
            _ => Focus::List,
        };
        self.set_focus(focus);
        self.set_list_hidden(saved.list_hidden);
        let mut order: Vec<SessionId> = self.windows.slots().to_vec();
        if let Some(first) = self.windows.focused_session() {
            order.retain(|&id| id != first);
            order.insert(0, first);
        }
        self.start_queue = order.into();
        self.advance_starts();
    }

    /// Startet die nächste wartende Session, sobald die vorige etwas anzeigt oder [STAGGER_MAX]
    /// verstrichen ist. Sessions, die per Eingabe schon gestartet wurden, werden übersprungen.
    fn advance_starts(&mut self) {
        if let Some((id, at)) = self.starting {
            let ready = self.session(id).is_none_or(Session::has_content);
            if !ready && at.elapsed() < STAGGER_MAX {
                return;
            }
            self.starting = None;
        }
        while let Some(id) = self.start_queue.pop_front() {
            let Some(session) = self.session_mut(id) else {
                continue;
            };
            if !session.is_pending() {
                continue;
            }
            match session.start() {
                Ok(()) => {
                    self.starting = Some((id, Instant::now()));
                    return;
                }
                Err(e) => self.notice = Some(format!("{e:#}")),
            }
        }
    }

    /// Lesestand der Protokolle sichern (beim Beenden).
    pub fn save_cache(&mut self) {
        self.store.save_cache();
    }

    /// Ergebnis des Einlesens beim Start übernehmen und ein aufgeschobenes Layout wiederherstellen.
    fn poll_store_load(&mut self) -> Result<()> {
        let Some(store) = self.store_load.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return Ok(());
        };
        trace::mark("app: sessions loaded");
        self.store_load = None;
        self.store = store;
        self.list_dirty = true;
        self.store.save_cache();
        self.last_scan = Instant::now();
        self.last_cache_save = Instant::now();
        if let Some(saved) = self.pending_restore.take() {
            self.restore(saved)?;
        }
        Ok(())
    }

    /// Die Protokolle werden beim Start noch eingelesen.
    pub fn loading(&self) -> bool {
        self.store_load.is_some()
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// Titel einer laufenden Session, wie ihn die Sessionliste zeigt.
    pub fn title_of(&self, id: SessionId) -> String {
        list::flatten(&self.groups)
            .find(|e| e.running == Some(id))
            .map_or_else(|| NEW_SESSION_TITLE.to_string(), |e| e.title.clone())
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        let cursor = self.cursor.as_deref()?;
        list::flatten(&self.groups).find(|e| e.claude_id == cursor)
    }

    fn selected_running(&self) -> Option<SessionId> {
        self.selected_entry().and_then(|e| e.running)
    }

    /// Sichtbare Fenster als (Fensterindex, Session, Rechteck) in Nummernreihenfolge; gezoomt nur
    /// das gezoomte Fenster über die ganze Fensterfläche.
    pub fn window_rects(&self) -> Vec<(usize, SessionId, Rect)> {
        layout::visible(
            ui::windows_area(self.screen, self.list_hidden),
            self.windows.len(),
            self.stacked,
            self.windows.zoomed(),
        )
        .into_iter()
        .filter_map(|(slot, rect)| {
            let id = *self.windows.slots().get(slot)?;
            Some((slot, id, rect))
        })
        .collect()
    }

    /// Rechteck von Fenster `slot`, falls es sichtbar ist.
    pub fn window_rect(&self, slot: usize) -> Option<Rect> {
        self.window_rects()
            .into_iter()
            .find(|&(s, _, _)| s == slot)
            .map(|(_, _, rect)| rect)
    }

    /// Hintergrundarbeit je Durchlauf der Hauptschleife. Liefert, ob sich etwas Sichtbares geändert
    /// hat (Zustände, Hinweise, Liste, Update) — dann muss neu gezeichnet werden. Ausgabe der
    /// Sessions zählt hier nicht, die prüft die Hauptschleife über [`Self::latest_output`].
    pub fn tick(&mut self) -> bool {
        let notice_before = self.notice.clone();
        let update_before = (
            self.update.as_ref().map(|u| u.version.clone()),
            self.update_install.is_some(),
            self.loading(),
        );
        let mut changes = Vec::new();
        for session in &mut self.sessions {
            let before = session.status;
            session.refresh();
            changes.push((session.id, before, session.status));
        }
        let statuses_changed = changes.iter().any(|&(_, before, after)| before != after);
        for id in waiting::newly_waiting(changes, self.windows.focused_session()) {
            self.notice = Some(format!("\"{}\" is waiting for you", self.title_of(id)));
            self.ring_bell = waiting::bell_enabled();
        }
        // Beendete Prozesse verschwinden; die Claude-Session bleibt in der Liste.
        let exited: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|s| s.status == Status::Exited)
            .map(|s| s.id)
            .collect();
        for id in exited {
            if self.windows.window_of(id).is_some() {
                let title = self.title_of(id);
                let last = self.session(id).and_then(Session::last_line);
                self.notice = Some(match last {
                    Some(line) => format!("\"{title}\" has exited: {line}"),
                    None => format!("\"{title}\" has exited"),
                });
            }
            self.remove_process(id);
        }
        self.expire_notice();
        self.poll_update();
        if let Err(e) = self.poll_store_load() {
            self.notice = Some(format!("{e:#}"));
        }
        self.advance_starts();
        if let Some(snapshot) = self
            .claimed_watch
            .as_ref()
            .and_then(|rx| rx.try_iter().last())
        {
            self.follow_parked(&snapshot.parked);
            if snapshot.claimed != self.claimed {
                self.claimed = snapshot.claimed;
                self.list_dirty = true;
            }
        }
        // Während des Einlesens beim Start nicht doppelt lesen.
        if self.store_load.is_none() && self.last_scan.elapsed() >= STORE_INTERVAL {
            self.list_dirty |= self.store.refresh();
            self.last_scan = Instant::now();
            if self.last_cache_save.elapsed() >= CACHE_INTERVAL {
                self.store.save_cache();
                self.last_cache_save = Instant::now();
            }
        }
        // Die Liste hängt auch von der Uhr ab (`ENDED_GRACE`, `external::RECENT_WRITE`); daher
        // außer nach Änderungen auch regelmäßig neu bauen — aber nicht in jedem Durchlauf.
        let list_changed =
            (self.list_dirty || self.last_list.elapsed() >= LIST_INTERVAL) && self.rebuild_list();
        let update_after = (
            self.update.as_ref().map(|u| u.version.clone()),
            self.update_install.is_some(),
            self.loading(),
        );
        statuses_changed
            || list_changed
            || self.notice != notice_before
            || update_after != update_before
    }

    /// Jüngste Ausgabe irgendeiner Session — die Hauptschleife zeichnet neu, wenn sie nach dem
    /// letzten Bild liegt.
    pub fn latest_output(&self) -> Option<Instant> {
        self.sessions.iter().map(Session::last_output).max()
    }

    /// Etwas auf dem Bildschirm bewegt sich von selbst: Spinner einer startenden oder arbeitenden
    /// Session, „loading sessions …“.
    pub fn animating(&self) -> bool {
        self.loading()
            || self
                .sessions
                .iter()
                .any(|s| matches!(s.status, Status::Starting | Status::Working))
    }

    /// Ergebnisse der Update-Prüfung bzw. -Installation aus den Hintergrund-Threads abholen.
    fn poll_update(&mut self) {
        if let Some(found) = self.update_check.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.update = Some(found);
            self.update_check = None;
        }
        if let Some(result) = self
            .update_install
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.update_install = None;
            match result {
                Ok(message) => {
                    self.update = None;
                    self.notice = Some(message);
                }
                Err(message) => self.notice = Some(message),
            }
        }
    }

    /// Claude Code hat eine eigene Session als Hintergrund-Job mit neuer ID weitergeführt (das
    /// Fenster zeigt jetzt diesen Job): Die Session übernimmt die neue ID samt Schnellwahl-Nummer,
    /// Gruppe, Auswahl und Markierung — sonst stünde sie doppelt in der Liste, einmal als „ext“.
    fn follow_parked(&mut self, parked: &HashMap<String, String>) {
        let mut changed = false;
        for session in &mut self.sessions {
            let Some(new) = parked.get(&session.claude_id) else {
                continue;
            };
            let old = std::mem::replace(&mut session.claude_id, new.clone());
            trace::mark(format_args!("session {old}: parked as {new}"));
            for id in self.tags.values_mut() {
                if *id == old {
                    id.clone_from(new);
                }
            }
            if let Some(group) = self.group_of.remove(&old) {
                self.group_of.insert(new.clone(), group);
            }
            if self.cursor.as_deref() == Some(old.as_str()) {
                self.cursor = Some(new.clone());
            }
            if self.marked.remove(&old) {
                self.marked.insert(new.clone());
            }
            self.superseded.insert(old, new.clone());
            changed = true;
        }
        if changed {
            self.list_dirty = true;
            self.save_state();
        }
    }

    /// Baut die Sessionliste neu; `true`, wenn sich an Liste, Cursor oder Markierung etwas
    /// geändert hat.
    fn rebuild_list(&mut self) -> bool {
        let running: Vec<Running> = self
            .sessions
            .iter()
            .map(|s| Running {
                id: s.id,
                claude_id: s.claude_id.clone(),
                cwd: s.cwd.clone(),
            })
            .collect();
        self.ended.retain(|(_, at)| at.elapsed() < ENDED_GRACE);
        let mut stored = self.store.sessions();
        // Das Protokoll vor der Verlegung ist eine überholte Kopie der laufenden Session.
        stored.retain(|s| !self.superseded.contains_key(&s.id));
        let own: HashSet<&str> = running
            .iter()
            .map(|r| r.claude_id.as_str())
            .chain(self.ended.iter().map(|(id, _)| id.as_str()))
            .collect();
        let elsewhere = external::running_elsewhere(
            stored.iter().map(|s| (s.id.as_str(), s.modified)),
            &self.claimed,
            &own,
            self.started,
            SystemTime::now(),
        );
        let old = std::mem::replace(
            &mut self.groups,
            list::build(stored, &running, &elsewhere, &self.group_of),
        );
        self.list_dirty = false;
        self.last_list = Instant::now();
        let cursor_before = self.cursor.clone();
        let marked_before = self.marked.len();
        let listed: HashSet<&str> = list::flatten(&self.groups)
            .map(|e| e.claude_id.as_str())
            .collect();
        self.marked.retain(|id| listed.contains(id.as_str()));
        if self.selected_entry().is_none() {
            let still: HashSet<&str> = list::flatten(&self.groups)
                .map(|e| e.claude_id.as_str())
                .collect();
            let before: Vec<&str> = list::flatten(&old).map(|e| e.claude_id.as_str()).collect();
            let successor = self
                .cursor
                .as_deref()
                .and_then(|gone| list::successor(&before, gone, &still));
            self.cursor = successor
                .or_else(|| {
                    list::flatten(&self.groups)
                        .next()
                        .map(|e| e.claude_id.as_str())
                })
                .map(str::to_string);
        }
        old != self.groups || self.cursor != cursor_before || self.marked.len() != marked_before
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        self.notice = None;
        if let Err(e) = self.handle_key(key) {
            self.notice = Some(format!("{e:#}"));
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        self.selection = None;
        if let Some(direction) = keys::focus_direction(&key) {
            self.close_overlays();
            self.move_focus(direction);
            return Ok(());
        }
        if let Some(picker) = self.dir_picker.as_mut() {
            match picker.on_key(&key) {
                DirOutcome::Stay => {}
                DirOutcome::Cancel => self.dir_picker = None,
                DirOutcome::Chosen(dir) => {
                    let target = picker.target;
                    self.dir_picker = None;
                    self.on_dir_chosen(dir, target)?;
                }
            }
            return Ok(());
        }
        if let Some(picker) = self.group_picker.as_mut() {
            match picker.on_key(&key) {
                GroupOutcome::Stay => {}
                GroupOutcome::Cancel => self.group_picker = None,
                GroupOutcome::Chosen(group) => {
                    let targets = std::mem::take(&mut picker.targets);
                    self.group_picker = None;
                    self.apply_group(&targets, group);
                }
            }
            return Ok(());
        }
        if let Some(menu) = self.menu.as_mut() {
            match menu.on_key(&key) {
                Outcome::Stay => {}
                Outcome::Close => self.menu = None,
                Outcome::Run(action) => {
                    let purpose = self.menu.take().and_then(|m| m.purpose);
                    self.run(action, purpose)?;
                }
            }
            return Ok(());
        }
        if keys::is_prefix(&key) {
            self.open_commands();
            return Ok(());
        }
        // Wie Ctrl+Pfeil erreichen Alt+Zahl und Ctrl+Zahl die Session nie.
        if let Some(tag_key) = keys::tag_key(&key) {
            return match tag_key {
                TagKey::Toggle(number) => {
                    self.toggle_tag(number);
                    Ok(())
                }
                TagKey::Show(number) => self.show_tagged(number),
            };
        }
        match self.windows.focused_session() {
            Some(id) => {
                if let Some(session) = self.session_mut(id) {
                    if let Some(bytes) = keys::to_bytes(&key, session.application_cursor()) {
                        session.write(&bytes)?;
                    }
                }
            }
            None => self.on_list_key(key)?,
        }
        Ok(())
    }

    pub fn on_paste(&mut self, text: &str) -> Result<()> {
        if let Some(picker) = self.dir_picker.as_mut() {
            picker.paste(text);
            return Ok(());
        }
        if let Some(picker) = self.group_picker.as_mut() {
            picker.paste(text);
            return Ok(());
        }
        if let Some(id) = self.windows.focused_session() {
            if let Some(session) = self.session_mut(id) {
                if let Err(e) = session.paste(text) {
                    self.notice = Some(format!("{e:#}"));
                }
            }
        }
        Ok(())
    }

    /// Maus über einem Fenster: Rad scrollt den Verlauf, linke Taste markiert (Ziehen, Doppelklick
    /// = Wort) und kopiert beim Loslassen in die Zwischenablage.
    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        let position = Position::new(mouse.column, mouse.row);
        // Das Overlay ist nur per Tastatur bedienbar; Klicks und Rad darauf erreichen das Fenster
        // darunter nicht (keine Markierung, kein Scrollen hinter dem Menü).
        if self.on_overlay(position) && !matches!(mouse.kind, MouseEventKind::Up(_)) {
            return;
        }
        if self.forward_mouse(mouse) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some((_, id, cell)) = self.window_at(position) else {
                    return;
                };
                // Die Markierung bezieht sich auf den sichtbaren Ausschnitt; Scrollen hebt sie auf.
                self.selection = None;
                let up = mouse.kind == MouseEventKind::ScrollUp;
                if let Some(session) = self.session_mut(id) {
                    if let Err(e) = session.wheel(up, cell.col, cell.row) {
                        self.notice = Some(format!("{e:#}"));
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let list_hit = self.list_hits.borrow().at(position).map(str::to_string);
                if let Some(claude_id) = list_hit {
                    self.on_list_click(claude_id);
                    return;
                }
                let Some((slot, id, cell)) = self.window_at(position) else {
                    return;
                };
                self.close_overlays();
                self.set_focus(Focus::Window(slot));
                let double = self.last_click.is_some_and(|(at, last_id, last_cell)| {
                    at.elapsed() < DOUBLE_CLICK && last_id == id && last_cell == cell
                });
                self.last_click = Some((Instant::now(), id, cell));
                if double {
                    let word = self
                        .session(id)
                        .and_then(|s| selection::word_at(id, s.screen().screen(), cell));
                    self.selection = word;
                    self.dragging = false;
                    self.copy_selection();
                } else {
                    self.selection = Some(Selection::new(id, cell));
                    self.dragging = true;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                let Some(session) = self.selection.map(|s| s.session) else {
                    return;
                };
                // Auch außerhalb des Fensters weiterziehen, aber auf dessen Inhalt begrenzt.
                let inner = self
                    .windows
                    .window_of(session)
                    .and_then(|slot| self.window_rect(slot))
                    .map(|r| r.inner(Margin::new(1, 1)));
                if let (Some(inner), Some(selection)) = (inner, self.selection.as_mut()) {
                    selection.head = cell_in(inner, position);
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging => {
                self.dragging = false;
                if self.selection.is_some_and(|s| s.is_empty()) {
                    self.selection = None;
                } else {
                    self.copy_selection();
                }
            }
            _ => {}
        }
    }

    /// Klick, Ziehen und Loslassen an das Programm der Session, wenn es die Maus eingeschaltet hat
    /// — Claude Code markiert dann selbst (über den ganzen Verlauf) und kopiert in die
    /// Zwischenablage. Mit Alt gedrückt bleibt es bei der eigenen Markierung. `true`, wenn das
    /// Ereignis damit erledigt ist.
    fn forward_mouse(&mut self, mouse: MouseEvent) -> bool {
        let position = Position::new(mouse.column, mouse.row);
        let modifiers = mouse::Modifiers {
            shift: mouse.modifiers.contains(KeyModifiers::SHIFT),
            alt: mouse.modifiers.contains(KeyModifiers::ALT),
            ctrl: mouse.modifiers.contains(KeyModifiers::CONTROL),
        };
        let (action, id, cell) = match mouse.kind {
            MouseEventKind::Down(button) => {
                if modifiers.alt || self.list_hits.borrow().at(position).is_some() {
                    return false;
                }
                let Some((slot, id, cell)) = self.window_at(position) else {
                    return false;
                };
                // Klick auf Rahmen oder Titel setzt nur den Fokus (eigener Weg unten).
                let on_content = self
                    .window_rect(slot)
                    .is_some_and(|r| r.inner(Margin::new(1, 1)).contains(position));
                if !on_content || !self.session(id).is_some_and(Session::wants_mouse) {
                    return false;
                }
                self.close_overlays();
                self.selection = None;
                self.dragging = false;
                self.set_focus(Focus::Window(slot));
                let button = mouse_button(button);
                self.forwarding = Some((id, button));
                (mouse::Action::Press(button), id, cell)
            }
            MouseEventKind::Drag(_) | MouseEventKind::Up(_) => {
                let Some((id, button)) = self.forwarding else {
                    return false;
                };
                // Auch außerhalb des Fensters weiterziehen, aber auf dessen Inhalt begrenzt.
                let Some(inner) = self
                    .windows
                    .window_of(id)
                    .and_then(|slot| self.window_rect(slot))
                    .map(|r| r.inner(Margin::new(1, 1)))
                else {
                    self.forwarding = None;
                    return true;
                };
                let action = if matches!(mouse.kind, MouseEventKind::Up(_)) {
                    self.forwarding = None;
                    mouse::Action::Release(button)
                } else {
                    mouse::Action::Drag(button)
                };
                (action, id, cell_in(inner, position))
            }
            _ => return false,
        };
        let report = mouse::Report {
            action,
            col: cell.col,
            row: cell.row,
            modifiers,
        };
        if let Some(session) = self.session_mut(id) {
            if let Err(e) = session.send_mouse(report) {
                self.notice = Some(format!("{e:#}"));
            }
        }
        true
    }

    fn on_overlay(&self, position: Position) -> bool {
        let open = self.menu.is_some() || self.dir_picker.is_some() || self.group_picker.is_some();
        open && self.overlay.get().is_some_and(|r| r.contains(position))
    }

    /// Klick in die Sessionliste: Session auswählen und Fokus auf die Liste; ein zweiter Klick auf
    /// dieselbe Session kurz danach wirkt wie Enter.
    fn on_list_click(&mut self, claude_id: String) {
        self.close_overlays();
        self.selection = None;
        let double = self
            .last_list_click
            .as_ref()
            .is_some_and(|(at, id)| at.elapsed() < DOUBLE_CLICK && *id == claude_id);
        self.last_list_click = Some((Instant::now(), claude_id.clone()));
        self.cursor = Some(claude_id);
        self.set_focus(Focus::List);
        if double {
            self.last_list_click = None;
            let enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
            if let Err(e) = self.on_list_key(enter) {
                self.notice = Some(format!("{e:#}"));
            }
        }
    }

    /// Schließt Menü, Verzeichnisauswahl und Gruppen-Dialog.
    fn close_overlays(&mut self) {
        self.menu = None;
        self.dir_picker = None;
        self.group_picker = None;
    }

    /// Fenster unter der Mausposition: Fensterindex, Session und Zelle im Fensterinhalt.
    fn window_at(&self, position: Position) -> Option<(usize, SessionId, Cell)> {
        self.window_rects()
            .into_iter()
            .find(|(_, _, rect)| rect.contains(position))
            .map(|(slot, id, rect)| (slot, id, cell_in(rect.inner(Margin::new(1, 1)), position)))
    }

    fn copy_selection(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some(text) = self
            .session(selection.session)
            .map(|s| selection.text(s.screen().screen()))
        else {
            return;
        };
        let copied = arboard::Clipboard::new().and_then(|mut c| c.set_text(text.clone()));
        match copied {
            Ok(()) => self.flash_notice(format!("copied {} characters", text.chars().count())),
            Err(e) => self.notice = Some(format!("clipboard: {e}")),
        }
    }

    /// Zeigt einen Hinweis, der nach [`FLASH_DURATION`] von selbst verschwindet.
    fn flash_notice(&mut self, message: String) {
        self.notice_expiry = Some((message.clone(), Instant::now() + FLASH_DURATION));
        self.notice = Some(message);
    }

    fn expire_notice(&mut self) {
        let Some((message, deadline)) = &self.notice_expiry else {
            return;
        };
        if Instant::now() < *deadline {
            return;
        }
        if self.notice.as_ref() == Some(message) {
            self.notice = None;
        }
        self.notice_expiry = None;
    }

    pub fn resize(&mut self, screen: Rect) {
        self.screen = screen;
        self.resize_windows();
    }

    /// Claude-ID der Session im Fenster mit Fokus bzw. der in der Liste gewählten.
    fn focused_claude_id(&self) -> Option<String> {
        match self.windows.focused_session() {
            Some(id) => self.session(id).map(|s| s.claude_id.clone()),
            None => self.selected_entry().map(|e| e.claude_id.clone()),
        }
    }

    fn toggle_tag(&mut self, number: u8) {
        let Some(claude_id) = self.focused_claude_id() else {
            return;
        };
        let message = match tags::toggle(&mut self.tags, number, &claude_id) {
            Toggled::Set => format!("tag {number} set (Ctrl+{number} shows it)"),
            Toggled::Removed => format!("tag {number} removed"),
        };
        self.flash_notice(message);
        self.save_state();
    }

    /// Zeigt die Session mit dieser Nummer: hat sie ein Fenster, bekommt es den Fokus. Sonst ersetzt
    /// sie die Session im Fenster mit Fokus bzw. geht — bei Fokus auf der Liste — weiter wie Enter.
    fn show_tagged(&mut self, number: u8) -> Result<()> {
        let Some(claude_id) = self.tags.get(&number).cloned() else {
            self.flash_notice(format!(
                "no session tagged {number} (Alt+{number} tags one)"
            ));
            return Ok(());
        };
        let Some(entry) = list::flatten(&self.groups)
            .find(|e| e.claude_id == claude_id)
            .cloned()
        else {
            // Die Liste kann beim Start noch unvollständig sein; dann bleibt die Nummer stehen.
            if self.loading() {
                self.flash_notice("loading sessions …".to_string());
            } else {
                self.tags.remove(&number);
                self.save_state();
                self.flash_notice(format!("session tagged {number} no longer exists"));
            }
            return Ok(());
        };
        self.cursor = Some(claude_id);
        let focused_slot = match self.windows.focus() {
            Focus::Window(slot) => Some(slot),
            Focus::List => None,
        };
        if let Some(slot) = entry.running.and_then(|id| self.windows.window_of(id)) {
            self.set_focus(Focus::Window(slot));
            return Ok(());
        }
        match (entry.running, focused_slot) {
            (Some(id), Some(slot)) => self.place(id, slot),
            (None, Some(slot)) => self.open_stored(&entry, Some(slot))?,
            (_, None) => {
                let enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
                self.on_list_key(enter)?;
            }
        }
        Ok(())
    }

    fn on_list_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            // Markieren und weiter zur nächsten Session, damit sich Folgen schnell markieren lassen.
            KeyCode::Char(' ') => {
                if !self.marked.remove(&entry.claude_id) {
                    self.marked.insert(entry.claude_id.clone());
                }
                self.move_cursor(1);
            }
            KeyCode::Esc => self.marked.clear(),
            KeyCode::Enter => match entry.running {
                Some(id) => match self.windows.window_of(id) {
                    Some(slot) => self.set_focus(Focus::Window(slot)),
                    None if self.windows.len() == 0 => self.place(id, 0),
                    None => self.open_pick(Purpose::Show(id)),
                },
                None => self.open_stored(&entry, None)?,
            },
            KeyCode::Char(d @ '1'..='4') => {
                let slot = d as usize - '1' as usize;
                if slot <= self.windows.len() && slot < MAX_WINDOWS {
                    match entry.running {
                        Some(id) => self.place(id, slot),
                        None => self.open_stored(&entry, Some(slot))?,
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn move_cursor(&mut self, delta: isize) {
        let ids: Vec<&str> = list::flatten(&self.groups)
            .map(|e| e.claude_id.as_str())
            .collect();
        let current = self
            .cursor
            .as_deref()
            .and_then(|c| ids.iter().position(|&id| id == c))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(ids.len().saturating_sub(1));
        self.cursor = ids.get(next).map(|id| id.to_string());
    }

    /// Öffnet eine gespeicherte Session in `slot` (`None`: Fensterauswahl, ohne offenes Fenster
    /// Fenster 1). Läuft sie anderswo, kommt erst die Rückfrage — außer bei einer
    /// Hintergrund-Session: `claude attach` startet keine zweite Instanz.
    fn open_stored(&mut self, entry: &Entry, slot: Option<usize>) -> Result<()> {
        if entry.external && self.job_of(&entry.claude_id).is_none() {
            self.menu = Some(Menu::confirm_open(
                &entry.title,
                entry.claude_id.clone(),
                slot,
            ));
            return Ok(());
        }
        self.open_confirmed(entry.claude_id.clone(), slot)
    }

    /// Job-ID, falls `claude_id` gerade als Hintergrund-Session außerhalb von claude-tui läuft.
    fn job_of(&self, claude_id: &str) -> Option<&str> {
        if self.sessions.iter().any(|s| s.claude_id == claude_id) {
            return None;
        }
        self.claimed.get(claude_id)?.as_deref()
    }

    fn open_confirmed(&mut self, claude_id: String, slot: Option<usize>) -> Result<()> {
        match slot {
            Some(slot) => self.resume_into(&claude_id, slot),
            None if self.windows.len() == 0 => self.resume_into(&claude_id, 0),
            None => {
                self.open_pick(Purpose::Resume(claude_id));
                Ok(())
            }
        }
    }

    /// Setzt eine gespeicherte Session in ihrem Verzeichnis fort (bzw. verbindet sich mit ihr, wenn
    /// sie als Hintergrund-Session läuft) und legt sie in `slot`.
    fn resume_into(&mut self, claude_id: &str, slot: usize) -> Result<()> {
        let stored = self
            .store
            .get(claude_id)
            .context("session is no longer stored")?;
        let cwd = stored.cwd.clone();
        let launch = match self.job_of(claude_id) {
            Some(job) => Launch::Attach(claude_id.to_string(), job.to_string()),
            None => Launch::Resume(claude_id.to_string()),
        };
        let id = self.spawn(&cwd, launch)?;
        self.place(id, slot);
        Ok(())
    }

    /// Verzeichnis gewählt: direkt ins Zielfenster, sonst (bei offenen Fenstern) Fensterauswahl.
    fn on_dir_chosen(&mut self, dir: PathBuf, target: Target) -> Result<()> {
        match target {
            Target::Window(slot) => {
                let id = self.spawn(&dir, Launch::New)?;
                self.place(id, slot);
            }
            Target::PickWindow if self.windows.len() == 0 => {
                let id = self.spawn(&dir, Launch::New)?;
                self.place(id, 0);
            }
            Target::PickWindow => self.open_pick(Purpose::NewSession(dir)),
        }
        Ok(())
    }

    fn run(&mut self, action: Action, purpose: Option<Purpose>) -> Result<()> {
        match action {
            Action::NewSession => {
                let target = match self.windows.focus() {
                    Focus::Window(slot) => Target::Window(slot),
                    Focus::List => Target::PickWindow,
                };
                let default = self.recent.first().unwrap_or(&self.start_dir);
                self.dir_picker = Some(DirPicker::new(target, self.recent.clone(), default));
            }
            Action::Place(slot) => match purpose {
                Some(Purpose::NewSession(dir)) => {
                    let id = self.spawn(&dir, Launch::New)?;
                    self.place(id, slot);
                }
                Some(Purpose::Show(id)) => self.place(id, slot),
                Some(Purpose::Resume(claude_id)) => self.resume_into(&claude_id, slot)?,
                Some(Purpose::Delete(_) | Purpose::DeleteMany(_) | Purpose::OpenElsewhere(..))
                | None => {}
            },
            Action::CloseWindow => {
                if let Some(slot) = self
                    .context_session()
                    .and_then(|id| self.windows.window_of(id))
                {
                    self.windows.close(slot);
                    self.after_layout_change();
                }
            }
            Action::DeleteSession
                if self.windows.focus() == Focus::List && !self.marked.is_empty() =>
            {
                // In Listenreihenfolge, damit die Abfrage stabil ist.
                let ids: Vec<String> = list::flatten(&self.groups)
                    .filter(|e| self.marked.contains(&e.claude_id))
                    .map(|e| e.claude_id.clone())
                    .collect();
                let question = format!("Delete {}?", menu::sessions(ids.len()));
                self.menu = Some(Menu::confirm_delete_many(question, ids));
            }
            Action::DeleteOlder => self.menu = Some(Menu::pick_age()),
            Action::DeleteOlderThan(days) => self.ask_delete_older(days),
            Action::DeleteSession => {
                let target = match self.windows.focused_session() {
                    Some(id) => self.session(id).map(|s| s.claude_id.clone()),
                    None => self.selected_entry().map(|e| e.claude_id.clone()),
                };
                if let Some(claude_id) = target {
                    let title = list::flatten(&self.groups)
                        .find(|e| e.claude_id == claude_id)
                        .map_or_else(|| NEW_SESSION_TITLE.to_string(), |e| e.title.clone());
                    self.menu = Some(Menu::confirm_delete(&title, claude_id));
                }
            }
            Action::ConfirmDelete => match purpose {
                Some(Purpose::Delete(claude_id)) => self.delete_session(&claude_id)?,
                Some(Purpose::DeleteMany(ids)) => self.delete_sessions(&ids)?,
                _ => {}
            },
            Action::ConfirmOpen => {
                if let Some(Purpose::OpenElsewhere(claude_id, slot)) = purpose {
                    self.open_confirmed(claude_id, slot)?;
                }
            }
            Action::Cancel => {}
            Action::Focus(focus) => self.set_focus(focus),
            Action::ToggleList => self.set_list_hidden(!self.list_hidden),
            Action::ToggleStacked => {
                self.windows.set_zoomed(false);
                self.stacked = !self.stacked;
                self.after_layout_change();
            }
            Action::ToggleZoom => {
                let zoom = self.windows.zoomed().is_none();
                self.windows.set_zoomed(zoom);
                self.resize_windows();
            }
            Action::NextWaiting => self.show_next_waiting()?,
            Action::SetGroup => self.open_group_picker(),
            Action::Quit => self.should_quit = true,
            Action::Update => {
                if let Some(available) = self.update.clone() {
                    self.notice = Some(format!("downloading v{} …", available.version));
                    self.update_install = Some(update::spawn_install(available));
                }
            }
        }
        Ok(())
    }

    /// Laufende Session, auf die sich c/x beziehen: die im Fenster mit Fokus, sonst die in der
    /// Liste gewählte.
    fn context_session(&self) -> Option<SessionId> {
        self.windows
            .focused_session()
            .or_else(|| self.selected_running())
    }

    fn window_names(&self) -> Vec<String> {
        self.windows
            .slots()
            .iter()
            .map(|&id| self.title_of(id))
            .collect()
    }

    fn open_commands(&mut self) {
        let windows = self.window_names();
        let selected = self.selected_running();
        self.menu = Some(Menu::commands(&Context {
            focus: self.windows.focus(),
            windows: &windows,
            selected_has_window: selected.is_some_and(|id| self.windows.window_of(id).is_some()),
            has_selection: self.selected_entry().is_some(),
            update: self
                .update
                .as_ref()
                .filter(|_| self.update_install.is_none())
                .map(|u| u.version.as_str()),
            list_hidden: self.list_hidden,
            stacked: self.stacked,
            zoomed: self.windows.zoomed().is_some(),
            waiting: self.sessions.iter().any(|s| s.status == Status::Waiting),
            marked: self.marked.len(),
        }));
    }

    /// Gruppen-Dialog für die markierten Sessions (nur mit Fokus auf der Liste) bzw. die Session
    /// mit Fokus. Die Ziele werden beim Öffnen festgehalten, nicht erst bei der Auswahl.
    fn open_group_picker(&mut self) {
        let targets: Vec<String> = if self.windows.focus() == Focus::List && !self.marked.is_empty()
        {
            list::flatten(&self.groups)
                .filter(|e| self.marked.contains(&e.claude_id))
                .map(|e| e.claude_id.clone())
                .collect()
        } else {
            self.focused_claude_id().into_iter().collect()
        };
        let title = match targets.as_slice() {
            [] => return,
            [one] => {
                let name = list::flatten(&self.groups)
                    .find(|e| &e.claude_id == one)
                    .map_or(NEW_SESSION_TITLE, |e| e.title.as_str());
                format!("Group for {}", menu::shorten(name, GROUP_TITLE_MAX))
            }
            many => format!("Group for {}", menu::sessions(many.len())),
        };
        let offer_none = targets.iter().any(|id| self.group_of.contains_key(id));
        let choices = groups::choices(&self.group_of, &self.recent_groups);
        self.group_picker = Some(GroupPicker::new(title, targets, choices, offer_none));
    }

    /// Ordnet die Sessions der gewählten Gruppe zu (`None` = aus der Gruppe nehmen).
    fn apply_group(&mut self, targets: &[String], group: Option<String>) {
        let known = groups::choices(&self.group_of, &self.recent_groups);
        let name = group.and_then(|g| groups::normalize(&g, &known));
        groups::assign(&mut self.group_of, targets, name.as_deref());
        let count = menu::sessions(targets.len());
        let message = match &name {
            Some(name) => {
                groups::remember(&mut self.recent_groups, name);
                format!("{count} in group \"{name}\"")
            }
            None => format!("{count} removed from group"),
        };
        // Markierung erledigt — sonst bezöge sich das nächste `x` noch auf sie.
        if self.windows.focus() == Focus::List {
            self.marked.clear();
        }
        self.rebuild_list();
        self.save_state();
        self.flash_notice(message);
    }

    /// Nächste wartende Session (Listenreihenfolge, reihum ab der aktuellen): hat sie ein Fenster,
    /// Fokus dorthin, sonst wie Enter in der Liste.
    fn show_next_waiting(&mut self) -> Result<()> {
        let order: Vec<SessionId> = list::flatten(&self.groups)
            .filter_map(|e| e.running)
            .collect();
        let is_waiting = |id| {
            self.session(id)
                .is_some_and(|s| s.status == Status::Waiting)
        };
        let Some(id) = waiting::next_waiting(&order, is_waiting, self.context_session()) else {
            return Ok(());
        };
        if let Some(session) = self.session(id) {
            self.cursor = Some(session.claude_id.clone());
        }
        let enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        self.on_list_key(enter)
    }

    /// Schließt Fenster und Prozess (falls laufend) und löscht das Protokoll endgültig.
    fn delete_session(&mut self, claude_id: &str) -> Result<()> {
        let result = self.remove_stored(claude_id);
        self.store.refresh();
        self.rebuild_list();
        result
    }

    /// Wie [`Self::delete_session`] für mehrere Sessions; Fehler halten die übrigen nicht auf.
    fn delete_sessions(&mut self, claude_ids: &[String]) -> Result<()> {
        let mut failed = Vec::new();
        for claude_id in claude_ids {
            if let Err(e) = self.remove_stored(claude_id) {
                failed.push(e);
            }
            self.marked.remove(claude_id);
        }
        self.store.refresh();
        self.rebuild_list();
        let deleted = claude_ids.len() - failed.len();
        match failed.into_iter().next() {
            None => {
                self.flash_notice(format!("deleted {}", menu::sessions(deleted)));
                Ok(())
            }
            Some(first) => Err(first.context(format!(
                "deleted {}, {} failed",
                menu::sessions(deleted),
                claude_ids.len() - deleted
            ))),
        }
    }

    fn remove_stored(&mut self, claude_id: &str) -> Result<()> {
        let running = self
            .sessions
            .iter()
            .find(|s| s.claude_id == claude_id)
            .map(|s| s.id);
        if let Some(id) = running {
            self.remove_process(id);
        }
        let tagged = tags::remove_session(&mut self.tags, claude_id);
        let grouped = self.group_of.remove(claude_id).is_some();
        if tagged || grouped {
            self.save_state();
        }
        // Überholte Protokolle vor einer Verlegung gehören zur Session und gehen mit.
        let predecessors: Vec<String> = self
            .superseded
            .iter()
            .filter(|(_, new)| *new == claude_id)
            .map(|(old, _)| old.clone())
            .collect();
        for old in &predecessors {
            if let Some(p) = self.store.path_of(old) {
                claude_store::delete(&p)?;
            }
            self.superseded.remove(old);
        }
        if !predecessors.is_empty() {
            self.save_state();
        }
        let path = self.store.path_of(claude_id);
        path.map_or(Ok(()), |p| claude_store::delete(&p))
    }

    /// Fragt nach, ob alle gespeicherten Sessions gelöscht werden sollen, deren letzte Aktivität
    /// mehr als `days` Tage zurückliegt — über alle Verzeichnisse. Laufende (in claude-tui oder
    /// anderswo) bleiben immer stehen.
    fn ask_delete_older(&mut self, days: u64) {
        let cutoff = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        let (busy, ids): (Vec<&Entry>, Vec<&Entry>) = list::flatten(&self.groups)
            .filter(|e| e.modified.is_some_and(|m| m < cutoff))
            .partition(|e| e.running.is_some() || e.external);
        let age = menu::age_label(days);
        let skipped = match busy.len() {
            0 => String::new(),
            n => format!(" ({n} running skipped)"),
        };
        if ids.is_empty() {
            self.notice = Some(format!("no stored sessions older than {age}{skipped}"));
            return;
        }
        let question = format!(
            "Delete {} older than {age}?{skipped}",
            menu::sessions(ids.len())
        );
        let ids = ids.into_iter().map(|e| e.claude_id.clone()).collect();
        self.menu = Some(Menu::confirm_delete_many(question, ids));
    }

    fn open_pick(&mut self, purpose: Purpose) {
        let title = match &purpose {
            Purpose::NewSession(_) => "New session in".to_string(),
            &Purpose::Show(id) => format!("Show {} in", self.title_of(id)),
            Purpose::Resume(_) => self
                .selected_entry()
                .map_or_else(String::new, |e| format!("Open {} in", e.title)),
            Purpose::Delete(_) | Purpose::DeleteMany(_) | Purpose::OpenElsewhere(..) => return,
        };
        self.menu = Some(Menu::pick_window(
            purpose,
            title,
            self.windows.slots().len(),
        ));
    }

    fn move_focus(&mut self, direction: Direction) {
        match self.windows.focus() {
            Focus::List => {
                if direction == Direction::Right && self.windows.len() > 0 {
                    let slot = self
                        .selected_running()
                        .and_then(|id| self.windows.window_of(id))
                        .unwrap_or(0);
                    self.set_focus(Focus::Window(slot));
                }
            }
            Focus::Window(slot) => {
                // Gezoomt: erst aufheben, dann den Nachbarn im normalen Layout suchen.
                if self.windows.zoomed().is_some() {
                    self.windows.set_zoomed(false);
                    self.resize_windows();
                }
                let rects = layout::windows(
                    ui::windows_area(self.screen, self.list_hidden),
                    self.windows.len(),
                    self.stacked,
                );
                match layout::neighbor(&rects, slot, direction) {
                    Some(next) => self.set_focus(Focus::Window(next)),
                    None if direction == Direction::Left => self.set_focus(Focus::List),
                    None => {}
                }
            }
        }
    }

    /// Setzt den Fokus; bei einem Fenster folgt die Listenauswahl dessen Session.
    fn set_focus(&mut self, focus: Focus) {
        let zoomed = self.windows.zoomed().is_some();
        self.windows.set_focus(focus);
        // Ein Fokuswechsel hebt den Zoom auf; die übrigen Fenster brauchen wieder ihre Größe.
        if zoomed && self.windows.zoomed().is_none() {
            self.resize_windows();
        }
        self.keep_list_reachable();
        self.sync_cursor();
        self.save_state();
    }

    /// Blendet die Sessionliste aus oder ein. Ausblenden geht nur mit einem offenen Fenster; hat
    /// die Liste den Fokus, wandert er ins Fenster ihrer Session (sonst Fenster 1).
    fn set_list_hidden(&mut self, hidden: bool) {
        if hidden && self.windows.len() == 0 {
            return;
        }
        if hidden && self.windows.focus() == Focus::List {
            let slot = self
                .selected_running()
                .and_then(|id| self.windows.window_of(id))
                .unwrap_or(0);
            self.windows.set_focus(Focus::Window(slot));
            self.sync_cursor();
        }
        self.list_hidden = hidden;
        self.resize_windows();
        self.save_state();
    }

    /// Fokus auf der Liste oder kein Fenster mehr offen: die Liste muss sichtbar sein.
    fn keep_list_reachable(&mut self) {
        if self.list_hidden && (self.windows.focus() == Focus::List || self.windows.len() == 0) {
            self.list_hidden = false;
            self.resize_windows();
        }
    }

    fn sync_cursor(&mut self) {
        if let Some(id) = self.windows.focused_session() {
            if let Some(session) = self.session(id) {
                self.cursor = Some(session.claude_id.clone());
            }
        }
    }

    fn place(&mut self, id: SessionId, slot: usize) {
        if self.windows.place(id, slot) {
            self.after_layout_change();
        }
    }

    /// Beendet den Prozess (Drop) und schließt sein Fenster; die Claude-Session bleibt gespeichert.
    fn remove_process(&mut self, id: SessionId) {
        if let Some(session) = self.session(id) {
            self.ended.push((session.claude_id.clone(), Instant::now()));
        }
        self.windows.remove_session(id);
        self.sessions.retain(|s| s.id != id);
        self.list_dirty = true;
        self.after_layout_change();
    }

    fn after_layout_change(&mut self) {
        self.keep_list_reachable();
        self.resize_windows();
        self.sync_cursor();
        self.save_state();
    }

    fn save_state(&self) {
        let state = State {
            windows: self
                .windows
                .slots()
                .iter()
                .filter_map(|&id| self.session(id).map(|s| s.claude_id.clone()))
                .collect(),
            focus: match self.windows.focus() {
                Focus::Window(slot) => Some(slot),
                Focus::List => None,
            },
            list_hidden: self.list_hidden,
            stacked: self.stacked,
            tags: self.tags.clone(),
            groups: self.group_of.clone(),
            recent_groups: self.recent_groups.clone(),
            superseded: self.superseded.clone(),
        };
        // Komfortfunktion: lässt sich der Zustand nicht speichern, läuft alles andere weiter.
        let _ = state::save(&state);
    }

    /// Jede sichtbare Session bekommt die Innenmaße ihres Fensters; Hintergrund-Sessions und
    /// beim Zoom verdeckte Fenster behalten ihre letzte Größe.
    fn resize_windows(&mut self) {
        for (_, id, rect) in self.window_rects() {
            let inner = rect.inner(Margin::new(1, 1));
            if let Some(session) = self.session_mut(id) {
                session.resize(inner.height.max(1), inner.width.max(1));
            }
        }
    }

    fn spawn(&mut self, dir: &Path, launch: Launch) -> Result<SessionId> {
        self.spawn_with(dir, launch, true)
    }

    /// `start_now = false`: nur Fenster und PTY anlegen, der Prozess startet später (Startreihe).
    fn spawn_with(&mut self, dir: &Path, launch: Launch, start_now: bool) -> Result<SessionId> {
        // IDs stammen aus Dateinamen bzw. Dateien unter `~/.claude` und gehen als Argumente an die
        // Shell: nur gültige Formen, damit weder Shell-Code noch eine Option (`--…`) daraus wird.
        let (claude_id, mut extra, attach) = match launch {
            Launch::New => {
                let claude_id = uuid::Uuid::new_v4().to_string();
                let args = vec!["--session-id".to_string(), claude_id.clone()];
                (claude_id, args, false)
            }
            Launch::Resume(claude_id) => {
                anyhow::ensure!(
                    session::valid_claude_id(&claude_id),
                    "cannot resume session with invalid id {claude_id:?}"
                );
                let args = vec!["--resume".to_string(), claude_id.clone()];
                (claude_id, args, false)
            }
            Launch::Attach(claude_id, job) => {
                // Die Claude-ID geht hier nicht in die Befehlszeile, nur die Job-ID.
                anyhow::ensure!(
                    session::valid_job_id(&job),
                    "cannot attach to background session with invalid id {job:?}"
                );
                (claude_id, vec!["attach".to_string(), job], true)
            }
        };
        // Ohne Hook-Einstellungen bleibt es bei der Heuristik; die Session startet trotzdem. Eine
        // Hintergrund-Session läuft mit ihren eigenen Einstellungen weiter, `attach` nimmt keine.
        if !attach {
            if let Ok(file) = hooks::write_settings() {
                extra.push("--settings".to_string());
                extra.push(file.to_string_lossy().into_owned());
            }
        }
        // Meldung eines früheren Laufs derselben Claude-Session gilt nicht mehr.
        hooks::clear(&claude_id);
        self.next_id += 1;
        let id = self.next_id;
        let inner = ui::windows_area(self.screen, self.list_hidden).inner(Margin::new(1, 1));
        let session = Session::spawn(
            id,
            claude_id,
            &self.command,
            &extra,
            dir,
            inner.height.max(1),
            inner.width.max(1),
            start_now,
        )?;
        self.sessions.push(session);
        recent::remember(&mut self.recent, dir);
        // Die Liste ist Komfort: lässt sie sich nicht speichern, läuft die Session trotzdem.
        let _ = recent::save(&self.recent);
        self.rebuild_list();
        Ok(id)
    }
}

/// Liest die Protokolle im Hintergrund ein (ab dem Stand des Caches), damit das erste Bild nicht
/// darauf wartet.
fn spawn_store_load() -> Option<Receiver<Store>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("session-store".into())
        .spawn(move || {
            let mut store = Store::from_cache();
            store.refresh();
            let _ = tx.send(store);
        })
        .ok()
        .map(|_| rx)
}

/// Zelle im Fensterinhalt `inner` unter der Mausposition, an den Rand geklemmt.
fn mouse_button(button: MouseButton) -> mouse::Button {
    match button {
        MouseButton::Left => mouse::Button::Left,
        MouseButton::Middle => mouse::Button::Middle,
        MouseButton::Right => mouse::Button::Right,
    }
}

fn cell_in(inner: Rect, position: Position) -> Cell {
    let col = position.x.clamp(inner.x, inner.right().saturating_sub(1)) - inner.x;
    let row = position.y.clamp(inner.y, inner.bottom().saturating_sub(1)) - inner.y;
    Cell { row, col }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_click_finds_session_row_but_not_headers_or_outside() {
        let hits = ListHits {
            area: Rect::new(1, 1, 40, 10),
            rows: vec![(2, "a".into()), (3, "b".into())],
        };
        assert_eq!(hits.at(Position::new(5, 3)), Some("b"));
        assert_eq!(hits.at(Position::new(5, 1)), None, "Überschrift");
        assert_eq!(
            hits.at(Position::new(45, 3)),
            None,
            "rechts neben der Liste"
        );
    }

    #[test]
    fn mouse_cell_is_clamped_to_window_content() {
        let inner = Rect::new(10, 5, 20, 8);
        assert_eq!(
            cell_in(inner, Position::new(12, 6)),
            Cell { row: 1, col: 2 }
        );
        assert_eq!(
            cell_in(inner, Position::new(0, 100)),
            Cell { row: 7, col: 0 }
        );
    }
}
