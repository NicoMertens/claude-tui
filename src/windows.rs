//! Welche Session in welchem Fenster liegt und wo der Fokus ist — ohne PTYs, damit testbar.

pub const MAX_WINDOWS: usize = 4;

pub type SessionId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    /// Fensterindex, 0 = Fenster 1.
    Window(usize),
}

/// Fenster mit festen Plätzen: Fenster n bleibt Fenster n, auch wenn andere fehlen oder
/// geschlossen werden (Anordnung in [`crate::layout`]).
#[derive(Debug)]
pub struct Windows {
    slots: [Option<SessionId>; MAX_WINDOWS],
    focus: Focus,
    /// Das Fenster mit Fokus füllt die ganze Fensterfläche (wie tmux-Zoom). Endet mit jeder
    /// Änderung von Fokus oder Fensterbelegung.
    zoomed: bool,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            slots: [None; MAX_WINDOWS],
            focus: Focus::List,
            zoomed: false,
        }
    }
}

impl Windows {
    /// Anzahl der offenen Fenster.
    pub fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// Session je Fensterplatz (`None` = Fenster nicht offen).
    pub fn slots(&self) -> &[Option<SessionId>; MAX_WINDOWS] {
        &self.slots
    }

    /// Welche Fensterplätze belegt sind.
    pub fn occupied(&self) -> [bool; MAX_WINDOWS] {
        self.slots.map(|s| s.is_some())
    }

    /// Offene Fenster als (Fensterindex, Session) in Nummernreihenfolge.
    pub fn open(&self) -> impl Iterator<Item = (usize, SessionId)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|id| (i, id)))
    }

    pub fn session(&self, slot: usize) -> Option<SessionId> {
        self.slots.get(slot).copied().flatten()
    }

    /// Offenes Fenster mit der kleinsten Nummer.
    pub fn first(&self) -> Option<usize> {
        self.open().next().map(|(i, _)| i)
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Session im Fenster mit Fokus.
    pub fn focused_session(&self) -> Option<SessionId> {
        match self.focus {
            Focus::Window(i) => self.session(i),
            Focus::List => None,
        }
    }

    pub fn set_focus(&mut self, focus: Focus) {
        match focus {
            Focus::Window(i) if self.session(i).is_none() => {}
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
        self.zoomed = zoomed && matches!(self.focus, Focus::Window(_)) && self.count() >= 2;
    }

    pub fn window_of(&self, id: SessionId) -> Option<usize> {
        self.slots.iter().position(|&s| s == Some(id))
    }

    /// Zeigt `id` in Fenster `slot` und setzt den Fokus dorthin.
    ///
    /// Jedes der vier Fenster lässt sich öffnen, auch wenn Fenster mit kleinerer Nummer fehlen.
    /// Ist die Session schon in einem anderen Fenster sichtbar, tauscht sie den Platz mit dem
    /// Zielfenster (ein leeres Zielfenster lässt ihren alten Platz leer). Eine verdrängte Session
    /// läuft ohne Fenster weiter.
    pub fn place(&mut self, id: SessionId, slot: usize) -> bool {
        if slot >= MAX_WINDOWS {
            return false;
        }
        match self.window_of(id) {
            Some(current) => self.slots.swap(current, slot),
            None => self.slots[slot] = Some(id),
        }
        self.zoomed = false;
        self.focus = Focus::Window(slot);
        true
    }

    /// Schließt ein Fenster; die Session läuft weiter, die übrigen Fenster behalten ihre Nummer.
    /// Lag der Fokus darauf, geht er an das nächste offene Fenster mit höherer Nummer, sonst an
    /// das letzte, ohne Fenster an die Liste.
    pub fn close(&mut self, slot: usize) {
        if self.session(slot).is_none() {
            return;
        }
        self.slots[slot] = None;
        self.zoomed = false;
        if self.focus == Focus::Window(slot) {
            let open: Vec<usize> = self.open().map(|(i, _)| i).collect();
            self.focus = open
                .iter()
                .find(|&&i| i > slot)
                .or(open.last())
                .map_or(Focus::List, |&i| Focus::Window(i));
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

    fn ids(w: &Windows) -> [Option<SessionId>; MAX_WINDOWS] {
        *w.slots()
    }

    #[test]
    fn any_window_opens_even_with_gaps() {
        let mut w = with(&[10, 11]);
        assert!(w.place(12, 3));
        assert_eq!(ids(&w), [Some(10), Some(11), None, Some(12)]);
        assert_eq!(w.count(), 3);
        assert_eq!(w.focus(), Focus::Window(3));
        assert!(!w.place(13, 4));
    }

    #[test]
    fn replacing_sends_old_session_to_background() {
        let mut w = with(&[1, 2]);
        assert!(w.place(3, 0));
        assert_eq!(ids(&w), [Some(3), Some(2), None, None]);
        assert_eq!(w.window_of(1), None);
    }

    #[test]
    fn visible_session_swaps_places() {
        let mut w = with(&[1, 2, 3]);
        assert!(w.place(3, 0));
        assert_eq!(ids(&w), [Some(3), Some(2), Some(1), None]);
        assert_eq!(w.focus(), Focus::Window(0));
    }

    #[test]
    fn visible_session_moves_to_empty_window() {
        let mut w = with(&[1, 2, 3]);
        assert!(w.place(1, 3));
        assert_eq!(ids(&w), [None, Some(2), Some(3), Some(1)]);
        assert_eq!(w.focus(), Focus::Window(3));
    }

    #[test]
    fn closing_keeps_numbers_and_moves_focus_on() {
        let mut w = with(&[1, 2, 3]);
        w.set_focus(Focus::Window(0));
        w.close(0);
        assert_eq!(ids(&w), [None, Some(2), Some(3), None]);
        assert_eq!(w.focus(), Focus::Window(1));
        assert_eq!(w.first(), Some(1));

        w.set_focus(Focus::Window(2));
        w.close(2);
        assert_eq!(w.focus(), Focus::Window(1), "kein höheres: das letzte");
        w.close(1);
        assert_eq!(w.focus(), Focus::List);
        assert_eq!(w.first(), None);
    }

    #[test]
    fn focus_only_on_open_windows() {
        let mut w = with(&[1]);
        w.set_focus(Focus::List);
        w.set_focus(Focus::Window(2));
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
        assert!(w.place(5, 3));
        assert_eq!(w.zoomed(), None, "neues Fenster");
    }

    #[test]
    fn closing_other_window_keeps_list_focus() {
        let mut w = with(&[1, 2]);
        w.set_focus(Focus::List);
        w.remove_session(1);
        assert_eq!(ids(&w), [None, Some(2), None, None]);
        assert_eq!(w.focus(), Focus::List);
    }
}
