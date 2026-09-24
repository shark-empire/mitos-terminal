//! What bytes an application receives for a key press, a mouse event, focus change…
//!
//! Kept independent of egui event plumbing (only `egui::Key` is used as an
//! enum) so the tables can be unit-tested exhaustively.
//!
//! Conventions follow xterm with `TERM=xterm-256color`:
//! * Backspace sends DEL (0x7f), Ctrl+Backspace sends BS (0x08);
//! * modified cursor/function keys use `CSI 1 ; <mod> X` / `CSI n ; <mod> ~`
//!   where `mod = 1 + shift + 2·alt + 4·ctrl`;
//! * printable characters are *not* produced here — they arrive as text events
//!   (so dead keys, compose and IME just work); Alt+text is ESC-prefixed by the caller.

use egui::Key;

use crate::term::{MouseEnc, MouseMode};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, alt: false, ctrl: false };

    pub fn from_egui(m: &egui::Modifiers) -> Mods {
        Mods { shift: m.shift, alt: m.alt, ctrl: m.ctrl }
    }

    pub fn any(&self) -> bool {
        self.shift || self.alt || self.ctrl
    }

    /// xterm modifier parameter: `1 + shift + 2·alt + 4·ctrl`.
    pub fn param(&self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
}

fn with_alt(mods: Mods, mut bytes: Vec<u8>) -> Vec<u8> {
    if mods.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn letter_index(key: Key) -> Option<u8> {
    Some(match key {
        Key::A => 1,
        Key::B => 2,
        Key::C => 3,
        Key::D => 4,
        Key::E => 5,
        Key::F => 6,
        Key::G => 7,
        Key::H => 8,
        Key::I => 9,
        Key::J => 10,
        Key::K => 11,
        Key::L => 12,
        Key::M => 13,
        Key::N => 14,
        Key::O => 15,
        Key::P => 16,
        Key::Q => 17,
        Key::R => 18,
        Key::S => 19,
        Key::T => 20,
        Key::U => 21,
        Key::V => 22,
        Key::W => 23,
        Key::X => 24,
        Key::Y => 25,
        Key::Z => 26,
        _ => return None,
    })
}

/// Bytes for a non-text key press, or `None` when the key produces no bytes
/// by itself (plain letters, digits, punctuation: those come as text events).
pub fn encode_key(key: Key, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    let m = mods.param();
    let has_mod = mods.any();

    // Cursor block: arrows, Home, End.
    let cursor_final: Option<u8> = match key {
        Key::ArrowUp => Some(b'A'),
        Key::ArrowDown => Some(b'B'),
        Key::ArrowRight => Some(b'C'),
        Key::ArrowLeft => Some(b'D'),
        Key::Home => Some(b'H'),
        Key::End => Some(b'F'),
        _ => None,
    };
    if let Some(f) = cursor_final {
        let s: Vec<u8> = if has_mod {
            format!("\x1b[1;{}{}", m, f as char).into_bytes()
        } else if app_cursor {
            vec![0x1b, b'O', f]
        } else {
            vec![0x1b, b'[', f]
        };
        return Some(s);
    }

    // `CSI n ~` keys.
    let tilde: Option<u8> = match key {
        Key::Insert => Some(2),
        Key::Delete => Some(3),
        Key::PageUp => Some(5),
        Key::PageDown => Some(6),
        Key::F5 => Some(15),
        Key::F6 => Some(17),
        Key::F7 => Some(18),
        Key::F8 => Some(19),
        Key::F9 => Some(20),
        Key::F10 => Some(21),
        Key::F11 => Some(23),
        Key::F12 => Some(24),
        Key::F13 => Some(25),
        Key::F14 => Some(26),
        Key::F15 => Some(28),
        Key::F16 => Some(29),
        Key::F17 => Some(31),
        Key::F18 => Some(32),
        Key::F19 => Some(33),
        Key::F20 => Some(34),
        _ => None,
    };
    if let Some(n) = tilde {
        let s: String = if has_mod { format!("\x1b[{};{}~", n, m) } else { format!("\x1b[{}~", n) };
        return Some(s.into_bytes());
    }

    // F1–F4 use SS3.
    let ss3: Option<u8> = match key {
        Key::F1 => Some(b'P'),
        Key::F2 => Some(b'Q'),
        Key::F3 => Some(b'R'),
        Key::F4 => Some(b'S'),
        _ => None,
    };
    if let Some(f) = ss3 {
        let s: String = if has_mod { format!("\x1b[1;{}{}", m, f as char) } else { format!("\x1bO{}", f as char) };
        return Some(s.into_bytes());
    }

    match key {
        Key::Enter => return Some(with_alt(mods, vec![b'\r'])),
        Key::Tab => {
            return Some(if mods.shift { b"\x1b[Z".to_vec() } else { with_alt(mods, vec![b'\t']) });
        }
        Key::Backspace => {
            let b = if mods.ctrl { 0x08 } else { 0x7f };
            return Some(with_alt(mods, vec![b]));
        }
        Key::Escape => return Some(with_alt(mods, vec![0x1b])),
        Key::Space => {
            if mods.ctrl {
                return Some(with_alt(mods, vec![0]));
            }
            if mods.alt {
                return Some(b"\x1b ".to_vec());
            }
            return None;
        }
        _ => {}
    }

    if mods.ctrl {
        let byte: Option<u8> = match key {
            Key::OpenBracket => Some(0x1b),
            Key::Backslash => Some(0x1c),
            Key::CloseBracket => Some(0x1d),
            Key::Slash | Key::Minus => Some(0x1f),
            Key::Num2 => Some(0x00),
            Key::Num3 => Some(0x1b),
            Key::Num4 => Some(0x1c),
            Key::Num5 => Some(0x1d),
            Key::Num6 => Some(0x1e),
            Key::Num7 => Some(0x1f),
            Key::Num8 => Some(0x7f),
            other => letter_index(other),
        };
        if let Some(b) = byte {
            return Some(with_alt(mods, vec![b]));
        }
    }
    None
}

/// Text typed by the user (or committed by an IME). Alt+text is ESC-prefixed.
pub fn encode_text(text: &str, alt: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + 1);
    if alt {
        v.push(0x1b);
    }
    v.extend_from_slice(text.as_bytes());
    v
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseKind {
    Press,
    Release,
    Move,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Clone, Copy, Debug)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// 0 = left, 1 = middle, 2 = right (ignored for wheel and plain moves).
    pub button: u8,
    /// Zero-based cell coordinates.
    pub col: usize,
    pub row: usize,
    pub mods: Mods,
    /// Button held during a `Move` (drag), if any.
    pub held: Option<u8>,
}

/// Encode a mouse event for the negotiated tracking mode and encoding.
/// `None` when the mode does not report this event (or it cannot be represented).
pub fn encode_mouse(mode: MouseMode, enc: MouseEnc, ev: &MouseEvent) -> Option<Vec<u8>> {
    let allowed = match mode {
        MouseMode::Off => false,
        MouseMode::X10 => ev.kind == MouseKind::Press,
        MouseMode::Normal => ev.kind != MouseKind::Move,
        MouseMode::Button => ev.kind != MouseKind::Move || ev.held.is_some(),
        MouseMode::Any => true,
    };
    if !allowed {
        return None;
    }

    let mut cb: u32 = match ev.kind {
        MouseKind::Press => ev.button.min(2) as u32,
        MouseKind::Release => {
            if enc == MouseEnc::Sgr {
                ev.button.min(2) as u32
            } else {
                3
            }
        }
        MouseKind::Move => 32 + ev.held.map(|b| b.min(2) as u32).unwrap_or(3),
        MouseKind::WheelUp => 64,
        MouseKind::WheelDown => 65,
        MouseKind::WheelLeft => 66,
        MouseKind::WheelRight => 67,
    };
    if mode != MouseMode::X10 {
        if ev.mods.shift {
            cb += 4;
        }
        if ev.mods.alt {
            cb += 8;
        }
        if ev.mods.ctrl {
            cb += 16;
        }
    }
    let x = ev.col + 1;
    let y = ev.row + 1;

    match enc {
        MouseEnc::Sgr => {
            let fin = if ev.kind == MouseKind::Release { 'm' } else { 'M' };
            Some(format!("\x1b[<{};{};{}{}", cb, x, y, fin).into_bytes())
        }
        MouseEnc::Urxvt => Some(format!("\x1b[{};{};{}M", cb + 32, x, y).into_bytes()),
        MouseEnc::Utf8 => {
            let mut v = b"\x1b[M".to_vec();
            for n in [cb + 32, x as u32 + 32, y as u32 + 32] {
                let ch = char::from_u32(n)?;
                let mut buf = [0u8; 4];
                v.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            Some(v)
        }
        MouseEnc::Default => {
            if x + 32 > 255 || y + 32 > 255 {
                return None;
            }
            Some(vec![0x1b, b'[', b'M', (cb + 32) as u8, (x + 32) as u8, (y + 32) as u8])
        }
    }
}

/// Alternate-scroll (DECSET 1007): the wheel on the alternate screen becomes arrow keys.
pub fn wheel_as_arrows(up: bool, lines: usize, app_cursor: bool) -> Vec<u8> {
    let f = if up { b'A' } else { b'B' };
    let one: Vec<u8> = if app_cursor { vec![0x1b, b'O', f] } else { vec![0x1b, b'[', f] };
    let n = lines.clamp(1, 64);
    let mut out = Vec::with_capacity(one.len() * n);
    for _ in 0..n {
        out.extend_from_slice(&one);
    }
    out
}

/// DECSET 1004 focus reporting.
pub fn focus_report(focused: bool) -> &'static [u8] {
    if focused {
        b"\x1b[I"
    } else {
        b"\x1b[O"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: Mods = Mods::NONE;
    const CTRL: Mods = Mods { shift: false, alt: false, ctrl: true };
    const ALT: Mods = Mods { shift: false, alt: true, ctrl: false };
    const SHIFT: Mods = Mods { shift: true, alt: false, ctrl: false };

    fn k(key: Key, mods: Mods, app: bool) -> Option<Vec<u8>> {
        encode_key(key, mods, app)
    }

    #[test]
    fn modifier_parameter() {
        assert_eq!(N.param(), 1);
        assert_eq!(SHIFT.param(), 2);
        assert_eq!(ALT.param(), 3);
        assert_eq!(CTRL.param(), 5);
        assert_eq!(Mods { shift: true, alt: true, ctrl: true }.param(), 8);
    }

    #[test]
    fn arrows_respect_application_cursor_mode() {
        assert_eq!(k(Key::ArrowUp, N, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(k(Key::ArrowUp, N, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(k(Key::ArrowLeft, CTRL, true), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(k(Key::ArrowDown, Mods { shift: true, alt: true, ctrl: false }, false), Some(b"\x1b[1;4B".to_vec()));
        assert_eq!(k(Key::Home, N, false), Some(b"\x1b[H".to_vec()));
        assert_eq!(k(Key::End, N, true), Some(b"\x1bOF".to_vec()));
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(k(Key::Delete, N, false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(k(Key::PageUp, CTRL, false), Some(b"\x1b[5;5~".to_vec()));
        assert_eq!(k(Key::Insert, N, false), Some(b"\x1b[2~".to_vec()));
        assert_eq!(k(Key::F1, N, false), Some(b"\x1bOP".to_vec()));
        assert_eq!(k(Key::F4, SHIFT, false), Some(b"\x1b[1;2S".to_vec()));
        assert_eq!(k(Key::F5, N, false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(k(Key::F12, N, false), Some(b"\x1b[24~".to_vec()));
        assert_eq!(k(Key::F5, CTRL, false), Some(b"\x1b[15;5~".to_vec()));
        assert_eq!(k(Key::F20, N, false), Some(b"\x1b[34~".to_vec()));
    }

    #[test]
    fn whitespace_and_control_keys() {
        assert_eq!(k(Key::Enter, N, false), Some(b"\r".to_vec()));
        assert_eq!(k(Key::Enter, ALT, false), Some(b"\x1b\r".to_vec()));
        assert_eq!(k(Key::Tab, N, false), Some(b"\t".to_vec()));
        assert_eq!(k(Key::Tab, SHIFT, false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(k(Key::Backspace, N, false), Some(vec![0x7f]));
        assert_eq!(k(Key::Backspace, CTRL, false), Some(vec![0x08]));
        assert_eq!(k(Key::Backspace, ALT, false), Some(vec![0x1b, 0x7f]));
        assert_eq!(k(Key::Escape, N, false), Some(vec![0x1b]));
        assert_eq!(k(Key::Space, N, false), None);
        assert_eq!(k(Key::Space, CTRL, false), Some(vec![0]));
        assert_eq!(k(Key::Space, ALT, false), Some(b"\x1b ".to_vec()));
    }

    #[test]
    fn control_letters_send_c0_codes() {
        assert_eq!(k(Key::C, CTRL, false), Some(vec![3]));
        assert_eq!(k(Key::D, CTRL, false), Some(vec![4]));
        assert_eq!(k(Key::Z, CTRL, false), Some(vec![26]));
        assert_eq!(k(Key::A, Mods { shift: true, alt: false, ctrl: true }, false), Some(vec![1]));
        assert_eq!(k(Key::C, Mods { shift: false, alt: true, ctrl: true }, false), Some(vec![0x1b, 3]));
        assert_eq!(k(Key::OpenBracket, CTRL, false), Some(vec![0x1b]));
        assert_eq!(k(Key::Backslash, CTRL, false), Some(vec![0x1c]));
        assert_eq!(k(Key::Num8, CTRL, false), Some(vec![0x7f]));
    }

    #[test]
    fn printable_keys_are_left_to_text_events() {
        assert_eq!(k(Key::A, N, false), None);
        assert_eq!(k(Key::A, ALT, false), None);
        assert_eq!(k(Key::A, SHIFT, false), None);
        assert_eq!(k(Key::Num5, N, false), None);
        assert_eq!(encode_text("é", false), "é".as_bytes().to_vec());
        assert_eq!(encode_text("x", true), b"\x1bx".to_vec());
    }

    fn ev(kind: MouseKind, button: u8, col: usize, row: usize) -> MouseEvent {
        MouseEvent { kind, button, col, row, mods: N, held: None }
    }

    #[test]
    fn sgr_mouse_encoding() {
        let m = |e: MouseEvent| encode_mouse(MouseMode::Normal, MouseEnc::Sgr, &e);
        assert_eq!(m(ev(MouseKind::Press, 0, 0, 0)), Some(b"\x1b[<0;1;1M".to_vec()));
        assert_eq!(m(ev(MouseKind::Release, 0, 0, 0)), Some(b"\x1b[<0;1;1m".to_vec()));
        assert_eq!(m(ev(MouseKind::Press, 2, 9, 4)), Some(b"\x1b[<2;10;5M".to_vec()));
        assert_eq!(m(ev(MouseKind::WheelUp, 0, 4, 6)), Some(b"\x1b[<64;5;7M".to_vec()));
        assert_eq!(m(ev(MouseKind::WheelDown, 0, 0, 0)), Some(b"\x1b[<65;1;1M".to_vec()));
        let mut e = ev(MouseKind::Press, 0, 0, 0);
        e.mods = CTRL;
        assert_eq!(m(e), Some(b"\x1b[<16;1;1M".to_vec()));
    }

    #[test]
    fn legacy_mouse_encodings() {
        let e = ev(MouseKind::Press, 0, 2, 3);
        assert_eq!(encode_mouse(MouseMode::Normal, MouseEnc::Default, &e), Some(vec![0x1b, b'[', b'M', 32, 35, 36]));
        let r = ev(MouseKind::Release, 0, 2, 3);
        assert_eq!(encode_mouse(MouseMode::Normal, MouseEnc::Default, &r), Some(vec![0x1b, b'[', b'M', 35, 35, 36]));
        assert_eq!(encode_mouse(MouseMode::Normal, MouseEnc::Urxvt, &ev(MouseKind::Press, 0, 0, 0)), Some(b"\x1b[32;1;1M".to_vec()));
        // coordinates beyond 223 cannot be sent in the legacy encoding
        assert_eq!(encode_mouse(MouseMode::Normal, MouseEnc::Default, &ev(MouseKind::Press, 0, 300, 0)), None);
        // …but UTF-8 mode can
        assert!(encode_mouse(MouseMode::Normal, MouseEnc::Utf8, &ev(MouseKind::Press, 0, 300, 0)).is_some());
    }

    #[test]
    fn tracking_modes_filter_events() {
        let mv = ev(MouseKind::Move, 0, 1, 1);
        assert_eq!(encode_mouse(MouseMode::Normal, MouseEnc::Sgr, &mv), None);
        assert_eq!(encode_mouse(MouseMode::Button, MouseEnc::Sgr, &mv), None);
        let mut drag = mv;
        drag.held = Some(0);
        assert_eq!(encode_mouse(MouseMode::Button, MouseEnc::Sgr, &drag), Some(b"\x1b[<32;2;2M".to_vec()));
        assert_eq!(encode_mouse(MouseMode::Any, MouseEnc::Sgr, &mv), Some(b"\x1b[<35;2;2M".to_vec()));
        assert_eq!(encode_mouse(MouseMode::Off, MouseEnc::Sgr, &ev(MouseKind::Press, 0, 0, 0)), None);
        assert!(encode_mouse(MouseMode::X10, MouseEnc::Default, &ev(MouseKind::Press, 0, 0, 0)).is_some());
        assert_eq!(encode_mouse(MouseMode::X10, MouseEnc::Default, &ev(MouseKind::Release, 0, 0, 0)), None);
    }

    #[test]
    fn wheel_and_focus_helpers() {
        assert_eq!(wheel_as_arrows(true, 3, false), b"\x1b[A\x1b[A\x1b[A".to_vec());
        assert_eq!(wheel_as_arrows(false, 1, true), b"\x1bOB".to_vec());
        assert_eq!(focus_report(true), b"\x1b[I");
        assert_eq!(focus_report(false), b"\x1b[O");
    }
}
