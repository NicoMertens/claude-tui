use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::{keys, session::Session};

pub struct App {
    pub sessions: Vec<Session>,
    pub active: usize,
    pub prefix_armed: bool,
    pub should_quit: bool,
    command: String,
    pane: (u16, u16),
    next_id: usize,
}

impl App {
    pub fn new(command: String, pane: (u16, u16)) -> Result<Self> {
        let mut app = Self {
            sessions: Vec::new(),
            active: 0,
            prefix_armed: false,
            should_quit: false,
            command,
            pane,
            next_id: 0,
        };
        app.spawn()?;
        Ok(app)
    }

    pub fn tick(&mut self) {
        for session in &mut self.sessions {
            session.refresh();
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.prefix_armed {
            self.prefix_armed = false;
            return self.on_command(key);
        }
        if keys::is_prefix(&key) {
            self.prefix_armed = true;
            return Ok(());
        }
        if let Some(session) = self.sessions.get_mut(self.active) {
            if let Some(bytes) = keys::to_bytes(&key, session.application_cursor()) {
                session.write(&bytes)?;
            }
        }
        Ok(())
    }

    pub fn on_paste(&mut self, text: &str) -> Result<()> {
        if let Some(session) = self.sessions.get_mut(self.active) {
            session.paste(text)?;
        }
        Ok(())
    }

    pub fn resize(&mut self, pane: (u16, u16)) {
        self.pane = pane;
        for session in &mut self.sessions {
            session.resize(pane.0, pane.1);
        }
    }

    fn on_command(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            _ if keys::is_prefix(&key) => {
                if let Some(session) = self.sessions.get_mut(self.active) {
                    session.write(keys::PREFIX_BYTES)?;
                }
            }
            KeyCode::Char('c') => self.spawn()?,
            KeyCode::Char('x') => self.close_active(),
            KeyCode::Char('n') | KeyCode::Char('j') | KeyCode::Down => self.select_relative(1),
            KeyCode::Char('p') | KeyCode::Char('k') | KeyCode::Up => self.select_relative(-1),
            KeyCode::Char(d @ '1'..='9') => {
                let index = d as usize - '1' as usize;
                if index < self.sessions.len() {
                    self.active = index;
                }
            }
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
        Ok(())
    }

    fn spawn(&mut self) -> Result<()> {
        self.next_id += 1;
        let name = format!("claude {}", self.next_id);
        let session = Session::spawn(name, &self.command, self.pane.0, self.pane.1)?;
        self.sessions.push(session);
        self.active = self.sessions.len() - 1;
        Ok(())
    }

    fn close_active(&mut self) {
        if self.active < self.sessions.len() {
            self.sessions.remove(self.active);
        }
        if self.sessions.is_empty() {
            self.should_quit = true;
        } else if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
    }

    fn select_relative(&mut self, delta: isize) {
        let len = self.sessions.len() as isize;
        if len > 0 {
            self.active = (self.active as isize + delta).rem_euclid(len) as usize;
        }
    }
}
