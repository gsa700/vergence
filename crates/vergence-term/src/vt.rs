// The escape-sequence parser (our own, after the DEC ANSI parser state machine
// described at vt100.net/emu/dec_ansi_parser): bytes in, actions out. UTF-8 is
// decoded in the ground state. Device control strings and SOS/PM/APC strings
// are recognized and ignored.

/// What the parser found. The terminal (grid.rs) acts on these.
pub trait Perform {
    /// A character to show.
    fn print(&mut self, c: char);
    /// A C0 control (BEL, BS, HT, LF, CR...).
    fn execute(&mut self, byte: u8);
    /// A control sequence: ESC [ (private marker) params (intermediates) final.
    fn csi(&mut self, private: Option<u8>, params: &[u16], intermediates: &[u8], last: char);
    /// An escape sequence: ESC (intermediates) final.
    fn esc(&mut self, intermediates: &[u8], last: u8);
    /// An operating system command: ESC ] data (BEL or ST).
    fn osc(&mut self, data: &[u8]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    Osc,
    /// DCS, SOS, PM, APC: skipped until ST.
    String,
}

const MAX_PARAMS: usize = 32;
/// OSC 52 carries copied text as base64, so this allows a large copy (1 MiB);
/// the buffer only grows as far as a sequence needs.
const MAX_OSC: usize = 1 << 20;

pub struct Parser {
    state: State,
    params: Vec<u16>,
    param: Option<u32>,
    intermediates: Vec<u8>,
    private: Option<u8>,
    osc: Vec<u8>,
    /// ESC seen inside a string: the next byte '\\' ends it.
    string_esc: bool,
    utf8: Vec<u8>,
    utf8_need: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Parser {
        Parser {
            state: State::Ground,
            params: Vec::new(),
            param: None,
            intermediates: Vec::new(),
            private: None,
            osc: Vec::new(),
            string_esc: false,
            utf8: Vec::new(),
            utf8_need: 0,
        }
    }

    pub fn advance(&mut self, p: &mut impl Perform, bytes: &[u8]) {
        for &b in bytes {
            self.byte(p, b);
        }
    }

    fn clear(&mut self) {
        self.params.clear();
        self.param = None;
        self.intermediates.clear();
        self.private = None;
    }

    fn push_param(&mut self) {
        if self.params.len() < MAX_PARAMS {
            self.params.push(self.param.unwrap_or(0).min(u16::MAX as u32) as u16);
        }
        self.param = None;
    }

    fn byte(&mut self, p: &mut impl Perform, b: u8) {
        // Anywhere: CAN and SUB cancel, ESC starts again (except in strings,
        // where ESC may begin the terminator).
        match (self.state, b) {
            (State::Osc | State::String, 0x1b) => {
                self.string_esc = true;
                return;
            }
            (State::Osc | State::String, b'\\') if self.string_esc => {
                if self.state == State::Osc {
                    p.osc(&self.osc);
                }
                self.osc.clear();
                self.string_esc = false;
                self.state = State::Ground;
                return;
            }
            (State::Osc | State::String, _) if self.string_esc => {
                // ESC then something else: the string ends, and that ESC starts a new sequence.
                self.string_esc = false;
                if self.state == State::Osc {
                    p.osc(&self.osc);
                }
                self.osc.clear();
                self.state = State::Escape;
                self.clear();
                return self.byte(p, b);
            }
            (_, 0x18 | 0x1a) => {
                self.state = State::Ground;
                self.utf8.clear();
                return;
            }
            (State::Ground, 0x1b)
            | (
                State::Escape
                | State::EscapeIntermediate
                | State::CsiEntry
                | State::CsiParam
                | State::CsiIntermediate
                | State::CsiIgnore,
                0x1b,
            ) => {
                self.utf8.clear();
                self.state = State::Escape;
                self.clear();
                return;
            }
            _ => {}
        }
        match self.state {
            State::Ground => self.ground(p, b),
            State::Escape => match b {
                0x00..=0x1f => p.execute(b),
                0x20..=0x2f => {
                    self.intermediates.push(b);
                    self.state = State::EscapeIntermediate;
                }
                b'[' => {
                    self.clear();
                    self.state = State::CsiEntry;
                }
                b']' => {
                    self.osc.clear();
                    self.state = State::Osc;
                }
                b'P' | b'X' | b'^' | b'_' => self.state = State::String,
                0x7f => {}
                _ => {
                    p.esc(&self.intermediates, b);
                    self.state = State::Ground;
                }
            },
            State::EscapeIntermediate => match b {
                0x00..=0x1f => p.execute(b),
                0x20..=0x2f => self.intermediates.push(b),
                0x7f => {}
                _ => {
                    p.esc(&self.intermediates, b);
                    self.state = State::Ground;
                }
            },
            State::CsiEntry | State::CsiParam => match b {
                0x00..=0x1f => p.execute(b),
                b'0'..=b'9' => {
                    self.param = Some(self.param.unwrap_or(0).saturating_mul(10).saturating_add((b - b'0') as u32));
                    self.state = State::CsiParam;
                }
                // Colon sub-parameters (38:2:r:g:b) are taken as separators too.
                b';' | b':' => {
                    self.push_param();
                    self.state = State::CsiParam;
                }
                b'<'..=b'?' if self.state == State::CsiEntry => {
                    self.private = Some(b);
                    self.state = State::CsiParam;
                }
                b'<'..=b'?' => self.state = State::CsiIgnore,
                0x20..=0x2f => {
                    if self.param.is_some() || !self.params.is_empty() {
                        self.push_param();
                    }
                    self.intermediates.push(b);
                    self.state = State::CsiIntermediate;
                }
                0x40..=0x7e => {
                    if self.param.is_some() || !self.params.is_empty() {
                        self.push_param();
                    }
                    p.csi(self.private, &self.params, &self.intermediates, b as char);
                    self.state = State::Ground;
                }
                _ => {}
            },
            State::CsiIntermediate => match b {
                0x00..=0x1f => p.execute(b),
                0x20..=0x2f => self.intermediates.push(b),
                0x30..=0x3f => self.state = State::CsiIgnore,
                0x40..=0x7e => {
                    p.csi(self.private, &self.params, &self.intermediates, b as char);
                    self.state = State::Ground;
                }
                _ => {}
            },
            State::CsiIgnore => match b {
                0x00..=0x1f => p.execute(b),
                0x40..=0x7e => self.state = State::Ground,
                _ => {}
            },
            State::Osc => match b {
                0x07 => {
                    p.osc(&self.osc);
                    self.osc.clear();
                    self.state = State::Ground;
                }
                0x00..=0x1f => {}
                _ => {
                    if self.osc.len() < MAX_OSC {
                        self.osc.push(b);
                    }
                }
            },
            State::String => {}
        }
    }

    fn ground(&mut self, p: &mut impl Perform, b: u8) {
        if self.utf8_need > 0 {
            if b & 0xc0 == 0x80 {
                self.utf8.push(b);
                if self.utf8.len() == self.utf8_need {
                    let c = std::str::from_utf8(&self.utf8).ok().and_then(|s| s.chars().next()).unwrap_or('\u{fffd}');
                    self.utf8.clear();
                    self.utf8_need = 0;
                    p.print(c);
                }
                return;
            }
            // A broken sequence: show a replacement, then this byte as usual.
            self.utf8.clear();
            self.utf8_need = 0;
            p.print('\u{fffd}');
        }
        match b {
            0x00..=0x1f => p.execute(b),
            0x20..=0x7e => p.print(b as char),
            0x7f => {}
            0xc2..=0xdf => self.start_utf8(b, 2),
            0xe0..=0xef => self.start_utf8(b, 3),
            0xf0..=0xf4 => self.start_utf8(b, 4),
            _ => p.print('\u{fffd}'),
        }
    }

    fn start_utf8(&mut self, b: u8, n: usize) {
        self.utf8.clear();
        self.utf8.push(b);
        self.utf8_need = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Log(Vec<String>);

    impl Perform for Log {
        fn print(&mut self, c: char) {
            self.0.push(format!("p{c}"));
        }
        fn execute(&mut self, b: u8) {
            self.0.push(format!("x{b}"));
        }
        fn csi(&mut self, private: Option<u8>, params: &[u16], inter: &[u8], last: char) {
            self.0.push(format!(
                "csi{}{:?}{}{last}",
                private.map(|p| p as char).unwrap_or(' '),
                params,
                String::from_utf8_lossy(inter)
            ));
        }
        fn esc(&mut self, inter: &[u8], last: u8) {
            self.0.push(format!("esc{}{}", String::from_utf8_lossy(inter), last as char));
        }
        fn osc(&mut self, data: &[u8]) {
            self.0.push(format!("osc{}", String::from_utf8_lossy(data)));
        }
    }

    fn run(bytes: &[u8]) -> Vec<String> {
        let mut l = Log::default();
        Parser::new().advance(&mut l, bytes);
        l.0
    }

    #[test]
    fn text_controls_and_utf8() {
        assert_eq!(run(b"a\r\n"), ["pa", "x13", "x10"]);
        assert_eq!(run("é漢🙂".as_bytes()), ["pé", "p漢", "p🙂"]);
        // A broken sequence: replacement, then the byte that broke it.
        assert_eq!(run(&[0xe6, b'a']), ["p\u{fffd}", "pa"]);
    }

    #[test]
    fn control_sequences() {
        assert_eq!(run(b"\x1b[1;31m"), ["csi [1, 31]m"]);
        assert_eq!(run(b"\x1b[H"), ["csi []H"]);
        assert_eq!(run(b"\x1b[;5H"), ["csi [0, 5]H"]);
        assert_eq!(run(b"\x1b[?1049h"), ["csi?[1049]h"]);
        assert_eq!(run(b"\x1b[38:2:1:2:3m"), ["csi [38, 2, 1, 2, 3]m"]);
        assert_eq!(run(b"\x1b[2 q"), ["csi [2] q"]);
        // ESC inside a sequence starts a new one.
        assert_eq!(run(b"\x1b[12\x1b[3m"), ["csi [3]m"]);
    }

    #[test]
    fn escapes_and_strings() {
        assert_eq!(run(b"\x1b7\x1b(B"), ["esc7", "esc(B"]);
        assert_eq!(run(b"\x1b]0;title\x07x"), ["osc0;title", "px"]);
        assert_eq!(run(b"\x1b]2;t\x1b\\x"), ["osc2;t", "px"]);
        // DCS is skipped to ST.
        assert_eq!(run(b"\x1bPq#0;2;0\x1b\\a"), ["pa"]);
    }
}
