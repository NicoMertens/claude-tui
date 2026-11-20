//! Hinweis auf Sessions, die auf dich warten: Übergänge erkennen.

use crate::{session::Status, windows::SessionId};

/// Abschalten der Terminal-Glocke.
const NO_BELL_ENV: &str = "CLAUDE_TUI_NO_BELL";

pub fn bell_enabled() -> bool {
    std::env::var_os(NO_BELL_ENV).is_none()
}

/// Sessions, die gerade auf „waiting“ gewechselt sind — außer der im fokussierten Fenster,
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
}
