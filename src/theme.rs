use ratatui::style::Color;

use crate::session::Status;

/// Claude-Orange, die Akzentfarbe des Themes.
pub const ACCENT: Color = Color::Rgb(0xD9, 0x77, 0x57);
/// Rahmen und Titel des Bereichs mit Fokus.
pub const FOCUS: Color = Color::Rgb(0xF0, 0xA2, 0x7F);
/// Rahmen und Titel aller anderen Bereiche.
pub const UNFOCUS: Color = Color::Rgb(0x8C, 0x4A, 0x32);
pub const BG: Color = Color::Rgb(0x1A, 0x19, 0x18);
pub const TEXT: Color = Color::Rgb(0xEC, 0xE9, 0xE4);
pub const MUTED: Color = Color::Rgb(0x8C, 0x88, 0x82);

pub const WAITING: Color = Color::Rgb(0xE5, 0xC0, 0x7B);
const IDLE: Color = Color::Rgb(0x8F, 0xBF, 0x7F);

pub fn status(status: Status) -> (&'static str, Color) {
    match status {
        Status::Working => ("arbeitet", ACCENT),
        Status::Waiting => ("wartet auf dich", WAITING),
        Status::Idle => ("bereit", IDLE),
        Status::Exited => ("beendet", MUTED),
    }
}
