use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Margin, Rect};

use crate::{
    keys,
    layout::{self, Direction},
    menu::{Action, Context, Menu, Outcome, Purpose},
    session::{Session, Status},
    windows::{Focus, SessionId, Windows},
};

pub struct App {
    pub sessions: Vec<Session>,
    pub windows: Windows,
    /// Ausgewählte Zeile der Sessionliste.
    pub cursor: usize,
    pub menu: Option<Menu>,
    pub should_quit: bool,
    command: String,
    /// Fläche rechts neben der Sessionliste, auf die die Fenster verteilt werden.
    area: Rect,
    next_id: SessionId,
}

impl App {
    pub fn new(command: String, area: Rect) -> Result<Self> {
        let mut app = Self {
            sessions: Vec::new(),
            windows: Windows::default(),
            cursor: 0,
            menu: None,
            should_quit: false,
            command,
            area,
            next_id: 0,
        };
        let id = app.spawn()?;
        app.place(id, 0);
        Ok(app)
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    fn selected_id(&self) -> Option<SessionId> {
        self.sessions.get(self.cursor).map(|s| s.id)
    }

    /// Rechtecke der Fenster in Nummernreihenfolge.
    pub fn window_rects(&self) -> Vec<Rect> {
        layout::windows(self.area, self.windows.len())
    }

    pub fn tick(&mut self) {
        for session in &mut self.sessions {
            session.refresh();
        }
        // Beendete Sessions verlieren ihr Fenster, bleiben aber als „beendet“ in der Liste.
        let exited: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|s| s.status == Status::Exited && self.windows.window_of(s.id).is_some())
            .map(|s| s.id)
            .collect();
        if !exited.is_empty() {
            for id in exited {
                self.windows.remove_session(id);
            }
            self.after_layout_change();
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(direction) = keys::focus_direction(&key) {
            self.menu = None;
            self.move_focus(direction);
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
            None => self.on_list_key(key),
        }
        Ok(())
    }

    pub fn on_paste(&mut self, text: &str) -> Result<()> {
        if let Some(id) = self.windows.focused_session() {
            if let Some(session) = self.session_mut(id) {
                session.paste(text)?;
            }
        }
        Ok(())
    }

    pub fn resize(&mut self, area: Rect) {
        self.area = area;
        self.resize_windows();
    }

    fn on_list_key(&mut self, key: KeyEvent) {
        let Some(id) = self.selected_id() else {
            return;
        };
        match key.code {
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down if self.cursor + 1 < self.sessions.len() => self.cursor += 1,
            KeyCode::Enter => match self.windows.window_of(id) {
                Some(slot) => self.set_focus(Focus::Window(slot)),
                None if self.is_exited(id) => {}
                None if self.windows.len() == 0 => self.place(id, 0),
                None => self.open_pick(Purpose::Show(id)),
            },
            KeyCode::Char(d @ '1'..='4') if !self.is_exited(id) => {
                self.place(id, d as usize - '1' as usize);
            }
            _ => {}
        }
    }

    fn is_exited(&self, id: SessionId) -> bool {
        self.session(id).is_some_and(|s| s.status == Status::Exited)
    }

    fn run(&mut self, action: Action, purpose: Option<Purpose>) -> Result<()> {
        match action {
            Action::NewSession => match self.windows.focus() {
                Focus::Window(slot) => {
                    let id = self.spawn()?;
                    self.place(id, slot);
                }
                Focus::List if self.windows.len() == 0 => {
                    let id = self.spawn()?;
                    self.place(id, 0);
                }
                Focus::List => self.open_pick(Purpose::NewSession),
            },
            Action::Place(slot) => match purpose {
                Some(Purpose::NewSession) => {
                    let id = self.spawn()?;
                    self.place(id, slot);
                }
                Some(Purpose::Show(id)) => self.place(id, slot),
                None => {}
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
            Action::KillSession => {
                if let Some(id) = self.context_session() {
                    self.kill(id);
                }
            }
            Action::Focus(focus) => self.set_focus(focus),
            Action::SendPrefix => {
                if let Some(id) = self.windows.focused_session() {
                    if let Some(session) = self.session_mut(id) {
                        session.write(keys::PREFIX_BYTES)?;
                    }
                }
            }
            Action::Quit => self.should_quit = true,
        }
        Ok(())
    }

    /// Session, auf die sich c/x beziehen: die im Fenster mit Fokus, sonst die in der Liste gewählte.
    fn context_session(&self) -> Option<SessionId> {
        self.windows
            .focused_session()
            .or_else(|| self.selected_id())
    }

    fn window_names(&self) -> Vec<String> {
        self.windows
            .slots()
            .iter()
            .map(|&id| {
                self.session(id)
                    .map_or_else(String::new, |s| s.name.clone())
            })
            .collect()
    }

    fn open_commands(&mut self) {
        let windows = self.window_names();
        let selected_has_window = self
            .selected_id()
            .is_some_and(|id| self.windows.window_of(id).is_some());
        self.menu = Some(Menu::commands(&Context {
            focus: self.windows.focus(),
            windows: &windows,
            selected_has_window,
            has_sessions: !self.sessions.is_empty(),
        }));
    }

    fn open_pick(&mut self, purpose: Purpose) {
        let title = match purpose {
            Purpose::NewSession => "Neue Session in".to_string(),
            Purpose::Show(id) => self
                .session(id)
                .map_or_else(String::new, |s| format!("{} zeigen in", s.name)),
        };
        self.menu = Some(Menu::pick_window(purpose, title, &self.window_names()));
    }

    fn move_focus(&mut self, direction: Direction) {
        match self.windows.focus() {
            Focus::List => {
                if direction == Direction::Right && self.windows.len() > 0 {
                    let slot = self
                        .selected_id()
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
        self.sync_cursor();
    }

    fn sync_cursor(&mut self) {
        if let Some(id) = self.windows.focused_session() {
            if let Some(index) = self.sessions.iter().position(|s| s.id == id) {
                self.cursor = index;
            }
        }
    }

    fn place(&mut self, id: SessionId, slot: usize) {
        if self.windows.place(id, slot) {
            self.after_layout_change();
        }
    }

    fn kill(&mut self, id: SessionId) {
        self.windows.remove_session(id);
        // Drop beendet den Prozess.
        self.sessions.retain(|s| s.id != id);
        self.cursor = self.cursor.min(self.sessions.len().saturating_sub(1));
        self.after_layout_change();
    }

    fn after_layout_change(&mut self) {
        self.resize_windows();
        self.sync_cursor();
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

    fn spawn(&mut self) -> Result<SessionId> {
        self.next_id += 1;
        let id = self.next_id;
        let inner = self.area.inner(Margin::new(1, 1));
        let session = Session::spawn(
            id,
            format!("claude {id}"),
            &self.command,
            inner.height.max(1),
            inner.width.max(1),
        )?;
        self.sessions.push(session);
        Ok(id)
    }
}
