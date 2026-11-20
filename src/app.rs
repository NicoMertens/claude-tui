use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context as _, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Margin, Position, Rect};

use crate::{
    claude_store::{self, Store},
    closed,
    dirpick::{DirPicker, Outcome as DirOutcome, Target},
    external,
    grouppick::{GroupPicker, Outcome as GroupOutcome},
    groups::{self, Assignments},
    help::{Help, Outcome as HelpOutcome},
    hooks,
    keys::{self, TagKey},
    layout::{self, Direction},
    list::{self, Cursor, Entry, Group, Row, Running, NEW_SESSION_TITLE},
    menu::{self, Action, Context, Menu, Outcome, Purpose},
    mouse,
    noteedit::{NoteEditor, Outcome as NoteOutcome},
    notelist::{self, NoteList},
    notepad::Notepad,
    recent,
    selection::{self, Cell, Selection},
    session::{self, Session, Status},
    state::{self, State},
    tags::{self, Tags, Toggled},
    todo::Todo,
    trace, ui,
    update::{self, Available},
    waiting,
    windows::{Focus, Pane, SessionId, Spot, Windows, MAX_WINDOWS},
};

/// So oft wird `~/.claude/projects` nach neuen Sessions und Titeln durchsucht.
const STORE_INTERVAL: Duration = Duration::from_secs(2);
/// Höchstens so oft wird der Lesestand der Protokolle gespeichert (außerdem beim Beenden).
const CACHE_INTERVAL: Duration = Duration::from_secs(60);
/// Beim Wiederherstellen startet die nächste Session, sobald die vorige etwas anzeigt, spätestens
/// nach dieser Zeit — mehrere Claude-Prozesse gleichzeitig bremsen sich gegenseitig aus.
const STAGGER_MAX: Duration = Duration::from_secs(3);
/// Zweiter Klick auf dieselbe Stelle innerhalb dieser Zeit = Doppelklick (Wort markieren, in
/// Sessionliste und Notizübersicht wie Enter).
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
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
/// Zeile als Cursor) je Session und Überschrift.
/// Ein sichtbares Fenster: wo es liegt (Art des Fensters), was es zeigt (Quelle), wie groß es ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub spot: Spot,
    pub pane: Pane,
    pub rect: Rect,
}

#[derive(Debug, Default)]
pub struct ListHits {
    pub area: Rect,
    pub rows: Vec<(u16, Cursor)>,
}

impl ListHits {
    fn at(&self, position: Position) -> Option<&Cursor> {
        if !self.area.contains(position) {
            return None;
        }
        self.rows
            .iter()
            .find(|(y, _)| *y == position.y)
            .map(|(_, cursor)| cursor)
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
    /// Auswahl in der Liste. Steht sie auf einer Session in einem eingeklappten Abschnitt, gilt
    /// dessen Überschrift als ausgewählt (siehe [`Self::visible_cursor`]).
    pub cursor: Option<Cursor>,
    /// Eingeklappte Abschnitte (Schlüssel nach [`list::Heading::key`]); bleiben über
    /// `state.json` erhalten.
    pub collapsed: BTreeSet<String>,
    /// In der Liste markierte Sessions (Leertaste); `x` löscht sie gemeinsam.
    pub marked: HashSet<String>,
    /// Schnellwahl-Nummern (Alt+Zahl vergibt, Ctrl+Zahl zeigt); bleiben über `state.json` erhalten.
    pub tags: Tags,
    /// Eigene Gruppen je Claude-ID; bleiben über `state.json` erhalten.
    pub group_of: Assignments,
    /// Claude-IDs mit ungelesener fertiger Antwort; bleiben über `state.json` erhalten. Laufende
    /// Sessions führen den Stand selbst, [`Self::sync_unread`] gleicht ab.
    unread: BTreeSet<String>,
    /// Zuletzt verwendete Gruppennamen, neueste zuerst.
    recent_groups: Vec<String>,
    /// Claude-IDs laufender Sessions, deren Fenster zuletzt zuging, neueste zuerst (Ctrl+A r,
    /// siehe `closed.rs`).
    closed: Vec<String>,
    /// Sessions in Fenstern beim letzten [`Self::track_closed`].
    shown: Vec<(SessionId, String)>,
    /// Notiz je Claude-ID (Ctrl+A t); bleibt über `state.json` erhalten.
    pub notes: BTreeMap<String, String>,
    /// Gruppen-Dialog (Ctrl+A g).
    pub group_picker: Option<GroupPicker>,
    /// Notiz-Editor (Ctrl+A t).
    pub note_editor: Option<NoteEditor>,
    /// Notizblock (Ctrl+A p); sein Text gehört keiner Session.
    pub notepad: Notepad,
    /// Todo-Liste (Ctrl+A d); gehört keiner Session.
    pub todo: Todo,
    /// Notizübersicht aller Sessions (Ctrl+A m): Auswahl und Scrollen.
    pub note_list: NoteList,
    /// Hilfeseite (Ctrl+A h).
    pub help: Option<Help>,
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
    last_list_click: Option<(Instant, Cursor)>,
    /// Letztes Ein-/Ausklappen per Klick; der zweite Klick eines Doppelklicks schaltet nicht zurück.
    last_list_toggle: Option<Instant>,
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
        let (notepad, notepad_error) = Notepad::load();
        let (todo, todo_error) = Todo::load();
        let mut app = Self {
            sessions: Vec::new(),
            windows: Windows::default(),
            groups: Vec::new(),
            cursor: None,
            collapsed: BTreeSet::new(),
            marked: HashSet::new(),
            tags: Tags::new(),
            group_of: Assignments::new(),
            unread: BTreeSet::new(),
            recent_groups: Vec::new(),
            closed: Vec::new(),
            shown: Vec::new(),
            notes: BTreeMap::new(),
            group_picker: None,
            note_editor: None,
            notepad,
            todo,
            note_list: NoteList::default(),
            help: None,
            menu: None,
            dir_picker: None,
            notice: None,
            notice_expiry: None,
            selection: None,
            dragging: false,
            forwarding: None,
            last_click: None,
            last_list_click: None,
            last_list_toggle: None,
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
            next_id: 0,
        };
        app.notice = notepad_error.or(todo_error);
        let mut saved = state::load();
        // Vor dem Wiederherstellen übernehmen: jedes `place` speichert den Zustand neu.
        app.tags = std::mem::take(&mut saved.tags);
        app.group_of = std::mem::take(&mut saved.groups);
        app.recent_groups = std::mem::take(&mut saved.recent_groups);
        app.superseded = std::mem::take(&mut saved.superseded);
        app.collapsed = std::mem::take(&mut saved.collapsed);
        app.unread = std::mem::take(&mut saved.unread);
        app.notes = std::mem::take(&mut saved.notes);
        // Fehlt eine der Sessions im Cache (erster Start, neue Session), erst nach dem Einlesen.
        let known = saved
            .windows
            .iter()
            .filter(|id| !id.is_empty())
            .all(|id| app.store.get(id).is_some());
        if known || app.store_load.is_none() {
            app.restore(saved);
        } else {
            trace::mark("app: restore waits for sessions");
            app.pending_restore = Some(saved);
        }
        app.rebuild_list();
        trace::mark("app: ready");
        Ok(app)
    }

    /// Stellt das Fensterlayout des letzten Laufs wieder her; jede Session darin wird fortgesetzt.
    /// Die Prozesse starten gestaffelt (siehe [`Self::advance_starts`]), das Fenster mit Fokus
    /// zuerst. Gibt es nichts wiederherzustellen, bleibt es bei der Sessionliste.
    fn restore(&mut self, saved: State) {
        trace::mark(format_args!("app: restore {} windows", saved.windows.len()));
        let now = external::claimed_now(&saved.windows);
        let elsewhere = &now.claimed;
        // Fehlt eine Session, bleibt ihr Fenster leer; die übrigen behalten ihre Nummer.
        for (slot, claude_id) in saved.windows.into_iter().enumerate().take(MAX_WINDOWS) {
            if claude_id.is_empty() {
                continue;
            }
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
                Ok(id) => self.place(id, slot),
                Err(e) => self.notice = Some(format!("{e:#}")),
            }
        }
        if let Some(slot) = saved.notepad_window.filter(|&s| s < MAX_WINDOWS) {
            self.place_pane(Pane::Notepad, slot);
        }
        if let Some(slot) = saved.todo_window.filter(|&s| s < MAX_WINDOWS) {
            self.place_pane(Pane::Todo, slot);
        }
        if let Some(slot) = saved.notes_window.filter(|&s| s < MAX_WINDOWS) {
            self.place_pane(Pane::Notes, slot);
        }
        let focus = match (saved.focus, self.windows.first()) {
            (Some(slot), _) if self.windows.pane(slot).is_some() => Focus::Window(slot),
            (_, Some(first)) => Focus::Window(first),
            (_, None) => Focus::List,
        };
        self.set_focus(focus);
        self.set_list_hidden(saved.list_hidden);
        let mut order: Vec<SessionId> = self
            .windows
            .open()
            .filter_map(|(_, pane)| pane.session())
            .collect();
        if let Some(first) = self.windows.focused_pane().and_then(Pane::session) {
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

    /// Speichert ausstehende Änderungen an Notizblock und Todo-Liste sofort (beim Beenden); ein
    /// offenes Eingabefeld der Todo-Liste wird übernommen.
    pub fn save_panels(&mut self) -> Result<()> {
        let notepad = self.notepad.save_if_due(true);
        self.todo.save_now()?;
        notepad
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
            self.restore(saved);
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

    /// Sessionfarbe aus `/color` einer laufenden Session, wie sie die Sessionliste zeigt.
    pub fn color_of(&self, id: SessionId) -> Option<String> {
        list::flatten(&self.groups)
            .find(|e| e.running == Some(id))
            .and_then(|e| e.color.clone())
    }

    /// Claude-Code-Version einer Session laut ihrem Protokoll; `None`, solange noch nichts darin steht.
    pub fn version_of(&self, claude_id: &str) -> Option<String> {
        self.store.get(claude_id)?.version
    }

    /// Die ausgewählte Session, sofern sie sichtbar ist (nicht in einem eingeklappten Abschnitt).
    /// Zustand einer Zeile der Sessionliste: der laufenden Session, sonst ▲ für eine gespeicherte
    /// Session mit ungelesener Antwort; `None` = kein Symbol.
    pub fn list_status(&self, entry: &Entry) -> Option<Status> {
        match entry.running.and_then(|id| self.session(id)) {
            Some(session) => Some(session.status).filter(|&s| s != Status::Exited),
            None => self
                .unread
                .contains(&entry.claude_id)
                .then_some(Status::Waiting),
        }
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        match self.visible_cursor()? {
            Cursor::Session(id) => list::flatten(&self.groups).find(|e| e.claude_id == id),
            Cursor::Heading(_) | Cursor::Notepad | Cursor::Todo | Cursor::Notes => None,
        }
    }

    /// Cursor, wie die Liste ihn zeigt: eine Session in einem eingeklappten Abschnitt wird durch
    /// dessen Überschrift vertreten.
    pub fn visible_cursor(&self) -> Option<Cursor> {
        match self.cursor.as_ref()? {
            Cursor::Session(id) => {
                let key = list::group_with(&self.groups, id)?.heading.key();
                Some(if self.collapsed.contains(&key) {
                    Cursor::Heading(key)
                } else {
                    Cursor::Session(id.clone())
                })
            }
            other @ (Cursor::Heading(_) | Cursor::Notepad | Cursor::Todo | Cursor::Notes) => {
                Some(other.clone())
            }
        }
    }

    /// Abschnitt der Auswahl: die gewählte Überschrift bzw. der Abschnitt der gewählten Session.
    fn selected_group(&self) -> Option<&Group> {
        match self.visible_cursor()? {
            Cursor::Heading(key) => self.groups.iter().find(|g| g.heading.key() == key),
            Cursor::Session(id) => list::group_with(&self.groups, &id),
            Cursor::Notepad | Cursor::Todo | Cursor::Notes => None,
        }
    }

    /// Springt zur Überschrift des vorigen (`delta` < 0) bzw. nächsten Abschnitts. Steht die
    /// Auswahl auf einer Session, führt der Sprung nach oben zuerst auf ihre eigene Überschrift.
    fn jump_section(&mut self, delta: isize) {
        let keys: Vec<String> = self.groups.iter().map(|g| g.heading.key()).collect();
        let Some(last) = keys.len().checked_sub(1) else {
            return;
        };
        let (index, on_heading) = match self.visible_cursor() {
            Some(Cursor::Heading(key)) => (keys.iter().position(|k| *k == key), true),
            Some(Cursor::Session(id)) => (
                list::group_with(&self.groups, &id)
                    .and_then(|g| keys.iter().position(|k| *k == g.heading.key())),
                false,
            ),
            // Darüber gibt es nichts; nach unten geht es zur ersten Überschrift.
            Some(Cursor::Notepad | Cursor::Todo | Cursor::Notes) if delta < 0 => return,
            Some(Cursor::Notepad | Cursor::Todo | Cursor::Notes) | None => (None, false),
        };
        let target = match index {
            None => 0,
            Some(i) if delta < 0 && !on_heading => i,
            Some(i) => i.saturating_add_signed(delta).min(last),
        };
        self.cursor = Some(Cursor::Heading(keys[target].clone()));
    }

    pub fn is_collapsed(&self, group: &Group) -> bool {
        self.collapsed.contains(&group.heading.key())
    }

    /// Klappt den Abschnitt ein oder aus. Eingeklappt rückt die Auswahl auf seine Überschrift.
    fn set_collapsed(&mut self, key: String, collapsed: bool) {
        if collapsed {
            self.cursor = Some(Cursor::Heading(key.clone()));
            self.collapsed.insert(key);
        } else {
            self.collapsed.remove(&key);
        }
        self.save_state();
    }

    /// Klappt alle Abschnitte ein oder aus; die Auswahl bleibt auf ihrem Abschnitt.
    fn set_all_collapsed(&mut self, collapsed: bool) {
        if collapsed {
            self.cursor = self.visible_cursor().map(|c| match c {
                Cursor::Session(id) => list::group_with(&self.groups, &id)
                    .map_or(Cursor::Session(id), |g| Cursor::Heading(g.heading.key())),
                heading => heading,
            });
            self.collapsed
                .extend(self.groups.iter().map(|g| g.heading.key()));
        } else {
            self.collapsed.clear();
        }
        self.save_state();
    }

    /// Wählt die Session aus und klappt ihren Abschnitt dafür bei Bedarf aus.
    fn reveal(&mut self, claude_id: &str) {
        if let Some(key) = list::group_with(&self.groups, claude_id).map(|g| g.heading.key()) {
            if self.collapsed.remove(&key) {
                self.save_state();
            }
        }
        self.cursor = Some(Cursor::Session(claude_id.to_string()));
    }

    fn selected_running(&self) -> Option<SessionId> {
        self.selected_entry().and_then(|e| e.running)
    }

    /// Sichtbare Fenster von unten nach oben: die Fensterplätze in Nummernreihenfolge (gezoomt nur
    /// das gezoomte über die ganze Fensterfläche), darüber das temporäre Fenster. Einzige Quelle
    /// für Zeichnen, Größen und Maus — wer hier nach Art des Fensters fragt, fragt nie nach der
    /// Quelle und umgekehrt.
    pub fn views(&self) -> Vec<View> {
        let area = ui::windows_area(self.screen, self.list_hidden);
        let slots = layout::visible(area, self.windows.occupied(), self.windows.zoomed())
            .into_iter()
            .filter_map(|(slot, rect)| {
                Some(View {
                    spot: Spot::Window(slot),
                    pane: self.windows.pane(slot)?,
                    rect,
                })
            });
        let temporary = self.windows.temporary().map(|pane| View {
            spot: Spot::Temporary,
            pane,
            rect: layout::temporary(area),
        });
        slots.chain(temporary).collect()
    }

    /// Oberstes sichtbares Fenster an der Position.
    fn view_at(&self, position: Position) -> Option<View> {
        self.views()
            .into_iter()
            .rev()
            .find(|v| v.rect.contains(position))
    }

    /// Rechteck des Fensters, in dem `pane` gerade sichtbar ist.
    fn rect_of(&self, pane: Pane) -> Option<Rect> {
        self.views()
            .into_iter()
            .find(|v| v.pane == pane)
            .map(|v| v.rect)
    }

    /// Inhaltsfläche von `pane`, falls sichtbar.
    fn content_of(&self, pane: Pane) -> Option<Rect> {
        self.rect_of(pane).map(|rect| self.content_rect(pane, rect))
    }

    /// Inhaltsfläche einer Quelle im Fensterrechteck `rect`: Terminal der Session bzw. Textfläche.
    pub fn content_rect(&self, pane: Pane, rect: Rect) -> Rect {
        match pane {
            Pane::Session(id) => self.window_content(id, rect).0,
            Pane::Notepad | Pane::Todo | Pane::Notes => layout::panel_text(rect),
        }
    }

    /// Session im Fenster, das die Eingaben bekommt (`None` auch bei Notizblock usw.).
    fn input_session(&self) -> Option<SessionId> {
        self.windows.input_pane().and_then(Pane::session)
    }

    /// Rechteck des temporären Fensters (auch wenn es gerade zu ist).
    fn temporary_rect(&self) -> Rect {
        layout::temporary(ui::windows_area(self.screen, self.list_hidden))
    }

    /// Zeigt `pane` an `spot`. Im temporären Fenster nur, wenn es kein eigenes Fenster hat —
    /// sonst bekommt dieses den Fokus.
    fn show_at(&mut self, pane: Pane, spot: Spot) {
        match spot {
            Spot::Window(slot) => self.place_pane(pane, slot),
            Spot::Temporary => {
                self.selection = None;
                self.windows.show_temporary(pane);
                self.after_layout_change();
            }
        }
    }

    fn close_temporary(&mut self) {
        self.windows.close_temporary();
        self.selection = None;
    }

    /// Klick in ein Fenster: ein Fensterplatz bekommt den Fokus (und schließt damit das
    /// temporäre Fenster), das temporäre Fenster hat die Eingaben ohnehin.
    fn activate(&mut self, spot: Spot) {
        match spot {
            Spot::Window(slot) => self.set_focus(Focus::Window(slot)),
            Spot::Temporary => {}
        }
    }

    /// Rechteck von Fenster `slot`, falls es sichtbar ist.
    pub fn window_rect(&self, slot: usize) -> Option<Rect> {
        self.views()
            .into_iter()
            .find(|v| v.spot == Spot::Window(slot))
            .map(|v| v.rect)
    }

    /// Notiz der Claude-Session, falls sie eine hat.
    pub fn note_of(&self, claude_id: &str) -> Option<&str> {
        self.notes.get(claude_id).map(String::as_str)
    }

    /// Terminalfläche und Notizzeilen der Session `id` im Fensterrechteck `rect`.
    pub fn window_content(&self, id: SessionId, rect: Rect) -> (Rect, Option<Rect>) {
        let note_lines = self
            .session(id)
            .and_then(|s| self.note_of(&s.claude_id))
            .map_or(0, |note| note.lines().count());
        layout::window_content(rect, note_lines)
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
        // Schon vorher ungelesen (z. B. nach einem Neustart wiederhergestellt): kein neuer Hinweis,
        // auch wenn der Zustand zwischendurch auf „arbeitet“ sprang.
        let mut already_unread = HashSet::new();
        for session in &mut self.sessions {
            let before = session.status;
            if session.is_unread() {
                already_unread.insert(session.id);
            }
            session.refresh();
            changes.push((session.id, before, session.status));
        }
        let statuses_changed = changes.iter().any(|&(_, before, after)| before != after);
        changes.retain(|(id, _, _)| !already_unread.contains(id));
        self.sync_unread();
        let switches: Vec<(SessionId, hooks::Switch)> = self
            .sessions
            .iter()
            .filter_map(|s| s.take_switch().map(|switch| (s.id, switch)))
            .collect();
        for (id, switch) in switches {
            self.follow_switch(id, switch);
        }
        let watched = self.input_session();
        for id in waiting::newly_waiting(changes, watched) {
            self.notice = Some(format!("\"{}\" is waiting", self.title_of(id)));
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
            let pane = Pane::Session(id);
            if self.windows.slot_of(pane).is_some() || self.windows.temporary() == Some(pane) {
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
        if let Err(e) = self.notepad.sync() {
            self.notice = Some(format!("{e:#}"));
        }
        if let Err(e) = self.todo.sync() {
            self.notice = Some(format!("{e:#}"));
        }
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
    /// Claude hat im Prozess `id` mit `/resume` bzw. `/clear` eine andere Session geladen: Fenster
    /// und Liste führen ab jetzt die neue. Anders als bei [`Self::follow_parked`] bleibt die alte
    /// eine eigene Session in der Liste (kein `superseded`); Markierung bleibt bei ihr, Nummer und
    /// Gruppe nur bei `/resume` (`carry == false`).
    fn follow_switch(&mut self, id: SessionId, switch: hooks::Switch) {
        let new = switch.session_id;
        // Dieselbe Session (z. B. `SessionStart` nach `--resume` beim Start): nichts zu tun. Zwei
        // Prozesse mit derselben Claude-ID darf es in claude-tui nicht geben.
        if let Some(other) = self.sessions.iter().find(|s| s.claude_id == new) {
            if other.id != id {
                trace::mark(format_args!("switch to {new} ignored: already open"));
            }
            return;
        }
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let old = std::mem::replace(&mut session.claude_id, new.clone());
        trace::mark(format_args!("session {old}: switched to {new}"));
        if switch.carry {
            for tagged in self.tags.values_mut() {
                if *tagged == old {
                    tagged.clone_from(&new);
                }
            }
            if let Some(group) = self.group_of.remove(&old) {
                self.group_of.insert(new.clone(), group);
            }
            if let Some(note) = self.notes.remove(&old) {
                self.notes.insert(new.clone(), note);
            }
            if let Some(editor) = self.note_editor.as_mut().filter(|e| e.target == old) {
                editor.target.clone_from(&new);
            }
        }
        // Ungelesen gehört zum Gesprächsinhalt: Die alte behält ihren Stand, die geladene (bei
        // `/clear` leer) bringt ihren eigenen mit.
        session.set_unread(self.unread.contains(&new));
        if self.cursor == Some(Cursor::Session(old.clone())) {
            self.cursor = Some(Cursor::Session(new));
        }
        // Das alte Protokoll wurde eben noch geschrieben; ohne diesen Eintrag sähe es kurz wie eine
        // anderswo laufende Session aus.
        self.ended.push((old, Instant::now()));
        self.list_dirty = true;
        self.save_state();
        // Mit oder ohne Notiz ändert sich die Höhe des Terminals.
        self.resize_windows();
    }

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
            if self.unread.remove(&old) {
                self.unread.insert(new.clone());
            }
            if let Some(note) = self.notes.remove(&old) {
                self.notes.insert(new.clone(), note);
            }
            if let Some(editor) = self.note_editor.as_mut().filter(|e| e.target == old) {
                editor.target.clone_from(new);
            }
            if self.cursor == Some(Cursor::Session(old.clone())) {
                self.cursor = Some(Cursor::Session(new.clone()));
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
            self.resize_windows();
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
        self.forget_deleted(&stored);
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
        let valid = match &self.cursor {
            Some(Cursor::Session(id)) => listed.contains(id.as_str()),
            Some(Cursor::Heading(key)) => self.groups.iter().any(|g| g.heading.key() == *key),
            Some(Cursor::Notepad | Cursor::Todo | Cursor::Notes) => true,
            None => false,
        };
        if !valid {
            let before: Vec<&str> = list::flatten(&old).map(|e| e.claude_id.as_str()).collect();
            let successor = match &self.cursor {
                Some(Cursor::Session(gone)) => list::successor(&before, gone, &listed),
                _ => None,
            };
            // Ersatz: die erste sichtbare Session, erst wenn alles eingeklappt ist eine Überschrift.
            let rows = list::rows(&self.groups, &self.collapsed);
            let first = rows
                .iter()
                .find(|r| matches!(r, Row::Entry(_)))
                .or(rows.first());
            self.cursor = match successor {
                Some(id) => Some(Cursor::Session(id.to_string())),
                None => first.map(Row::cursor),
            };
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
            self.close_temporary();
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
        if let Some(editor) = self.note_editor.as_mut() {
            match editor.on_key(&key) {
                NoteOutcome::Stay => {}
                NoteOutcome::Cancel => self.note_editor = None,
                NoteOutcome::Save(note) => {
                    let target = std::mem::take(&mut editor.target);
                    self.note_editor = None;
                    self.apply_note(target, note);
                }
            }
            return Ok(());
        }
        if let Some(help) = self.help.as_mut() {
            if help.on_key(&key) == HelpOutcome::Close {
                self.help = None;
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
        if self.windows.temporary().is_some() {
            if keys::is_temporary_toggle(&key) {
                self.close_temporary();
                return Ok(());
            }
            // Ctrl+A und Ctrl+Zahl wirken auf die Fenster darunter: erst schließen. Alt+Zahl und
            // Ctrl+Leertaste gelten dem Inhalt des temporären Fensters.
            if keys::is_prefix(&key) || matches!(keys::tag_key(&key), Some(TagKey::Show(_))) {
                self.close_temporary();
            }
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
        if keys::is_toggle_unread(&key) {
            self.toggle_unread();
            return Ok(());
        }
        match self.windows.input_pane() {
            Some(pane) => self.pane_key(pane, key)?,
            None => self.on_list_key(key)?,
        }
        Ok(())
    }

    /// Taste an den Inhalt eines Fensters — gleich, in welcher Art Fenster er liegt.
    fn pane_key(&mut self, pane: Pane, key: KeyEvent) -> Result<()> {
        match pane {
            Pane::Session(id) => {
                if let Some(session) = self.session_mut(id) {
                    if let Some(bytes) = keys::to_bytes(&key, session.application_cursor()) {
                        session.write(&bytes)?;
                    }
                }
            }
            Pane::Notepad => self.notepad.on_key(&key),
            Pane::Todo => self.todo.on_key(&key),
            Pane::Notes => {
                let open = self
                    .note_list
                    .on_key(&key, &note_items(&self.groups, &self.notes));
                if let Some(open) = open {
                    self.open_noted(open)?;
                }
            }
        }
        Ok(())
    }

    /// Mehrere ohne Pause angekommene Zeichen sind eingefügter Text (Windows liefert Einfügen als
    /// Tastendrücke): in einer Session, damit Claude ihn als einen Block bekommt und ein Enter darin
    /// nicht abschickt, und in der Todo-Liste, die Buchstaben sonst als Befehle nimmt. Menü, Hilfe
    /// und Dialoge nehmen Tasten als Befehle und bekommen sie einzeln.
    pub fn takes_bursts_as_paste(&self) -> bool {
        if self.menu.is_some()
            || self.help.is_some()
            || self.dir_picker.is_some()
            || self.group_picker.is_some()
            || self.note_editor.is_some()
        {
            return false;
        }
        match self.windows.input_pane() {
            Some(Pane::Session(_)) => true,
            Some(Pane::Todo) => !self.todo.is_editing(),
            Some(Pane::Notes) => true,
            Some(Pane::Notepad) | None => false,
        }
    }

    pub fn on_paste(&mut self, text: &str) -> Result<()> {
        // Schnell Getipptes kann als Einfügen ankommen; es darf nicht in die Session dahinter.
        if self.help.is_some() {
            return Ok(());
        }
        if let Some(picker) = self.dir_picker.as_mut() {
            picker.paste(text);
            return Ok(());
        }
        if let Some(picker) = self.group_picker.as_mut() {
            picker.paste(text);
            return Ok(());
        }
        if let Some(editor) = self.note_editor.as_mut() {
            editor.paste(text);
            return Ok(());
        }
        match self.windows.input_pane() {
            Some(Pane::Session(id)) => {
                if let Some(session) = self.session_mut(id) {
                    if let Err(e) = session.paste(text) {
                        self.notice = Some(format!("{e:#}"));
                    }
                }
            }
            Some(Pane::Notepad) => self.notepad.paste(text),
            Some(Pane::Todo) => self.todo.paste(text),
            Some(Pane::Notes) | None => {}
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
            // Nur die Hilfe nutzt das Mausrad selbst: zum Scrollen.
            if let Some(help) = self.help.as_mut() {
                match mouse.kind {
                    MouseEventKind::ScrollUp => help.wheel(true),
                    MouseEventKind::ScrollDown => help.wheel(false),
                    _ => {}
                }
            }
            return;
        }
        let outside_temporary =
            self.windows.temporary().is_some() && !self.temporary_rect().contains(position);
        if outside_temporary && matches!(mouse.kind, MouseEventKind::Down(_)) {
            self.close_temporary();
        }
        if self.panel_mouse(mouse) {
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
                let list_hit = self.list_hits.borrow().at(position).cloned();
                if let Some(cursor) = list_hit {
                    self.on_list_click(cursor);
                    return;
                }
                let Some((spot, id, cell)) = self.window_at(position) else {
                    return;
                };
                self.close_overlays();
                self.activate(spot);
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
                let inner = self.content_of(Pane::Session(session));
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
                let Some((spot, id, cell)) = self.window_at(position) else {
                    return false;
                };
                // Klick auf Rahmen oder Titel setzt nur den Fokus (eigener Weg unten).
                let on_content = self
                    .content_of(Pane::Session(id))
                    .is_some_and(|r| r.contains(position));
                if !on_content || !self.session(id).is_some_and(Session::wants_mouse) {
                    return false;
                }
                self.close_overlays();
                self.selection = None;
                self.dragging = false;
                self.activate(spot);
                let button = mouse_button(button);
                self.forwarding = Some((id, button));
                (mouse::Action::Press(button), id, cell)
            }
            MouseEventKind::Drag(_) | MouseEventKind::Up(_) => {
                let Some((id, button)) = self.forwarding else {
                    return false;
                };
                // Auch außerhalb des Fensters weiterziehen, aber auf dessen Inhalt begrenzt.
                let Some(inner) = self.content_of(Pane::Session(id)) else {
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

    /// Klick und Mausrad über Notizblock, Todo-Liste bzw. Notizübersicht — in jeder Art Fenster:
    /// Fokus, Cursor bzw. Auswahl setzen, scrollen. `true`, wenn das Ereignis damit erledigt ist;
    /// Sessions gehen eigene Wege (Markieren, Weiterleiten), Ziehen und Loslassen gehören weiter
    /// einer laufenden Markierung.
    fn panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let position = Position::new(mouse.column, mouse.row);
        let Some(view) = self.view_at(position) else {
            return false;
        };
        let pane = view.pane;
        if pane.session().is_some() {
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = mouse.kind == MouseEventKind::ScrollUp;
                match pane {
                    Pane::Todo => self.todo.wheel(up),
                    Pane::Notes => self
                        .note_list
                        .wheel(up, &note_items(&self.groups, &self.notes)),
                    Pane::Notepad => self.notepad.wheel(up),
                    Pane::Session(_) => return false,
                }
            }
            MouseEventKind::Down(button) => {
                self.close_overlays();
                self.selection = None;
                self.dragging = false;
                self.activate(view.spot);
                let text =
                    Some(self.content_rect(pane, view.rect)).filter(|r| r.contains(position));
                if let (Some(text), MouseButton::Left) = (text, button) {
                    let cell = cell_in(text, position);
                    let (row, col) = (cell.row.into(), cell.col.into());
                    match pane {
                        Pane::Todo => self.todo.click(row, col),
                        Pane::Notes => {
                            let items = note_items(&self.groups, &self.notes);
                            let open = self.note_list.click(row, &items, Instant::now());
                            if let Some(open) = open {
                                if let Err(e) = self.open_noted(open) {
                                    self.notice = Some(format!("{e:#}"));
                                }
                            }
                        }
                        Pane::Notepad => self.notepad.click(row, col),
                        Pane::Session(_) => return false,
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn on_overlay(&self, position: Position) -> bool {
        let open = self.menu.is_some()
            || self.dir_picker.is_some()
            || self.group_picker.is_some()
            || self.note_editor.is_some()
            || self.help.is_some();
        open && self.overlay.get().is_some_and(|r| r.contains(position))
    }

    /// Klick in die Sessionliste: Zeile auswählen und Fokus auf die Liste. Ein zweiter Klick auf
    /// dieselbe Session kurz danach wirkt wie Enter; ein Klick auf die schon gewählte Überschrift
    /// klappt ihren Abschnitt ein bzw. aus.
    fn on_list_click(&mut self, cursor: Cursor) {
        self.close_overlays();
        self.selection = None;
        let heading_again = matches!(cursor, Cursor::Heading(_))
            && self.windows.focus() == Focus::List
            && self.visible_cursor().as_ref() == Some(&cursor)
            && !self
                .last_list_toggle
                .is_some_and(|at| at.elapsed() < DOUBLE_CLICK);
        let double = self
            .last_list_click
            .as_ref()
            .is_some_and(|(at, c)| at.elapsed() < DOUBLE_CLICK && *c == cursor);
        self.last_list_click = Some((Instant::now(), cursor.clone()));
        self.cursor = Some(cursor);
        self.set_focus(Focus::List);
        if let (true, Some(Cursor::Heading(key))) = (heading_again, self.cursor.clone()) {
            self.last_list_toggle = Some(Instant::now());
            let collapsed = self.collapsed.contains(&key);
            self.set_collapsed(key, !collapsed);
            return;
        }
        if double
            && matches!(
                self.cursor,
                Some(Cursor::Session(_) | Cursor::Notepad | Cursor::Todo | Cursor::Notes)
            )
        {
            self.last_list_click = None;
            let enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
            if let Err(e) = self.on_list_key(enter) {
                self.notice = Some(format!("{e:#}"));
            }
        }
    }

    /// Schließt Menü, Verzeichnisauswahl, Gruppen-Dialog, Notiz-Editor und Hilfe.
    fn close_overlays(&mut self) {
        self.menu = None;
        self.help = None;
        self.dir_picker = None;
        self.group_picker = None;
        self.note_editor = None;
    }

    /// Session-Fenster obenauf unter der Mausposition: wo es liegt, Session und Zelle im
    /// Terminal.
    fn window_at(&self, position: Position) -> Option<(Spot, SessionId, Cell)> {
        let view = self.view_at(position)?;
        let id = view.pane.session()?;
        let term = self.content_rect(view.pane, view.rect);
        Some((view.spot, id, cell_in(term, position)))
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

    /// Claude-ID der Session im Fenster mit den Eingaben bzw. der in der Liste gewählten.
    fn focused_claude_id(&self) -> Option<String> {
        match self.windows.input_pane() {
            Some(pane) => pane
                .session()
                .and_then(|id| self.session(id))
                .map(|s| s.claude_id.clone()),
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
        self.reveal(&claude_id);
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
            (None, Some(slot)) => self.open_stored(&entry, Some(Spot::Window(slot)))?,
            (_, None) => {
                let enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
                self.on_list_key(enter)?;
            }
        }
        Ok(())
    }

    fn on_list_key(&mut self, key: KeyEvent) -> Result<()> {
        let page = self.list_page();
        match key.code {
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-page),
            KeyCode::PageDown => self.move_cursor(page),
            KeyCode::Esc => self.marked.clear(),
            // Wie in einem Dateibaum: ← klappt den Abschnitt der Auswahl ein, → klappt aus.
            KeyCode::Left | KeyCode::Right => {
                if let Some(heading) = self.selected_group().map(|g| g.heading.key()) {
                    self.set_collapsed(heading, key.code == KeyCode::Left);
                }
            }
            _ => match self.visible_cursor() {
                Some(Cursor::Heading(heading)) => self.on_heading_key(key, heading),
                Some(Cursor::Session(_)) | None => self.on_entry_key(key)?,
                Some(cursor) => {
                    if let Some(pane) = self.cursor_pane(&cursor) {
                        self.on_panel_row_key(key, pane);
                    }
                }
            },
        }
        Ok(())
    }

    /// Enter klappt den Abschnitt ein bzw. aus, die Leertaste markiert alle seine Sessions (bzw.
    /// hebt die Markierung auf, wenn schon alle markiert sind) und geht weiter.
    fn on_heading_key(&mut self, key: KeyEvent, heading: String) {
        match key.code {
            KeyCode::Enter => {
                let collapsed = self.collapsed.contains(&heading);
                self.set_collapsed(heading, !collapsed);
            }
            KeyCode::Char(' ') => {
                let ids: Vec<String> = self
                    .groups
                    .iter()
                    .find(|g| g.heading.key() == heading)
                    .map(|g| g.entries.iter().map(|e| e.claude_id.clone()).collect())
                    .unwrap_or_default();
                if ids.iter().all(|id| self.marked.contains(id)) {
                    for id in &ids {
                        self.marked.remove(id);
                    }
                } else {
                    self.marked.extend(ids);
                }
                self.move_cursor(1);
            }
            _ => {}
        }
    }

    /// Inhalt hinter einer Listenzeile: laufende Session bzw. Notizblock, Todo-Liste, Notizübersicht.
    fn cursor_pane(&self, cursor: &Cursor) -> Option<Pane> {
        match cursor {
            Cursor::Session(claude_id) => self
                .sessions
                .iter()
                .find(|s| &s.claude_id == claude_id)
                .map(|s| Pane::Session(s.id)),
            Cursor::Heading(_) => None,
            Cursor::Notepad => Some(Pane::Notepad),
            Cursor::Todo => Some(Pane::Todo),
            Cursor::Notes => Some(Pane::Notes),
        }
    }

    /// Fenster des Inhalts der gewählten Listenzeile, falls er eines hat.
    fn selected_slot(&self) -> Option<usize> {
        let pane = self.cursor_pane(&self.visible_cursor()?)?;
        self.windows.slot_of(pane)
    }

    /// Zeile des Notizblocks, der Todo-Liste bzw. der Notizübersicht: Öffnen-Tasten wie bei einer
    /// Session ([`keys::destination`]).
    fn on_panel_row_key(&mut self, key: KeyEvent, pane: Pane) {
        if let Some(spot) = keys::destination(&key) {
            self.open_pane(pane, spot);
        }
    }

    /// Zeigt einen vorhandenen Inhalt an `spot`; ohne Ziel bekommt sein Fenster den Fokus, sonst
    /// kommt die Fensterauswahl (ohne offenes Fenster gleich Fenster 1).
    fn open_pane(&mut self, pane: Pane, spot: Option<Spot>) {
        match (spot, self.windows.slot_of(pane)) {
            (Some(spot), _) => self.show_at(pane, spot),
            (None, Some(slot)) => self.set_focus(Focus::Window(slot)),
            (None, None) => self.show_panel(pane),
        }
    }

    /// Ziel für Inhalte aus dem Menü: aus einem Fenster heraus ersetzen sie dieses, aus der
    /// Sessionliste kommt die Fensterauswahl (`None`).
    fn focused_spot(&self) -> Option<Spot> {
        match self.windows.focus() {
            Focus::Window(slot) => Some(Spot::Window(slot)),
            Focus::List => None,
        }
    }

    /// Inhalt aus dem Menü: ersetzt das Fenster mit Fokus, aus der Sessionliste wie
    /// [`Self::show_panel`].
    fn show_from_menu(&mut self, pane: Pane) {
        match self.focused_spot() {
            Some(spot) => self.show_at(pane, spot),
            None => self.show_panel(pane),
        }
    }

    /// Inhalt in ein Fenster: ohne offenes Fenster direkt in Fenster 1, sonst Fensterauswahl.
    fn show_panel(&mut self, pane: Pane) {
        if self.windows.count() == 0 {
            self.place_pane(pane, 0);
        } else {
            self.open_pick(Purpose::Show(pane));
        }
    }

    fn on_entry_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };
        match key.code {
            // Markieren und weiter zur nächsten Session, damit sich Folgen schnell markieren lassen.
            KeyCode::Char(' ') => {
                if !self.marked.remove(&entry.claude_id) {
                    self.marked.insert(entry.claude_id.clone());
                }
                self.move_cursor(1);
            }
            _ => {
                if let Some(spot) = keys::destination(&key) {
                    self.open_entry(&entry, spot)?;
                }
            }
        }
        Ok(())
    }

    /// Session einer Listenzeile zeigen: in Fenster bzw. temporärem Fenster `spot`, sonst wie
    /// Enter — ein vorhandenes Fenster bekommt den Fokus, sonst kommt die Fensterauswahl (ohne
    /// offenes Fenster gleich Fenster 1); eine gespeicherte Session wird fortgesetzt.
    fn open_entry(&mut self, entry: &Entry, spot: Option<Spot>) -> Result<()> {
        match entry.running {
            Some(id) => self.open_pane(Pane::Session(id), spot),
            None => self.open_stored(entry, spot)?,
        }
        Ok(())
    }

    /// Einträge der Notizübersicht: Sessions mit Notiz in der Reihenfolge der Sessionliste.
    pub fn note_items(&self) -> Vec<notelist::Item<'_>> {
        note_items(&self.groups, &self.notes)
    }

    /// Öffnet die in der Notizübersicht gewählte Session wie aus der Sessionliste. Lag die
    /// Übersicht im temporären Fenster, geht es zu: die Session soll nicht darunter verschwinden.
    fn open_noted(&mut self, open: notelist::Open) -> Result<()> {
        self.close_temporary();
        let entry = list::flatten(&self.groups)
            .find(|e| e.claude_id == open.claude_id)
            .cloned();
        match entry {
            Some(entry) => self.open_entry(&entry, open.slot.map(Spot::Window)),
            None => Ok(()),
        }
    }

    /// Sprungweite für Bild ↑/↓: eine Listenhöhe weniger eine Zeile, damit die Lage erkennbar
    /// bleibt.
    fn list_page(&self) -> isize {
        let height = self.list_hits.borrow().area.height as isize;
        (height - 1).max(1)
    }

    fn move_cursor(&mut self, delta: isize) {
        let rows: Vec<Cursor> = [Cursor::Notepad, Cursor::Todo, Cursor::Notes]
            .into_iter()
            .chain(
                list::rows(&self.groups, &self.collapsed)
                    .iter()
                    .map(Row::cursor),
            )
            .collect();
        let visible = self.visible_cursor();
        let current = rows
            .iter()
            .position(|c| Some(c) == visible.as_ref())
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(rows.len().saturating_sub(1));
        self.cursor = rows.into_iter().nth(next);
    }

    /// Öffnet eine gespeicherte Session in `spot` (`None`: Fensterauswahl, ohne offenes Fenster
    /// Fenster 1). Läuft sie anderswo, kommt erst die Rückfrage — außer bei einer
    /// Hintergrund-Session: `claude attach` startet keine zweite Instanz.
    fn open_stored(&mut self, entry: &Entry, spot: Option<Spot>) -> Result<()> {
        if entry.external && self.job_of(&entry.claude_id).is_none() {
            self.menu = Some(Menu::confirm_open(
                &entry.title,
                entry.claude_id.clone(),
                spot,
            ));
            return Ok(());
        }
        self.open_confirmed(entry.claude_id.clone(), spot)
    }

    /// Job-ID, falls `claude_id` gerade als Hintergrund-Session außerhalb von claude-tui läuft.
    fn job_of(&self, claude_id: &str) -> Option<&str> {
        if self.sessions.iter().any(|s| s.claude_id == claude_id) {
            return None;
        }
        self.claimed.get(claude_id)?.as_deref()
    }

    fn open_confirmed(&mut self, claude_id: String, spot: Option<Spot>) -> Result<()> {
        match spot {
            Some(spot) => self.resume_into(&claude_id, spot),
            None if self.windows.count() == 0 => self.resume_into(&claude_id, Spot::Window(0)),
            None => {
                self.open_pick(Purpose::Resume(claude_id));
                Ok(())
            }
        }
    }

    /// Setzt eine gespeicherte Session in ihrem Verzeichnis fort (bzw. verbindet sich mit ihr, wenn
    /// sie als Hintergrund-Session läuft) und zeigt sie in `spot`.
    fn resume_into(&mut self, claude_id: &str, spot: Spot) -> Result<()> {
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
        self.show_at(Pane::Session(id), spot);
        Ok(())
    }

    /// Verzeichnis gewählt: direkt ins Zielfenster, sonst (bei offenen Fenstern) Fensterauswahl.
    fn on_dir_chosen(&mut self, dir: PathBuf, target: Target) -> Result<()> {
        match target {
            Target::Window(slot) => {
                let id = self.spawn(&dir, Launch::New)?;
                self.place(id, slot);
            }
            Target::PickWindow if self.windows.count() == 0 => {
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
            Action::Place(slot) => self.place_for(purpose, Spot::Window(slot))?,
            Action::PlaceTemporary => self.place_for(purpose, Spot::Temporary)?,
            Action::RecentlyClosed => {
                let sessions = self
                    .closed_entries()
                    .into_iter()
                    .map(|e| (e.claude_id.clone(), e.title.clone()))
                    .collect();
                self.menu = Some(Menu::recently_closed(sessions));
            }
            Action::Reopen(index) => {
                let Some(Purpose::Reopen(claude_ids)) = purpose else {
                    return Ok(());
                };
                let entry = claude_ids.get(index).and_then(|claude_id| {
                    list::flatten(&self.groups)
                        .find(|e| e.claude_id == *claude_id)
                        .cloned()
                });
                let spot = self.focused_spot();
                match entry {
                    Some(entry) => self.open_entry(&entry, spot)?,
                    None => self.notice = Some("The session no longer exists".to_string()),
                }
            }
            Action::CloseWindow => {
                let slot = match self.windows.focus() {
                    Focus::Window(slot) => Some(slot),
                    Focus::List => self.selected_slot(),
                };
                if let Some(slot) = slot {
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
                if let Some(claude_id) = self.focused_claude_id() {
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
            Action::ToggleZoom => {
                let zoom = self.windows.zoomed().is_none();
                self.windows.set_zoomed(zoom);
                self.resize_windows();
            }
            Action::SetGroup => self.open_group_picker(),
            Action::EditNote => self.open_note_editor(),
            Action::ShowNotepad => self.show_from_menu(Pane::Notepad),
            Action::ShowTodo => self.show_from_menu(Pane::Todo),
            Action::ShowNotes => self.show_from_menu(Pane::Notes),
            Action::ToggleCollapsed => {
                if let Some(group) = self.selected_group() {
                    let (key, collapsed) = (group.heading.key(), self.is_collapsed(group));
                    self.set_collapsed(key, !collapsed);
                }
            }
            Action::CollapseAll(collapsed) => self.set_all_collapsed(collapsed),
            Action::Help => self.help = Some(Help::default()),
            Action::Quit => {
                let running = self
                    .sessions
                    .iter()
                    .filter(|s| s.status != Status::Exited)
                    .count();
                if running == 0 {
                    self.should_quit = true;
                } else {
                    self.menu = Some(Menu::confirm_quit(running));
                }
            }
            Action::ConfirmQuit => self.should_quit = true,
            Action::Update => {
                if let Some(available) = self.update.clone() {
                    self.notice = Some(format!("downloading v{} …", available.version));
                    self.update_install = Some(update::spawn_install(available));
                }
            }
        }
        Ok(())
    }

    /// Ziel der Fensterauswahl an `spot` zeigen.
    fn place_for(&mut self, purpose: Option<Purpose>, spot: Spot) -> Result<()> {
        match purpose {
            Some(Purpose::NewSession(dir)) => {
                let id = self.spawn(&dir, Launch::New)?;
                self.show_at(Pane::Session(id), spot);
            }
            Some(Purpose::Show(pane)) => self.show_at(pane, spot),
            Some(Purpose::Resume(claude_id)) => self.resume_into(&claude_id, spot)?,
            Some(
                Purpose::Delete(_)
                | Purpose::DeleteMany(_)
                | Purpose::OpenElsewhere(..)
                | Purpose::Reopen(_),
            )
            | None => {}
        }
        Ok(())
    }

    /// Titel der Fensterauswahl für einen Inhalt, vor „in“.
    fn pane_title(&self, pane: Pane) -> String {
        match pane {
            Pane::Session(id) => format!("Show {}", self.title_of(id)),
            Pane::Notepad => "Notepad".to_string(),
            Pane::Todo => "Todo list".to_string(),
            Pane::Notes => "Notes overview".to_string(),
        }
    }

    /// Laufende Session, auf die sich c/x beziehen: die im Fenster mit den Eingaben, sonst die in
    /// der Liste gewählte.
    fn context_session(&self) -> Option<SessionId> {
        match self.windows.input_pane() {
            Some(pane) => pane.session(),
            None => self.selected_running(),
        }
    }

    fn open_commands(&mut self) {
        self.menu = Some(Menu::commands(&Context {
            focus: self.windows.focus(),
            windows: self.windows.occupied(),
            selected_has_window: self.selected_slot().is_some(),
            panel_focused: self
                .windows
                .focused_pane()
                .is_some_and(|p| p.session().is_none()),
            has_selection: self.selected_entry().is_some(),
            update: self
                .update
                .as_ref()
                .filter(|_| self.update_install.is_none())
                .map(|u| u.version.as_str()),
            list_hidden: self.list_hidden,
            zoomed: self.windows.zoomed().is_some(),
            marked: self.marked.len(),
            collapsed: self.selected_group().map(|g| self.is_collapsed(g)),
            any_expanded: self.groups.iter().any(|g| !self.is_collapsed(g)),
            any_collapsed: self.groups.iter().any(|g| self.is_collapsed(g)),
            recently_closed: !self.closed_entries().is_empty(),
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

    /// Notiz-Editor für die Session mit Fokus bzw. die in der Liste gewählte.
    fn open_note_editor(&mut self) {
        let Some(target) = self.focused_claude_id() else {
            return;
        };
        let name = list::flatten(&self.groups)
            .find(|e| e.claude_id == target)
            .map_or(NEW_SESSION_TITLE, |e| e.title.as_str());
        let title = format!("Note for {}", menu::shorten(name, GROUP_TITLE_MAX));
        let text = self.note_of(&target).unwrap_or_default();
        self.note_editor = Some(NoteEditor::new(title, target.clone(), text));
    }

    /// Speichert die Notiz der Session (`None` = entfernen).
    fn apply_note(&mut self, target: String, note: Option<String>) {
        let message = match note {
            Some(note) => {
                self.notes.insert(target, note);
                "note saved"
            }
            None if self.notes.remove(&target).is_some() => "note removed",
            None => return,
        };
        self.flash_notice(message.to_string());
        self.save_state();
        self.resize_windows();
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
        let unread = self.unread.remove(claude_id);
        let noted = self.notes.remove(claude_id).is_some();
        if tagged || grouped || unread || noted {
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
            &Purpose::Show(pane) => format!("{} in", self.pane_title(pane)),
            Purpose::Resume(claude_id) => list::flatten(&self.groups)
                .find(|e| e.claude_id == *claude_id)
                .map_or_else(String::new, |e| format!("Open {} in", e.title)),
            Purpose::Delete(_)
            | Purpose::DeleteMany(_)
            | Purpose::OpenElsewhere(..)
            | Purpose::Reopen(_) => return,
        };
        self.menu = Some(Menu::pick_window(purpose, title, self.windows.occupied()));
    }

    fn move_focus(&mut self, direction: Direction) {
        match self.windows.focus() {
            Focus::List => match direction {
                Direction::Right => {
                    let slot = self
                        .selected_running()
                        .and_then(|id| self.windows.window_of(id))
                        .or_else(|| self.windows.first());
                    if let Some(slot) = slot {
                        self.set_focus(Focus::Window(slot));
                    }
                }
                // In der Liste springen Ctrl+↑/↓ zwischen den Gruppen bzw. Verzeichnissen.
                Direction::Up => self.jump_section(-1),
                Direction::Down => self.jump_section(1),
                _ => {}
            },
            Focus::Window(slot) => {
                // Gezoomt: erst aufheben, dann den Nachbarn im normalen Layout suchen.
                if self.windows.zoomed().is_some() {
                    self.windows.set_zoomed(false);
                    self.resize_windows();
                }
                let rects = layout::windows(
                    ui::windows_area(self.screen, self.list_hidden),
                    self.windows.occupied(),
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
    /// die Liste den Fokus, wandert er ins Fenster ihrer Session (sonst ins offene mit der
    /// kleinsten Nummer).
    fn set_list_hidden(&mut self, hidden: bool) {
        if hidden && self.windows.count() == 0 {
            return;
        }
        if hidden && self.windows.focus() == Focus::List {
            if let Some(slot) = self
                .selected_running()
                .and_then(|id| self.windows.window_of(id))
                .or_else(|| self.windows.first())
            {
                self.windows.set_focus(Focus::Window(slot));
            }
            self.sync_cursor();
        }
        self.list_hidden = hidden;
        self.resize_windows();
        self.save_state();
    }

    /// Fokus auf der Liste oder kein Fenster mehr offen: die Liste muss sichtbar sein.
    fn keep_list_reachable(&mut self) {
        if self.list_hidden && (self.windows.focus() == Focus::List || self.windows.count() == 0) {
            self.list_hidden = false;
            self.resize_windows();
        }
    }

    fn sync_cursor(&mut self) {
        match self.windows.focused_pane() {
            Some(Pane::Session(id)) => {
                if let Some(session) = self.session(id) {
                    self.cursor = Some(Cursor::Session(session.claude_id.clone()));
                }
            }
            Some(Pane::Notepad) => self.cursor = Some(Cursor::Notepad),
            Some(Pane::Todo) => self.cursor = Some(Cursor::Todo),
            Some(Pane::Notes) => self.cursor = Some(Cursor::Notes),
            None => {}
        }
    }

    fn place(&mut self, id: SessionId, slot: usize) {
        self.place_pane(Pane::Session(id), slot);
    }

    fn place_pane(&mut self, pane: Pane, slot: usize) {
        if self.windows.place(pane, slot) {
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

    /// Ctrl+Space für die Session mit Fokus bzw. die in der Liste gewählte — läuft sie nicht,
    /// direkt am gespeicherten Stand.
    fn toggle_unread(&mut self) {
        if self
            .windows
            .input_pane()
            .is_some_and(|p| p.session().is_none())
        {
            return;
        }
        if let Some(id) = self.context_session() {
            if let Some(session) = self.session_mut(id) {
                session.toggle_unread();
            }
            self.sync_unread();
        } else if let Some(claude_id) = self.selected_entry().map(|e| e.claude_id.clone()) {
            if !self.unread.remove(&claude_id) {
                self.unread.insert(claude_id);
            }
            self.save_state();
        }
    }

    /// Übernimmt den Ungelesen-Stand der laufenden Sessions; speichert nur bei einer Änderung.
    fn sync_unread(&mut self) {
        let mut changed = false;
        for session in &self.sessions {
            changed |= if session.is_unread() {
                self.unread.insert(session.claude_id.clone())
            } else {
                self.unread.remove(&session.claude_id)
            };
        }
        if changed {
            self.save_state();
        }
    }

    /// Vergisst Nummern, Gruppen und Ungelesen-Einträge zu Sessions, deren Protokoll es nicht mehr
    /// gibt (z. B. außerhalb gelöscht oder von Claude Code aufgeräumt). Erst, wenn alle Protokolle
    /// eingelesen sind — vorher fehlen noch welche.
    fn forget_deleted(&mut self, stored: &[claude_store::StoredSession]) {
        if self.store_load.is_some() {
            return;
        }
        let known = stored
            .iter()
            .map(|s| s.id.as_str())
            .chain(self.sessions.iter().map(|s| s.claude_id.as_str()));
        let per_session = state::PerSession {
            tags: &mut self.tags,
            groups: &mut self.group_of,
            unread: &mut self.unread,
            notes: &mut self.notes,
        };
        if per_session.forget_unknown(known) {
            self.save_state();
        }
    }

    /// Sessions in Fenstern, auch im temporären, samt Claude-ID.
    fn shown_sessions(&self) -> Vec<(SessionId, String)> {
        self.windows
            .slots()
            .iter()
            .copied()
            .chain([self.windows.temporary()])
            .flatten()
            .filter_map(Pane::session)
            .filter_map(|id| self.session(id).map(|s| (id, s.claude_id.clone())))
            .collect()
    }

    /// Merkt sich laufende Sessions, deren Fenster seit dem letzten Aufruf zuging (Ctrl+A r);
    /// einmal je Durchlauf der Ereignisschleife, damit jeder Weg erfasst ist — Schließen, Ersetzen,
    /// temporäres Fenster. Endet oder verschwindet der Prozess (auch beim Löschen), fällt die
    /// Session heraus.
    pub fn track_closed(&mut self) {
        let now = self.shown_sessions();
        let sessions = &self.sessions;
        let running = |claude_id: &str| {
            sessions
                .iter()
                .any(|s| s.claude_id == claude_id && s.status != Status::Exited)
        };
        closed::record(&mut self.closed, &self.shown, &now, running);
        self.shown = now;
    }

    /// Listeneinträge der zuletzt geschlossenen Fenster, neueste zuerst.
    fn closed_entries(&self) -> Vec<&Entry> {
        self.closed
            .iter()
            .filter_map(|id| list::flatten(&self.groups).find(|e| e.claude_id == *id))
            .collect()
    }

    fn save_state(&self) {
        let state = State {
            windows: self
                .windows
                .slots()
                .iter()
                .map(|slot| {
                    slot.and_then(Pane::session)
                        .and_then(|id| self.session(id))
                        .map(|s| s.claude_id.clone())
                        .unwrap_or_default()
                })
                .collect(),
            notepad_window: self.windows.slot_of(Pane::Notepad),
            todo_window: self.windows.slot_of(Pane::Todo),
            notes_window: self.windows.slot_of(Pane::Notes),
            focus: match self.windows.focus() {
                Focus::Window(slot) => Some(slot),
                Focus::List => None,
            },
            list_hidden: self.list_hidden,
            tags: self.tags.clone(),
            groups: self.group_of.clone(),
            recent_groups: self.recent_groups.clone(),
            superseded: self.superseded.clone(),
            collapsed: self.collapsed.clone(),
            unread: self.unread.clone(),
            notes: self.notes.clone(),
        };
        // Komfortfunktion: lässt sich der Zustand nicht speichern, läuft alles andere weiter.
        let _ = state::save(&state);
    }

    /// Jeder sichtbare Inhalt bekommt die Maße seines Fensters, gleich welcher Art; Inhalte ohne
    /// Fenster und beim Zoom verdeckte behalten ihre letzte Größe.
    fn resize_windows(&mut self) {
        for view in self.views() {
            let content = self.content_rect(view.pane, view.rect);
            self.pane_resize(view.pane, content);
        }
    }

    /// Neue Inhaltsfläche einer Quelle.
    fn pane_resize(&mut self, pane: Pane, content: Rect) {
        let (width, height) = (content.width, content.height);
        match pane {
            Pane::Session(id) => {
                if let Some(session) = self.session_mut(id) {
                    session.resize(height.max(1), width.max(1));
                }
            }
            Pane::Notepad => self.notepad.set_view(width.into(), height.into()),
            Pane::Todo => self.todo.set_view(width.into(), height.into()),
            Pane::Notes => self.note_list.set_view(width.into(), height.into()),
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
        let unread = self.unread.contains(&claude_id);
        let mut session = Session::spawn(
            id,
            claude_id,
            &self.command,
            &extra,
            dir,
            inner.height.max(1),
            inner.width.max(1),
            start_now,
        )?;
        session.set_unread(unread);
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

/// Sessions mit Notiz in der Reihenfolge der Sessionliste (Einträge der Notizübersicht).
fn note_items<'a>(
    groups: &'a [Group],
    notes: &'a BTreeMap<String, String>,
) -> Vec<notelist::Item<'a>> {
    list::flatten(groups)
        .filter_map(|e| Some((e.claude_id.as_str(), notes.get(&e.claude_id)?.as_str())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_click_finds_rows_but_nothing_outside() {
        let hits = ListHits {
            area: Rect::new(1, 1, 40, 10),
            rows: vec![
                (1, Cursor::Heading("dir:/x".into())),
                (2, Cursor::Session("a".into())),
                (3, Cursor::Session("b".into())),
            ],
        };
        assert_eq!(
            hits.at(Position::new(5, 3)),
            Some(&Cursor::Session("b".into()))
        );
        assert_eq!(
            hits.at(Position::new(5, 1)),
            Some(&Cursor::Heading("dir:/x".into()))
        );
        assert_eq!(hits.at(Position::new(5, 6)), None, "unter der Liste");
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
