//! Welche Session in welchem Fenster liegt und wo der Fokus ist — ohne PTYs, damit testbar.

pub const MAX_WINDOWS: usize = 4;

pub type SessionId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    /// Fensterindex, 0 = Fenster 1.
    Window(usize),
}

/// Fenster in Positionsreihenfolge. Wird eines geschlossen, rücken die übrigen nach.
#[derive(Debug)]
pub struct Windows {
    slots: Vec<SessionId>,
    focus: Focus,
    /// Das Fenster mit Fokus füllt die ganze Fensterfläche (wie tmux-Zoom). Endet mit jeder
    /// Änderung von Fokus oder Fensterbelegung.
    zoomed: bool,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            focus: Focus::List,
            zoomed: false,
        }
    }
}

impl Windows {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn slots(&self) -> &[SessionId] {
        &self.slots
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Session im Fenster mit Fokus.
    pub fn focused_session(&self) -> Option<SessionId> {
        match self.focus {
            Focus::Window(i) => self.slots.get(i).copied(),
            Focus::List => None,
        }
    }

    pub fn set_focus(&mut self, focus: Focus) {
        match focus {
            Focus::Window(i) if i >= self.slots.len() => {}
            _ if focus == self.focus => {}
            _ => {
                self.focus = focus;
                self.zoomed = false;
            }
        }
    }

    /// Gezoomtes Fenster, falls eines gezoomt ist.
    pub fn zoomed(&self) -> Option<usize> {
        match self.focus {
            Focus::Window(i) if self.zoomed => Some(i),
            _ => None,
        }
    }

    /// Zoomt das Fenster mit Fokus bzw. hebt den Zoom auf. Zoomen geht nur mit Fokus auf einem
    /// Fenster und mindestens zwei offenen Fenstern.
    pub fn set_zoomed(&mut self, zoomed: bool) {
        self.zoomed = zoomed && matches!(self.focus, Focus::Window(_)) && self.slots.len() >= 2;
    }

    pub fn window_of(&self, id: SessionId) -> Option<usize> {
        self.slots.iter().position(|&s| s == id)
    }

    /// Zeigt `id` in Fenster `slot` und setzt den Fokus dorthin.
    ///
    /// `slot == len()` öffnet ein neues Fenster. Ist die Session schon in einem anderen Fenster
    /// sichtbar, tauscht sie den Platz mit dem Zielfenster (bzw. wandert ins neue Fenster). Eine
    /// verdrängte Session läuft ohne Fenster weiter.
    pub fn place(&mut self, id: SessionId, slot: usize) -> bool {
        let len = self.slots.len();
        if slot > len || slot >= MAX_WINDOWS {
            return false;
        }
        match self.window_of(id) {
            Some(current) if current == slot => {}
            Some(current) if slot < len => self.slots.swap(current, slot),
            Some(current) => {
                self.slots.remove(current);
                self.slots.push(id);
            }
            None if slot < len => self.slots[slot] = id,
            None => self.slots.push(id),
        }
        self.zoomed = false;
        if let Some(i) = self.window_of(id) {
            self.focus = Focus::Window(i);
        }
        true
    }

    /// Schließt ein Fenster; die Session läuft weiter. Liegt der Fokus darauf, geht er an das
    /// Fenster, das nachrückt, sonst an das letzte, ohne Fenster an die Liste.
    pub fn close(&mut self, slot: usize) {
        if slot >= self.slots.len() {
            return;
        }
        self.slots.remove(slot);
        self.zoomed = false;
        if let Focus::Window(f) = self.focus {
            self.focus = if self.slots.is_empty() {
                Focus::List
            } else if f > slot {
                Focus::Window(f - 1)
            } else {
                Focus::Window(f.min(self.slots.len() - 1))
            };
        }
    }

    /// Entfernt die Session aus ihrem Fenster, falls sie eines hat.
    pub fn remove_session(&mut self, id: SessionId) {
        if let Some(slot) = self.window_of(id) {
            self.close(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(ids: &[SessionId]) -> Windows {
        let mut w = Windows::default();
        for (slot, &id) in ids.iter().enumerate() {
            assert!(w.place(id, slot));
        }
        w
    }

    #[test]
    fn new_window_only_directly_after_last() {
        let mut w = with(&[10, 11]);
        assert!(!w.place(12, 3));
        assert!(w.place(12, 2));
        assert_eq!(w.slots(), &[10, 11, 12]);
        assert_eq!(w.focus(), Focus::Window(2));
    }

    #[test]
    fn at_most_four_windows() {
        let mut w = with(&[1, 2, 3, 4]);
        assert!(!w.place(5, 4));
        assert_eq!(w.len(), 4);
    }

    #[test]
    fn replacing_sends_old_session_to_background() {
        let mut w = with(&[1, 2]);
        assert!(w.place(3, 0));
        assert_eq!(w.slots(), &[3, 2]);
        assert_eq!(w.window_of(1), None);
    }

    #[test]
    fn visible_session_swaps_places() {
        let mut w = with(&[1, 2, 3]);
        assert!(w.place(3, 0));
        assert_eq!(w.slots(), &[3, 2, 1]);
        assert_eq!(w.focus(), Focus::Window(0));
    }

    #[test]
    fn visible_session_moves_to_new_window() {
        let mut w = with(&[1, 2, 3]);
        assert!(w.place(1, 3));
        assert_eq!(w.slots(), &[2, 3, 1]);
        assert_eq!(w.focus(), Focus::Window(2));
    }

    #[test]
    fn closing_moves_windows_up_and_keeps_focus_sensible() {
        let mut w = with(&[1, 2, 3]);
        w.set_focus(Focus::Window(2));
        w.close(0);
        assert_eq!(w.slots(), &[2, 3]);
        assert_eq!(w.focus(), Focus::Window(1));

        w.close(1);
        assert_eq!(w.focus(), Focus::Window(0));
        w.close(0);
        assert_eq!(w.focus(), Focus::List);
    }

    #[test]
    fn zoom_needs_window_focus_and_two_windows() {
        let mut w = with(&[1]);
        w.set_zoomed(true);
        assert_eq!(w.zoomed(), None, "ein Fenster");
        let mut w = with(&[1, 2]);
        w.set_focus(Focus::List);
        w.set_zoomed(true);
        assert_eq!(w.zoomed(), None, "Fokus auf der Liste");
        w.set_focus(Focus::Window(0));
        w.set_zoomed(true);
        assert_eq!(w.zoomed(), Some(0));
        w.set_zoomed(false);
        assert_eq!(w.zoomed(), None);
    }

    #[test]
    fn zoom_ends_with_focus_change_but_not_on_same_focus() {
        let mut w = with(&[1, 2]);
        w.set_zoomed(true);
        w.set_focus(Focus::Window(1));
        assert_eq!(w.zoomed(), Some(1));
        w.set_focus(Focus::Window(0));
        assert_eq!(w.zoomed(), None);
        w.set_zoomed(true);
        w.set_focus(Focus::List);
        assert_eq!(w.zoomed(), None);
    }

    #[test]
    fn zoom_ends_when_windows_change() {
        let mut w = with(&[1, 2, 3]);
        w.set_zoomed(true);
        assert!(w.place(4, 0));
        assert_eq!(w.zoomed(), None, "ersetzt");
        w.set_zoomed(true);
        w.close(0);
        assert_eq!(w.zoomed(), None, "geschlossen");
        w.set_zoomed(true);
        assert!(w.place(5, 2));
        assert_eq!(w.zoomed(), None, "neues Fenster");
    }

    #[test]
    fn closing_other_window_keeps_list_focus() {
        let mut w = with(&[1, 2]);
        w.set_focus(Focus::List);
        w.remove_session(1);
        assert_eq!(w.slots(), &[2]);
        assert_eq!(w.focus(), Focus::List);
    }
}
