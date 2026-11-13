//! Mausereignisse für das Programm in der Session kodieren (xterm-Mausprotokoll).
//!
//! Schaltet das Programm selbst die Maus ein (Claude Code im Fullscreen-Modus: DECSET 1003 + 1006),
//! bekommt es Klicks, Ziehen und Rad durchgereicht und markiert bzw. kopiert dann selbst — über
//! den ganzen Verlauf statt nur den sichtbaren Ausschnitt.

use vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press(Button),
    Release(Button),
    Drag(Button),
    Wheel { up: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Ein Mausereignis an Zeile/Spalte des Session-Bildschirms (0-basiert).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub action: Action,
    pub col: u16,
    pub row: u16,
    pub modifiers: Modifiers,
}

/// Bytes für das Programm, oder `None`, wenn es dieses Ereignis im aktuellen Modus nicht
/// angefordert hat.
pub fn encode(
    report: Report,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    let wanted = match (mode, report.action) {
        (MouseProtocolMode::None, _) => false,
        (_, Action::Press(_) | Action::Wheel { .. }) => true,
        (MouseProtocolMode::Press, _) => false,
        (_, Action::Release(_)) => true,
        (MouseProtocolMode::PressRelease, _) => false,
        (_, Action::Drag(_)) => true,
    };
    if !wanted {
        return None;
    }
    let button_code = |b: Button| match b {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    };
    let sgr = encoding == MouseProtocolEncoding::Sgr;
    let mut code: u32 = match report.action {
        Action::Press(b) => button_code(b),
        // Die klassischen Kodierungen kennen beim Loslassen keine Taste, nur „3“.
        Action::Release(b) if sgr => button_code(b),
        Action::Release(_) => 3,
        Action::Drag(b) => button_code(b) + 32,
        Action::Wheel { up } => 64 + u32::from(!up),
    };
    let m = report.modifiers;
    code += 4 * u32::from(m.shift) + 8 * u32::from(m.alt) + 16 * u32::from(m.ctrl);
    let x = u32::from(report.col) + 1;
    let y = u32::from(report.row) + 1;
    let bytes = match encoding {
        MouseProtocolEncoding::Sgr => {
            let end = if matches!(report.action, Action::Release(_)) {
                'm'
            } else {
                'M'
            };
            format!("\x1b[<{code};{x};{y}{end}").into_bytes()
        }
        MouseProtocolEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for v in [code, x, y] {
                let c = char::from_u32(v + 32)?;
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        // Klassisch: Werte + 32 als einzelne Bytes, also nur bis Spalte/Zeile 223.
        MouseProtocolEncoding::Default => {
            let byte = |v: u32| u8::try_from(v + 32).unwrap_or(u8::MAX);
            vec![0x1b, b'[', b'M', byte(code), byte(x), byte(y)]
        }
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(action: Action) -> Report {
        Report {
            action,
            col: 4,
            row: 2,
            modifiers: Modifiers::default(),
        }
    }

    fn sgr(action: Action, mode: MouseProtocolMode) -> Option<String> {
        encode(report(action), mode, MouseProtocolEncoding::Sgr)
            .map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn sgr_press_drag_release_are_one_based() {
        let any = MouseProtocolMode::AnyMotion;
        assert_eq!(
            sgr(Action::Press(Button::Left), any).as_deref(),
            Some("\x1b[<0;5;3M")
        );
        assert_eq!(
            sgr(Action::Drag(Button::Left), any).as_deref(),
            Some("\x1b[<32;5;3M")
        );
        assert_eq!(
            sgr(Action::Release(Button::Right), any).as_deref(),
            Some("\x1b[<2;5;3m")
        );
        assert_eq!(
            sgr(Action::Wheel { up: false }, any).as_deref(),
            Some("\x1b[<65;5;3M")
        );
    }

    #[test]
    fn events_are_filtered_by_requested_mode() {
        let drag = Action::Drag(Button::Left);
        let release = Action::Release(Button::Left);
        assert_eq!(sgr(drag, MouseProtocolMode::None), None);
        assert_eq!(sgr(release, MouseProtocolMode::Press), None);
        assert_eq!(sgr(drag, MouseProtocolMode::PressRelease), None);
        assert!(sgr(release, MouseProtocolMode::PressRelease).is_some());
        assert!(sgr(drag, MouseProtocolMode::ButtonMotion).is_some());
        assert!(sgr(Action::Wheel { up: true }, MouseProtocolMode::Press).is_some());
    }

    #[test]
    fn modifiers_add_to_button_code() {
        let mut r = report(Action::Press(Button::Left));
        r.modifiers = Modifiers {
            shift: true,
            alt: false,
            ctrl: true,
        };
        let bytes = encode(r, MouseProtocolMode::AnyMotion, MouseProtocolEncoding::Sgr);
        assert_eq!(bytes.as_deref(), Some(&b"\x1b[<20;5;3M"[..]));
    }

    #[test]
    fn classic_encoding_has_no_button_on_release_and_caps_coordinates() {
        let mut r = report(Action::Release(Button::Left));
        r.col = 300;
        let bytes = encode(
            r,
            MouseProtocolMode::PressRelease,
            MouseProtocolEncoding::Default,
        );
        assert_eq!(bytes, Some(vec![0x1b, b'[', b'M', 35, 255, 35]));
    }

    #[test]
    fn utf8_encoding_extends_coordinates() {
        let mut r = report(Action::Press(Button::Left));
        r.col = 300;
        let bytes = encode(r, MouseProtocolMode::Press, MouseProtocolEncoding::Utf8).unwrap();
        let mut expected = b"\x1b[M ".to_vec();
        expected.extend_from_slice("\u{14d}#".as_bytes());
        assert_eq!(bytes, expected);
    }
}
