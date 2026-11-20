//! CCITT fax decoding (ITU-T T.4 and T.6; ISO 32000-2 §7.4.6), our own:
//! Group 3 one-dimensional (K = 0), mixed (K > 0) and Group 4 (K < 0),
//! with EndOfLine, EncodedByteAlign, EndOfBlock and BlackIs1. What scanned
//! pages and many logos in PDFs are.
//!
//! The run-length codes below are T.4's tables, written as bit strings so
//! each can be checked against the standard by eye; the tests decode
//! libtiff's encodings of one bitmap in all three schemes and compare.
//! Untrusted: rows, columns and the work done are bounded; damage ends the
//! picture where it is (the rows so far are kept, the rest white).

use std::sync::OnceLock;

const MAX_COLUMNS: usize = 1 << 16;
const MAX_PIXELS: usize = 1 << 28;

#[rustfmt::skip]
const WHITE: &[(&str, u16)] = &[
    ("00110101", 0), ("000111", 1), ("0111", 2), ("1000", 3), ("1011", 4), ("1100", 5), ("1110", 6), ("1111", 7),
    ("10011", 8), ("10100", 9), ("00111", 10), ("01000", 11), ("001000", 12), ("000011", 13), ("110100", 14), ("110101", 15),
    ("101010", 16), ("101011", 17), ("0100111", 18), ("0001100", 19), ("0001000", 20), ("0010111", 21), ("0000011", 22), ("0000100", 23),
    ("0101000", 24), ("0101011", 25), ("0010011", 26), ("0100100", 27), ("0011000", 28), ("00000010", 29), ("00000011", 30), ("00011010", 31),
    ("00011011", 32), ("00010010", 33), ("00010011", 34), ("00010100", 35), ("00010101", 36), ("00010110", 37), ("00010111", 38), ("00101000", 39),
    ("00101001", 40), ("00101010", 41), ("00101011", 42), ("00101100", 43), ("00101101", 44), ("00000100", 45), ("00000101", 46), ("00001010", 47),
    ("00001011", 48), ("01010010", 49), ("01010011", 50), ("01010100", 51), ("01010101", 52), ("00100100", 53), ("00100101", 54), ("01011000", 55),
    ("01011001", 56), ("01011010", 57), ("01011011", 58), ("01001010", 59), ("01001011", 60), ("00110010", 61), ("00110011", 62), ("00110100", 63),
    ("11011", 64), ("10010", 128), ("010111", 192), ("0110111", 256), ("00110110", 320), ("00110111", 384), ("01100100", 448), ("01100101", 512),
    ("01101000", 576), ("01100111", 640), ("011001100", 704), ("011001101", 768), ("011010010", 832), ("011010011", 896), ("011010100", 960), ("011010101", 1024),
    ("011010110", 1088), ("011010111", 1152), ("011011000", 1216), ("011011001", 1280), ("011011010", 1344), ("011011011", 1408), ("010011000", 1472), ("010011001", 1536),
    ("010011010", 1600), ("011000", 1664), ("010011011", 1728),
];

#[rustfmt::skip]
const BLACK: &[(&str, u16)] = &[
    ("0000110111", 0), ("010", 1), ("11", 2), ("10", 3), ("011", 4), ("0011", 5), ("0010", 6), ("00011", 7),
    ("000101", 8), ("000100", 9), ("0000100", 10), ("0000101", 11), ("0000111", 12), ("00000100", 13), ("00000111", 14), ("000011000", 15),
    ("0000010111", 16), ("0000011000", 17), ("0000001000", 18), ("00001100111", 19), ("00001101000", 20), ("00001101100", 21), ("00000110111", 22), ("00000101000", 23),
    ("00000010111", 24), ("00000011000", 25), ("000011001010", 26), ("000011001011", 27), ("000011001100", 28), ("000011001101", 29), ("000001101000", 30), ("000001101001", 31),
    ("000001101010", 32), ("000001101011", 33), ("000011010010", 34), ("000011010011", 35), ("000011010100", 36), ("000011010101", 37), ("000011010110", 38), ("000011010111", 39),
    ("000001101100", 40), ("000001101101", 41), ("000011011010", 42), ("000011011011", 43), ("000001010100", 44), ("000001010101", 45), ("000001010110", 46), ("000001010111", 47),
    ("000001100100", 48), ("000001100101", 49), ("000001010010", 50), ("000001010011", 51), ("000000100100", 52), ("000000110111", 53), ("000000111000", 54), ("000000100111", 55),
    ("000000101000", 56), ("000001011000", 57), ("000001011001", 58), ("000000101011", 59), ("000000101100", 60), ("000001011010", 61), ("000001100110", 62), ("000001100111", 63),
    ("0000001111", 64), ("000011001000", 128), ("000011001001", 192), ("000001011011", 256), ("000000110011", 320), ("000000110100", 384), ("000000110101", 448), ("0000001101100", 512),
    ("0000001101101", 576), ("0000001001010", 640), ("0000001001011", 704), ("0000001001100", 768), ("0000001001101", 832), ("0000001110010", 896), ("0000001110011", 960), ("0000001110100", 1024),
    ("0000001110101", 1088), ("0000001110110", 1152), ("0000001110111", 1216), ("0000001010010", 1280), ("0000001010011", 1344), ("0000001010100", 1408), ("0000001010101", 1472), ("0000001011010", 1536),
    ("0000001011011", 1600), ("0000001100100", 1664), ("0000001100101", 1728),
];

/// The extended make-up codes, the same for both colors.
#[rustfmt::skip]
const EXTENDED: &[(&str, u16)] = &[
    ("00000001000", 1792), ("00000001100", 1856), ("00000001101", 1920), ("000000010010", 1984), ("000000010011", 2048), ("000000010100", 2112), ("000000010101", 2176),
    ("000000010110", 2240), ("000000010111", 2304), ("000000011100", 2368), ("000000011101", 2432), ("000000011110", 2496), ("000000011111", 2560),
];

const PEEK: u32 = 13;

/// A run-length table indexed by the next 13 bits: (code length, run).
struct Table(Vec<(u8, u16)>);

fn table(codes: &[(&str, u16)]) -> Table {
    let mut t = vec![(0u8, 0u16); 1 << PEEK];
    for (bits, run) in codes.iter().chain(EXTENDED) {
        let len = bits.len() as u32;
        let code = u32::from_str_radix(bits, 2).expect("a bit string");
        let base = code << (PEEK - len);
        for k in 0..(1u32 << (PEEK - len)) {
            t[(base | k) as usize] = (len as u8, *run);
        }
    }
    Table(t)
}

fn tables() -> &'static (Table, Table) {
    static T: OnceLock<(Table, Table)> = OnceLock::new();
    T.get_or_init(|| (table(WHITE), table(BLACK)))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub k: i64,
    pub columns: usize,
    pub rows: usize,
    pub end_of_line: bool,
    pub byte_align: bool,
    pub end_of_block: bool,
    pub black_is_1: bool,
}

impl Default for Params {
    fn default() -> Params {
        Params { k: 0, columns: 1728, rows: 0, end_of_line: false, byte_align: false, end_of_block: true, black_is_1: false }
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    /// The next `n` bits (at most 32), zeros past the end.
    fn peek(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        // Five bytes from the current one cover any 32 bits at any offset.
        let at = self.pos / 8;
        let mut window = 0u64;
        for k in 0..5 {
            window = window << 8 | u64::from(self.data.get(at + k).copied().unwrap_or(0));
        }
        ((window << (24 + self.pos % 8)) >> (64 - n)) as u32
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    fn bit(&mut self) -> u32 {
        let b = self.peek(1);
        self.pos += 1;
        b
    }

    fn done(&self) -> bool {
        self.pos >= self.data.len() * 8
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    /// Past fill zeros and an EOL (000000000001), if there is one here.
    fn eol(&mut self) -> bool {
        let start = self.pos;
        while !self.done() && self.peek(1) == 0 {
            self.pos += 1;
            if self.pos - start > 4096 {
                break;
            }
        }
        if self.pos - start >= 11 && self.peek(1) == 1 {
            self.pos += 1;
            return true;
        }
        self.pos = start;
        false
    }

    /// One run of a color: make-up codes, then a terminating one.
    fn run(&mut self, black: bool) -> Option<usize> {
        let t = if black { &tables().1 } else { &tables().0 };
        let mut total = 0usize;
        loop {
            let (len, run) = t.0[self.peek(PEEK) as usize];
            if len == 0 || self.done() {
                return None;
            }
            self.skip(len as usize);
            total += run as usize;
            if run < 64 {
                return Some(total);
            }
            if total > MAX_COLUMNS * 2 {
                return None;
            }
        }
    }
}

/// The 2D modes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Pass,
    Horizontal,
    Vertical(i32),
}

fn mode(b: &mut Bits) -> Option<Mode> {
    let codes = [(1, 0b1, Mode::Vertical(0)), (3, 0b011, Mode::Vertical(1)), (3, 0b010, Mode::Vertical(-1)), (3, 0b001, Mode::Horizontal), (4, 0b0001, Mode::Pass), (6, 0b000011, Mode::Vertical(2)), (6, 0b000010, Mode::Vertical(-2)), (7, 0b0000011, Mode::Vertical(3)), (7, 0b0000010, Mode::Vertical(-3))];
    for (len, code, m) in codes {
        if b.peek(len) == code {
            b.skip(len as usize);
            return Some(m);
        }
    }
    None
}

/// Decodes to packed rows, (columns + 7) / 8 bytes each, a 1 bit black
/// when `black_is_1`, else white (PDF's default: 0 is black).
pub fn decode(data: &[u8], p: &Params) -> Vec<u8> {
    let columns = p.columns.clamp(1, MAX_COLUMNS);
    let stride = columns.div_ceil(8);
    let max_rows = if p.rows > 0 { p.rows.min(MAX_PIXELS / columns) } else { MAX_PIXELS / columns };
    let mut out: Vec<u8> = Vec::new();
    let mut b = Bits { data, pos: 0 };
    // The reference line's changing elements (positions where the color flips), white first.
    let mut reference: Vec<usize> = Vec::new();
    let mut rows = 0;
    while rows < max_rows && !b.done() {
        // EncodedByteAlign: a row starts on a byte, unless an EOL's fill
        // zeros (skipped with it) already put it there.
        if p.byte_align && (p.k < 0 || !p.end_of_line) {
            b.align();
        }
        if p.k >= 0 {
            // Two EOLs in a row: the end of the picture (RTC).
            if b.eol() && b.peek(12) == 1 {
                break;
            }
        } else if p.end_of_block && b.peek(24) == 0x001001 {
            // EOFB.
            break;
        }
        let two_d = match p.k {
            k if k < 0 => true,
            0 => false,
            _ => b.bit() == 0,
        };
        let changes = if two_d { row_2d(&mut b, &reference, columns) } else { row_1d(&mut b, columns) };
        let Some(changes) = changes else { break };
        // The row into bits.
        let mut row = vec![if p.black_is_1 { 0u8 } else { 0xFF }; stride];
        let mut black = false;
        let mut x = 0usize;
        for &c in changes.iter().chain(std::iter::once(&columns)) {
            let c = c.min(columns);
            if black {
                for px in x..c {
                    let (byte, bit) = (px / 8, 7 - px % 8);
                    if p.black_is_1 { row[byte] |= 1 << bit } else { row[byte] &= !(1 << bit) }
                }
            }
            x = c;
            black = !black;
            if x >= columns {
                break;
            }
        }
        out.extend_from_slice(&row);
        reference = changes;
        rows += 1;
    }
    if p.rows > 0 && rows < p.rows.min(max_rows) {
        // The rows the data did not reach: white.
        out.resize(p.rows.min(max_rows) * stride, if p.black_is_1 { 0 } else { 0xFF });
    }
    out
}

/// A row of alternating white and black runs (Modified Huffman).
fn row_1d(b: &mut Bits, columns: usize) -> Option<Vec<usize>> {
    let mut changes = Vec::new();
    let mut x = 0usize;
    let mut black = false;
    while x < columns {
        let run = b.run(black)?;
        x = (x + run).min(columns);
        changes.push(x);
        black = !black;
    }
    // The last change at the edge is no change.
    if changes.last() == Some(&columns) {
        changes.pop();
    }
    Some(changes)
}

/// A row coded against the reference line (T.4 2D, T.6).
fn row_2d(b: &mut Bits, reference: &[usize], columns: usize) -> Option<Vec<usize>> {
    let mut changes: Vec<usize> = Vec::new();
    let mut a0: isize = -1;
    let mut black = false;
    let mut guard = 0;
    // Every reference change before `lo` is at or left of a0. a0 only moves
    // right on good data, so the search for b1 starts there instead of at
    // the row's start (step 9: halftone scans have thousands of changes a
    // row, and the scan from 0 made each row quadratic). If a0 moves left
    // (a negative vertical step), it starts from 0 again: the same b1 as a
    // whole scan, on any data.
    let mut lo = 0;
    let mut last_a0 = a0;
    while a0 < columns as isize {
        guard += 1;
        if guard > columns * 4 + 16 {
            return None;
        }
        if a0 < last_a0 {
            lo = 0;
        }
        last_a0 = a0;
        while lo < reference.len() && reference[lo] as isize <= a0 {
            lo += 1;
        }
        // b1: the first change on the reference line right of a0 to the
        // other color; b2 the next one.
        let mut i = lo;
        while i < reference.len() && (reference[i] as isize <= a0 || (i % 2 == 1) != black) {
            i += 1;
        }
        let b1 = reference.get(i).copied().unwrap_or(columns);
        let b2 = reference.get(i + 1).copied().unwrap_or(columns);
        match mode(b)? {
            Mode::Pass => {
                a0 = b2 as isize;
            }
            Mode::Horizontal => {
                let start = a0.max(0) as usize;
                let r1 = b.run(black)?;
                let r2 = b.run(!black)?;
                let a1 = (start + r1).min(columns);
                let a2 = (a1 + r2).min(columns);
                changes.push(a1);
                changes.push(a2);
                a0 = a2 as isize;
            }
            Mode::Vertical(d) => {
                let a1 = (b1 as isize + d as isize).clamp(0, columns as isize);
                changes.push(a1 as usize);
                a0 = a1;
                black = !black;
            }
        }
    }
    // Changes at the right edge are no changes.
    while changes.last().is_some_and(|c| *c >= columns) {
        changes.pop();
    }
    Some(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test bitmap: a PBM (1 is black), 3000 x 40.
    fn bitmap() -> (usize, usize, Vec<u8>) {
        let d = include_bytes!("../testdata/ccitt-t.pbm");
        let header_end = d.iter().enumerate().filter(|(_, b)| **b == b'\n').nth(1).unwrap().0 + 1;
        (3000, 40, d[header_end..].to_vec())
    }

    fn check(raw: &[u8], k: i64, end_of_line: bool) {
        let (w, h, want) = bitmap();
        let got = decode(raw, &Params { k, columns: w, rows: h, end_of_line, black_is_1: true, ..Default::default() });
        assert_eq!(got.len(), want.len());
        let stride = w.div_ceil(8);
        for row in 0..h {
            let (g, e) = (&got[row * stride..(row + 1) * stride], &want[row * stride..(row + 1) * stride]);
            // The PBM's last byte pads with zeros; compare the real columns only.
            let full = w / 8;
            assert_eq!(&g[..full], &e[..full], "k {k}: row {row} differs");
        }
    }

    #[test]
    fn group_4_as_libtiff_writes_it() {
        check(include_bytes!("../testdata/ccitt-g4.raw"), -1, false);
    }

    #[test]
    fn group_3_two_dimensional() {
        check(include_bytes!("../testdata/ccitt-g3_2d.raw"), 2, true);
    }

    #[test]
    fn group_3_one_dimensional() {
        check(include_bytes!("../testdata/ccitt-g3_1d.raw"), 0, true);
    }

    #[test]
    fn the_tables_are_prefix_codes() {
        for (name, t) in [("white", WHITE), ("black", BLACK)] {
            let all: Vec<&str> = t.iter().chain(EXTENDED).map(|(b, _)| *b).collect();
            for (i, a) in all.iter().enumerate() {
                for (j, b) in all.iter().enumerate() {
                    assert!(i == j || !b.starts_with(a), "{name}: {a} is a prefix of {b}");
                }
            }
            assert_eq!(t.len(), 64 + 27, "{name}");
        }
    }

    #[test]
    fn damage_never_panics() {
        let raw = include_bytes!("../testdata/ccitt-g4.raw").to_vec();
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for i in 0..300 {
            let mut f = raw.clone();
            for _ in 0..8 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let k = x as usize % f.len();
                f[k] = (x >> 40) as u8;
            }
            let k = [-1, 0, 2][i % 3];
            let out = decode(&f, &Params { k, columns: 3000, rows: 40, ..Default::default() });
            assert!(out.len() <= 3000usize.div_ceil(8) * 40);
        }
    }
}
