//! Terminal modes, cursor styles and character sets.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseMode {
    Off,
    /// DECSET 9 — press only.
    X10,
    /// DECSET 1000 — press + release.
    Normal,
    /// DECSET 1002 — plus drag motion.
    Button,
    /// DECSET 1003 — all motion.
    Any,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseEnc {
    Default,
    Utf8,
    Sgr,
    Urxvt,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blink: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Modes {
    /// DECAWM (7).
    pub autowrap: bool,
    /// DECOM (6).
    pub origin: bool,
    /// IRM (4).
    pub insert: bool,
    /// LNM (20).
    pub lnm: bool,
    /// DECTCEM (25).
    pub cursor_visible: bool,
    /// DECCKM (1).
    pub app_cursor: bool,
    /// DECKPAM / DECKPNM.
    pub app_keypad: bool,
    /// 2004.
    pub bracketed_paste: bool,
    pub mouse: MouseMode,
    pub mouse_enc: MouseEnc,
    /// 1004.
    pub focus_events: bool,
    /// 1007 — wheel → arrows on the alternate screen.
    pub alt_scroll: bool,
    /// DECSCNM (5).
    pub reverse_video: bool,
    /// 2026 — synchronized output.
    pub sync_output: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            autowrap: true,
            origin: false,
            insert: false,
            lnm: false,
            cursor_visible: true,
            app_cursor: false,
            app_keypad: false,
            bracketed_paste: false,
            mouse: MouseMode::Off,
            mouse_enc: MouseEnc::Default,
            focus_events: false,
            alt_scroll: true,
            reverse_video: false,
            sync_output: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charset {
    Ascii,
    DecGraphics,
    Uk,
}

#[derive(Clone, Copy, Debug)]
pub struct CharsetState {
    pub g: [Charset; 4],
    /// Which of G0..G3 is invoked into GL.
    pub gl: usize,
}

impl Default for CharsetState {
    fn default() -> Self {
        CharsetState { g: [Charset::Ascii; 4], gl: 0 }
    }
}

impl CharsetState {
    #[inline]
    pub fn is_plain(&self) -> bool {
        self.g[self.gl] == Charset::Ascii
    }

    pub fn map(&self, c: char) -> char {
        match self.g[self.gl] {
            Charset::Ascii => c,
            Charset::Uk => {
                if c == '#' {
                    '£'
                } else {
                    c
                }
            }
            Charset::DecGraphics => dec_graphics(c),
        }
    }
}

fn dec_graphics(c: char) -> char {
    match c {
        '_' => ' ',
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        other => other,
    }
}
