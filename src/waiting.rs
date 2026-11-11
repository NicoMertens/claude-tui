//! Hinweis auf Sessions, die auf dich warten: Übergänge erkennen und reihum anspringen.

use crate::{session::Status, windows::SessionId};

/// Abschalten der Terminal-Glocke.
const NO_BELL_ENV: &str = "CLAUDE_TUI_NO_BELL";

pub fn bell_enabled() -> bool {
    std::env::var_os(NO_BELL_ENV).is_none()
}

/// Sessions, die gerade auf „waiting for you“ gewechselt sind — außer der im fokussierten Fenster,
/// die man ohnehin sieht. `changes` liefert je Session (ID, Zustand vorher, Zustand jetzt).
pub fn newly_waiting(
    changes: impl IntoIterator<Item = (SessionId, Status, Status)>,
    focused: Option<SessionId>,
) -> Vec<SessionId> {
    changes
        .into_iter()
        .filter(|&(id, before, now)| {
            before != Status::Waiting && now == Status::Waiting && Some(id) != focused
        })
        .map(|(id, _, _)| id)
        .collect()
}

/// Nächste wartende Session nach `current` in Listenreihenfolge, am Ende wieder von vorn.
pub fn next_waiting(
    order: &[SessionId],
    is_waiting: impl Fn(SessionId) -> bool,
    current: Option<SessionId>,
) -> Option<SessionId> {
    let start = current
        .and_then(|c| order.iter().position(|&id| id == c))
        .map_or(0, |i| i + 1);
    (0..order.len())
        .map(|offset| order[(start + offset) % order.len()])
        .find(|&id| is_waiting(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Status::{Idle, Waiting, Working};

    #[test]
    fn only_the_transition_counts() {
        let changes = [
            (1, Working, Waiting),
            (2, Waiting, Waiting),
            (3, Idle, Working),
        ];
        assert_eq!(newly_waiting(changes, None), [1]);
    }

    #[test]
    fn focused_window_gets_no_hint() {
        let changes = [(1, Working, Waiting), (2, Idle, Waiting)];
        assert_eq!(newly_waiting(changes, Some(1)), [2]);
    }

    #[test]
    fn next_waiting_goes_round_in_list_order() {
        let order = [5, 3, 8, 1];
        let waiting = |id| id == 3 || id == 1;
        assert_eq!(next_waiting(&order, waiting, None), Some(3));
        assert_eq!(next_waiting(&order, waiting, Some(3)), Some(1));
        assert_eq!(next_waiting(&order, waiting, Some(1)), Some(3));
        assert_eq!(next_waiting(&order, waiting, Some(8)), Some(1));
        // Aktuelle Session nicht in der Liste: von vorn.
        assert_eq!(next_waiting(&order, waiting, Some(42)), Some(3));
    }

    #[test]
    fn single_waiting_session_is_found_again() {
        let order = [5, 3];
        assert_eq!(next_waiting(&order, |id| id == 3, Some(3)), Some(3));
        assert_eq!(next_waiting(&order, |_| false, Some(3)), None);
        assert_eq!(next_waiting(&[], |_| true, None), None);
    }
}
