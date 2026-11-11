use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::layout::Direction;

pub const PREFIX_HINT: &str = "Ctrl+A";
/// Was die Session bekommt, wenn der Präfix zweimal gedrückt wird.
pub const PREFIX_BYTES: &[u8] = &[0x01];

pub fn is_prefix(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('a') && key.modifiers == KeyModifiers::CONTROL
}

/// Übersetzt einen Tastendruck in die Bytefolge, die ein xterm-kompatibles Terminal senden würde.
pub fn to_bytes(key: &KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);

    let mut bytes = match key.code {
        // AltGr meldet Windows als Ctrl+Alt — auf deutscher Tastatur kommen so @, €, {, [ usw.
        KeyCode::Char(c) if ctrl && alt && !c.is_ascii_alphabetic() => {
            return Some(c.to_string().into_bytes())
        }
        KeyCode::Char(c) if ctrl => vec![control_byte(c)?],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter if shift => b"\x1b\r".to_vec(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return Some(cursor(b'A', mods, application_cursor)),
        KeyCode::Down => return Some(cursor(b'B', mods, application_cursor)),
        KeyCode::Right => return Some(cursor(b'C', mods, application_cursor)),
        KeyCode::Left => return Some(cursor(b'D', mods, application_cursor)),
        KeyCode::Home => return Some(cursor(b'H', mods, application_cursor)),
        KeyCode::End => return Some(cursor(b'F', mods, application_cursor)),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(n) => function_key(n)?,
        _ => return None,
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

fn control_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        ' ' | '@' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        _ => None,
    }
}

fn cursor(code: u8, mods: KeyModifiers, application_cursor: bool) -> Vec<u8> {
    let modifier = 1
        + u8::from(mods.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(mods.contains(KeyModifiers::ALT))
        + 4 * u8::from(mods.contains(KeyModifiers::CONTROL));
    if modifier > 1 {
        format!("\x1b[1;{modifier}{}", code as char).into_bytes()
    } else if application_cursor {
        vec![0x1b, b'O', code]
    } else {
        vec![0x1b, b'[', code]
    }
}

fn function_key(n: u8) -> Option<Vec<u8>> {
    let seq: &[u8] = match n {
        1 => b"\x1bOP",
        2 => b"\x1bOQ",
        3 => b"\x1bOR",
        4 => b"\x1bOS",
        5 => b"\x1b[15~",
        6 => b"\x1b[17~",
        7 => b"\x1b[18~",
        8 => b"\x1b[19~",
        9 => b"\x1b[20~",
        10 => b"\x1b[21~",
        11 => b"\x1b[23~",
        12 => b"\x1b[24~",
        _ => return None,
    };
    Some(seq.to_vec())
}

/// Alt+Pfeil wechselt den Fokus und erreicht die Session nie. Nur reines Alt — Ctrl+Alt ist unter
/// Windows AltGr.
pub fn focus_direction(key: &KeyEvent) -> Option<Direction> {
    if key.modifiers != KeyModifiers::ALT {
        return None;
    }
    match key.code {
        KeyCode::Left => Some(Direction::Left),
        KeyCode::Right => Some(Direction::Right),
        KeyCode::Up => Some(Direction::Up),
        KeyCode::Down => Some(Direction::Down),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_arrow_is_focus_change_but_ctrl_arrow_is_not() {
        let alt_left = KeyEvent::new(KeyCode::Left, KeyModifiers::ALT);
        let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(focus_direction(&alt_left), Some(Direction::Left));
        assert_eq!(focus_direction(&ctrl_left), None);
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn ctrl_c_sends_etx() {
        assert_eq!(
            to_bytes(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            Some(vec![3])
        );
    }

    #[test]
    fn altgr_char_is_sent_verbatim() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(
            to_bytes(&key(KeyCode::Char('@'), altgr), false),
            Some(b"@".to_vec())
        );
    }

    #[test]
    fn arrows_respect_application_cursor_mode() {
        assert_eq!(
            to_bytes(&key(KeyCode::Up, KeyModifiers::NONE), true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            to_bytes(&key(KeyCode::Up, KeyModifiers::NONE), false),
            Some(b"\x1b[A".to_vec())
        );
    }

    #[test]
    fn ctrl_arrow_uses_modifier_parameter() {
        assert_eq!(
            to_bytes(&key(KeyCode::Right, KeyModifiers::CONTROL), false),
            Some(b"\x1b[1;5C".to_vec())
        );
    }
}
