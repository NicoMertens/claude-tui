use std::time::Duration;

use ratatui::style::Color;

use crate::session::Status;

/// Claude-Orange, die Akzentfarbe des Themes.
pub const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
/// Rahmen und Titel des Bereichs mit Fokus.
pub const FOCUS: Color = Color::Rgb(0xF0, 0xA2, 0x7F);
/// Rahmen und Titel aller anderen Bereiche.
pub const UNFOCUS: Color = Color::Rgb(0x8C, 0x4A, 0x32);
pub const BG: Color = Color::Rgb(0x1A, 0x19, 0x18);
/// Hintergrund der ausgewählten Zeile in der Sessionliste.
pub const SELECTED_BG: Color = Color::Rgb(0x33, 0x2D, 0x29);
pub const TEXT: Color = Color::Rgb(0xEC, 0xE9, 0xE4);
pub const MUTED: Color = Color::Rgb(0x8C, 0x88, 0x82);

pub const WAITING: Color = Color::Rgb(0xE5, 0xC0, 0x7B);
/// Rot für „arbeitet“ (Fensterleiste und Sessionliste).
const BUSY: Color = Color::Rgb(0xE0, 0x5A, 0x4F);

/// Bilder des Claude-Spinners; läuft vor und zurück wie in Claude Code.
const SPINNER: [&str; 6] = ["·", "✢", "✳", "✶", "✻", "✽"];
const SPINNER_FRAME: Duration = Duration::from_millis(120);

/// Symbol und Farbe eines Zustands; `None` = gespeichert, läuft nicht. `elapsed` treibt den
/// Spinner an.
pub fn status_symbol(status: Option<Status>, elapsed: Duration) -> (&'static str, Color) {
    match status {
        Some(Status::Working) => (spinner(elapsed), BUSY),
        Some(Status::Waiting) => ("▲", WAITING),
        Some(Status::Idle) => ("●", MUTED),
        Some(Status::Exited) | None => ("○", MUTED),
    }
}

/// Statustext in der Fensterleiste: rot = arbeitet, gelb = Rückfrage, grau = bereit.
pub fn status_label(status: Status) -> (&'static str, Color) {
    match status {
        Status::Working => ("arbeitet", BUSY),
        Status::Waiting => ("wartet auf dich", WAITING),
        Status::Idle => ("bereit", MUTED),
        Status::Exited => ("beendet", MUTED),
    }
}

fn spinner(elapsed: Duration) -> &'static str {
    let cycle = 2 * SPINNER.len() - 2;
    let step = (elapsed.as_millis() / SPINNER_FRAME.as_millis()) as usize % cycle;
    let index = if step < SPINNER.len() {
        step
    } else {
        cycle - step
    };
    SPINNER[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_runs_forth_and_back() {
        let frames: Vec<&str> = (0..10).map(|i| spinner(SPINNER_FRAME * i)).collect();
        assert_eq!(frames, ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"]);
    }
}
