//! Zuletzt geschlossene Fenster (Ctrl+A r): laufende Claude-Sessions, deren Fenster zuging —
//! geschlossen, durch einen anderen Inhalt ersetzt oder als temporäres Fenster zugemacht. Neueste
//! zuerst, ohne Doppelte und ohne Sessions, die gerade ein Fenster haben. Endet der Prozess, fällt
//! die Session heraus; deshalb nicht in `state.json` (nach einem Neustart läuft keine mehr).

use crate::windows::SessionId;

/// Höchstzahl der gemerkten Sessions; passt auf die Tasten 1–9 im Menü.
pub const MAX: usize = 9;

/// Gleicht `closed` mit den Sessions in Fenstern ab: was in `before` stand und in `now` fehlt,
/// kommt nach vorn; was in `now` steht oder nicht mehr `running` ist, fällt heraus.
pub fn record(
    closed: &mut Vec<String>,
    before: &[(SessionId, String)],
    now: &[(SessionId, String)],
    running: impl Fn(&str) -> bool,
) {
    closed.retain(|c| running(c) && !now.iter().any(|(_, shown)| shown == c));
    for (id, claude_id) in before {
        if now.iter().any(|(shown, _)| shown == id) || !running(claude_id) {
            continue;
        }
        closed.retain(|c| c != claude_id);
        closed.insert(0, claude_id.clone());
    }
    closed.truncate(MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(list: &[(SessionId, &str)]) -> Vec<(SessionId, String)> {
        list.iter().map(|&(id, c)| (id, c.to_string())).collect()
    }

    #[test]
    fn closed_window_comes_first_and_reopened_one_leaves() {
        let mut closed = vec!["old".to_string()];
        let before = shown(&[(1, "a"), (2, "b")]);
        record(&mut closed, &before, &shown(&[(2, "b")]), |_| true);
        assert_eq!(closed, ["a", "old"]);
        // Wieder geöffnet: gehört nicht mehr zu den geschlossenen.
        let now = shown(&[(2, "b"), (3, "old")]);
        record(&mut closed, &shown(&[(2, "b")]), &now, |_| true);
        assert_eq!(closed, ["a"]);
    }

    #[test]
    fn only_running_sessions_and_list_is_capped() {
        let mut closed: Vec<String> = (0..MAX).map(|i| i.to_string()).collect();
        closed.push("ended".into());
        // Fenster ging zu, weil der Prozess endete (oder gelöscht): nicht merken.
        let before = shown(&[(1, "gone"), (2, "x")]);
        record(&mut closed, &before, &[], |c| {
            c != "gone" && c != "ended" && c != "3"
        });
        assert_eq!(closed.len(), MAX);
        assert_eq!(closed[0], "x");
        assert!(!closed
            .iter()
            .any(|c| c == "gone" || c == "ended" || c == "3"));
    }
}
