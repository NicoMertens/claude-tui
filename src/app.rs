use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Margin, Position, Rect};

use crate::{
    claude_store::{self, Store},
    dirpick::{DirPicker, Outcome as DirOutcome, Target},
    keys,
    layout::{self, Direction},
    list::{self, Entry, Group, Running, NEW_SESSION_TITLE},
    menu::{Action, Context, Menu, Outcome, Purpose},
    recent,
    selection::{self, Cell, Selection},
    session::{Session, Status},
    state::{self, State},
    ui,
    update::{self, Available},
    windows::{Focus, SessionId, Windows, MAX_WINDOWS},
};

/// So oft wird `~/.claude/projects` nach neuen Sessions und Titeln durchsucht.
const STORE_INTERVAL: Duration = Duration::from_secs(2);
/// Zweiter Klick auf dieselbe Zelle innerhalb dieser Zeit = Doppelklick (Wort markieren).
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// So lange bleiben befristete Hinweise (z. B. „copied N characters“) stehen.
const FLASH_DURATION: Duration = Duration::from_secs(5);

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
}

pub struct App {
    /// Laufende Claude-Prozesse.
    pub sessions: Vec<Session>,
    pub windows: Windows,
    /// Sessionliste: gespeicherte und laufende Sessions, gruppiert nach Verzeichnis.
    pub groups: Vec<Group>,
    /// Claude-ID der in der Liste ausgewählten Session.
    pub cursor: Option<String>,
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
    command: String,
    /// Verzeichnis, aus dem claude-tui gestartet wurde.
    start_dir: PathBuf,
    /// Zuletzt genutzte Startverzeichnisse, neueste zuerst.
    recent: Vec<PathBuf>,
    store: Store,
    last_scan: Instant,
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
        let mut store = Store::new();
        store.refresh();
        let mut app = Self {
            sessions: Vec::new(),
            windows: Windows::default(),
            groups: Vec::new(),
            cursor: None,
            menu: None,
            dir_picker: None,
            notice: None,
            notice_expiry: None,
            selection: None,
            dragging: false,
            last_click: None,
            last_list_click: None,
            list_hits: RefCell::new(ListHits::default()),
            overlay: std::cell::Cell::new(None),
            update: None,
            update_check: update::spawn_check(),
            update_install: None,
            should_quit: false,
            command,
            start_dir: std::env::current_dir().context("current directory not readable")?,
            recent: recent::load(),
            store,
            last_scan: Instant::now(),
            screen,
            list_hidden: false,
            stacked: false,
            next_id: 0,
        };
        app.restore();
        if app.windows.len() == 0 {
            // Nichts wiederherzustellen: erste Session ohne Auswahl im Startverzeichnis.
            let start_dir = app.start_dir.clone();
            let id = app.spawn(&start_dir, Launch::New)?;
            app.place(id, 0);
        }
        app.rebuild_list();
        Ok(app)
    }

    /// Stellt das Fensterlayout des letzten Laufs wieder her; jede Session darin wird fortgesetzt.
    fn restore(&mut self) {
        let saved = state::load();
        self.stacked = saved.stacked;
        for claude_id in saved.windows {
            let Some(stored) = self.store.get(&claude_id) else {
                continue;
            };
            match self.spawn(&stored.cwd, Launch::Resume(claude_id)) {
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

    /// Rechtecke der Fenster in Nummernreihenfolge.
    pub fn window_rects(&self) -> Vec<Rect> {
        layout::windows(
            ui::windows_area(self.screen, self.list_hidden),
            self.windows.len(),
            self.stacked,
        )
    }

    pub fn tick(&mut self) {
        for session in &mut self.sessions {
            session.refresh();
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
                self.notice = Some(format!("\"{}\" has exited", self.title_of(id)));
            }
            self.remove_process(id);
        }
        self.expire_notice();
        self.poll_update();
        if self.last_scan.elapsed() >= STORE_INTERVAL {
            self.store.refresh();
            self.last_scan = Instant::now();
        }
        self.rebuild_list();
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

    fn rebuild_list(&mut self) {
        let running: Vec<Running> = self
            .sessions
            .iter()
            .map(|s| Running {
                id: s.id,
                claude_id: s.claude_id.clone(),
                cwd: s.cwd.clone(),
            })
            .collect();
        self.groups = list::build(self.store.sessions(), &running);
        if self.selected_entry().is_none() {
            self.cursor = list::flatten(&self.groups)
                .next()
                .map(|e| e.claude_id.clone());
        }
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
            self.menu = None;
            self.dir_picker = None;
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
                self.menu = None;
                self.dir_picker = None;
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
                    .and_then(|slot| self.window_rects().get(slot).copied())
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

    fn on_overlay(&self, position: Position) -> bool {
        let open = self.menu.is_some() || self.dir_picker.is_some();
        open && self.overlay.get().is_some_and(|r| r.contains(position))
    }

    /// Klick in die Sessionliste: Session auswählen und Fokus auf die Liste; ein zweiter Klick auf
    /// dieselbe Session kurz danach wirkt wie Enter.
    fn on_list_click(&mut self, claude_id: String) {
        self.menu = None;
        self.dir_picker = None;
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

    /// Fenster unter der Mausposition: Fensterindex, Session und Zelle im Fensterinhalt.
    fn window_at(&self, position: Position) -> Option<(usize, SessionId, Cell)> {
        self.window_rects()
            .into_iter()
            .zip(self.windows.slots().iter().copied())
            .enumerate()
            .find(|(_, (rect, _))| rect.contains(position))
            .map(|(slot, (rect, id))| (slot, id, cell_in(rect.inner(Margin::new(1, 1)), position)))
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

    fn on_list_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => match entry.running {
                Some(id) => match self.windows.window_of(id) {
                    Some(slot) => self.set_focus(Focus::Window(slot)),
                    None if self.windows.len() == 0 => self.place(id, 0),
                    None => self.open_pick(Purpose::Show(id)),
                },
                None if self.windows.len() == 0 => self.resume_into(&entry.claude_id, 0)?,
                None => self.open_pick(Purpose::Resume(entry.claude_id)),
            },
            KeyCode::Char(d @ '1'..='4') => {
                let slot = d as usize - '1' as usize;
                if slot <= self.windows.len() && slot < MAX_WINDOWS {
                    match entry.running {
                        Some(id) => self.place(id, slot),
                        None => self.resume_into(&entry.claude_id, slot)?,
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

    /// Setzt eine gespeicherte Session in ihrem Verzeichnis fort und legt sie in `slot`.
    fn resume_into(&mut self, claude_id: &str, slot: usize) -> Result<()> {
        let stored = self
            .store
            .get(claude_id)
            .context("session is no longer stored")?;
        let id = self.spawn(&stored.cwd, Launch::Resume(claude_id.to_string()))?;
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
                Some(Purpose::Delete(_)) | None => {}
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
            Action::ConfirmDelete => {
                if let Some(Purpose::Delete(claude_id)) = purpose {
                    self.delete_session(&claude_id)?;
                }
            }
            Action::Cancel => {}
            Action::Focus(focus) => self.set_focus(focus),
            Action::ToggleList => self.set_list_hidden(!self.list_hidden),
            Action::ToggleStacked => {
                self.stacked = !self.stacked;
                self.after_layout_change();
            }
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
        }));
    }

    /// Schließt Fenster und Prozess (falls laufend) und löscht das Protokoll endgültig.
    fn delete_session(&mut self, claude_id: &str) -> Result<()> {
        let running = self
            .sessions
            .iter()
            .find(|s| s.claude_id == claude_id)
            .map(|s| s.id);
        if let Some(id) = running {
            self.remove_process(id);
        }
        let path = self.store.path_of(claude_id);
        let result = path.map_or(Ok(()), |p| claude_store::delete(&p));
        self.store.refresh();
        self.rebuild_list();
        result
    }

    fn open_pick(&mut self, purpose: Purpose) {
        let title = match &purpose {
            Purpose::NewSession(_) => "New session in".to_string(),
            &Purpose::Show(id) => format!("Show {} in", self.title_of(id)),
            Purpose::Resume(_) => self
                .selected_entry()
                .map_or_else(String::new, |e| format!("Open {} in", e.title)),
            Purpose::Delete(_) => return,
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
            Focus::Window(slot) => match layout::neighbor(&self.window_rects(), slot, direction) {
                Some(next) => self.set_focus(Focus::Window(next)),
                None if direction == Direction::Left => self.set_focus(Focus::List),
                None => {}
            },
        }
    }

    /// Setzt den Fokus; bei einem Fenster folgt die Listenauswahl dessen Session.
    fn set_focus(&mut self, focus: Focus) {
        self.windows.set_focus(focus);
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
        self.windows.remove_session(id);
        self.sessions.retain(|s| s.id != id);
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
        };
        // Komfortfunktion: lässt sich der Zustand nicht speichern, läuft alles andere weiter.
        let _ = state::save(&state);
    }

    /// Jede sichtbare Session bekommt die Innenmaße ihres Fensters; Hintergrund-Sessions behalten
    /// ihre letzte Größe.
    fn resize_windows(&mut self) {
        let rects = self.window_rects();
        let slots = self.windows.slots().to_vec();
        for (id, rect) in slots.into_iter().zip(rects) {
            let inner = rect.inner(Margin::new(1, 1));
            if let Some(session) = self.session_mut(id) {
                session.resize(inner.height.max(1), inner.width.max(1));
            }
        }
    }

    fn spawn(&mut self, dir: &Path, launch: Launch) -> Result<SessionId> {
        let (claude_id, flag) = match launch {
            Launch::New => (uuid::Uuid::new_v4().to_string(), "--session-id"),
            Launch::Resume(claude_id) => (claude_id, "--resume"),
        };
        let command = format!("{} {flag} {claude_id}", self.command);
        self.next_id += 1;
        let id = self.next_id;
        let inner = ui::windows_area(self.screen, self.list_hidden).inner(Margin::new(1, 1));
        let session = Session::spawn(
            id,
            claude_id,
            &command,
            dir,
            inner.height.max(1),
            inner.width.max(1),
        )?;
        self.sessions.push(session);
        recent::remember(&mut self.recent, dir);
        // Die Liste ist Komfort: lässt sie sich nicht speichern, läuft die Session trotzdem.
        let _ = recent::save(&self.recent);
        self.rebuild_list();
        Ok(id)
    }
}

/// Zelle im Fensterinhalt `inner` unter der Mausposition, an den Rand geklemmt.
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
