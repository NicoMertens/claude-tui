use std::time::Duration;

use ratatui::style::Color;

use crate::session::Status;

/// Claude-Orange, die Akzentfarbe des Themes.
pub const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
/// Rahmen und Titel des Bereichs mit Fokus — dasselbe Blau wie die Menübox.
pub const FOCUS: Color = Color::Rgb(0x00, 0x9F, 0xDA);
/// Rahmen und Titel aller anderen Bereiche (grau).
pub const UNFOCUS: Color = Color::Rgb(0x6B, 0x68, 0x64);
/// Hintergrund der TUI: der Standardhintergrund des Terminals, denselben nutzt Claude Code. So
/// gehen Rand und Sessionfenster ohne Farbsprung ineinander über.
pub const BG: Color = Color::Reset;
/// Schrift auf farbig hinterlegter Auswahl (Menü).
pub const ON_ACCENT: Color = Color::Rgb(0x1A, 0x19, 0x18);
/// Rand, Titel und Auswahl des Menüs (eigene Box unter der Sessionliste).
pub const MENU: Color = Color::Rgb(0x00, 0x9F, 0xDA);
/// Hintergrund der ausgewählten Zeile in der Sessionliste.
pub const SELECTED_BG: Color = Color::Rgb(0x33, 0x2D, 0x29);
pub const TEXT: Color = Color::Rgb(0xEC, 0xE9, 0xE4);
pub const MUTED: Color = Color::Rgb(0x8C, 0x88, 0x82);

pub const WAITING: Color = Color::Rgb(0xE5, 0xC0, 0x7B);
/// Rot für „arbeitet“ (Fensterleiste und Sessionliste).
const BUSY: Color = Color::Rgb(0xE0, 0x5A, 0x4F);

/// Bilder des Claude-Spinners; läuft vor und zurück wie in Claude Code. Kein `✳` (U+2733): das
/// stellt Windows als Emoji dar (grünes Quadrat, doppelt breit).
const SPINNER: [&str; 6] = ["·", "✢", "✣", "✶", "✻", "✽"];
pub const SPINNER_FRAME: Duration = Duration::from_millis(120);

/// Symbol und Farbe eines Zustands; `None` = gespeichert, läuft nicht. `elapsed` treibt den
/// Spinner an.
pub fn status_symbol(status: Option<Status>, elapsed: Duration) -> (&'static str, Color) {
    match status {
        Some(Status::Starting) => (spinner(elapsed), MUTED),
        Some(Status::Working) => (spinner(elapsed), BUSY),
        Some(Status::Waiting) => ("▲", WAITING),
        Some(Status::Idle) => ("●", MUTED),
        Some(Status::Exited) | None => ("○", MUTED),
    }
}

/// Statustext in der Fensterleiste: rot = arbeitet, gelb = Rückfrage, grau = bereit.
pub fn status_label(status: Status) -> (&'static str, Color) {
    match status {
        Status::Starting => ("starting", MUTED),
        Status::Working => ("working", BUSY),
        Status::Waiting => ("waiting for you", WAITING),
        Status::Idle => ("ready", MUTED),
        Status::Exited => ("exited", MUTED),
    }
}

/// Symbol und Farbe einer gespeicherten Session, die außerhalb von claude-tui läuft.
pub const ELSEWHERE: (&str, Color) = ("◇", Color::Rgb(0x7A, 0x9C, 0xB0));
/// Steht statt der Fensternummer bei einer Session, die außerhalb von claude-tui läuft.
pub const ELSEWHERE_LABEL: &str = " ext";

/// Sessionfarbe aus Claudes `/color` als Textfarbe; unbekannte Namen bleiben ungefärbt.
pub fn session_color(name: &str) -> Option<Color> {
    let rgb = match name {
        "red" => (0xE5, 0x53, 0x4B),
        "orange" => (0xE8, 0x8A, 0x4F),
        "yellow" => (0xE5, 0xC0, 0x7B),
        "green" => (0x8F, 0xBF, 0x7F),
        "cyan" => (0x56, 0xC2, 0xC9),
        "blue" => (0x4F, 0xA3, 0xE0),
        "purple" => (0xB4, 0x8E, 0xDB),
        "pink" => (0xE0, 0x7A, 0xB8),
        _ => return None,
    };
    Some(Color::Rgb(rgb.0, rgb.1, rgb.2))
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
        assert_eq!(frames, ["·", "✢", "✣", "✶", "✻", "✽", "✻", "✶", "✣", "✢"]);
    }

    #[test]
    fn spinner_has_no_emoji_characters() {
        // Zeichen mit Emoji-Darstellung unter Windows (✳ ✴ ❇ u. a.) verschieben die Zeile.
        const EMOJI: [char; 4] = ['✳', '✴', '❇', '❄'];
        assert!(SPINNER
            .iter()
            .all(|frame| !frame.chars().any(|c| EMOJI.contains(&c))));
    }
}
