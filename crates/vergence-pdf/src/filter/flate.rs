//! DEFLATE (RFC 1951) inside a zlib wrapper (RFC 1950): our own decoder,
//! for /FlateDecode. Written in the manner of Mark Adler's puff: canonical
//! Huffman codes decoded a bit at a time with counts and symbol tables,
//! which is short and easy to check, and fast enough for page content.
//!
//! **Untrusted input.** Every table is checked for over-subscription, every
//! distance against what has been written, and the output against `limit`.
//! A damaged stream gives back what was decoded before the damage (as
//! other readers do: pages with a bad last block still draw), with `ok`
//! false.

const MAX_BITS: usize = 15;

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    count: u32,
}

impl Bits<'_> {
    fn bits(&mut self, need: u32) -> Option<u32> {
        while self.count < need {
            let b = *self.data.get(self.pos)?;
            self.pos += 1;
            self.buf |= (b as u32) << self.count;
            self.count += 8;
        }
        let v = self.buf & ((1u32 << need) - 1);
        self.buf >>= need;
        self.count -= need;
        Some(v)
    }

    fn align(&mut self) {
        self.buf = 0;
        self.count = 0;
    }
}

struct Huffman {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

impl Huffman {
    /// From code lengths; None if the lengths over-subscribe the code.
    fn new(lengths: &[u8]) -> Option<Huffman> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for c in count.iter().skip(1) {
            left <<= 1;
            left -= *c as i32;
            if left < 0 {
                return None;
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Some(Huffman { count, symbol })
    }

    fn decode(&self, b: &mut Bits) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS {
            code |= b.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return self.symbol.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Decodes a zlib stream (or a bare DEFLATE one, if the zlib header is not
/// there), at most `limit` bytes out. The bool: whether it ended cleanly.
pub fn inflate(data: &[u8], limit: usize) -> (Vec<u8>, bool) {
    // A zlib header: CM 8, a check that divides by 31, no preset dictionary.
    let raw = match data {
        [cmf, flg, ..] if cmf & 0x0f == 8 && ((*cmf as u16) << 8 | *flg as u16).is_multiple_of(31) && flg & 0x20 == 0 => &data[2..],
        _ => data,
    };
    let mut out = Vec::with_capacity(raw.len().saturating_mul(4).min(limit).min(64 << 20));
    let ok = blocks(raw, &mut out, limit).is_some();
    (out, ok)
}

fn blocks(data: &[u8], out: &mut Vec<u8>, limit: usize) -> Option<()> {
    let mut b = Bits { data, pos: 0, buf: 0, count: 0 };
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => stored(&mut b, out, limit)?,
            1 => {
                let (lit, dist) = fixed();
                codes(&mut b, out, &lit, &dist, limit)?
            }
            2 => {
                let (lit, dist) = dynamic(&mut b)?;
                codes(&mut b, out, &lit, &dist, limit)?
            }
            _ => return None,
        }
        if last == 1 {
            return Some(());
        }
    }
}

fn stored(b: &mut Bits, out: &mut Vec<u8>, limit: usize) -> Option<()> {
    b.align();
    let d = b.data;
    let (len, nlen) = (u16::from_le_bytes([*d.get(b.pos)?, *d.get(b.pos + 1)?]), u16::from_le_bytes([*d.get(b.pos + 2)?, *d.get(b.pos + 3)?]));
    if len != !nlen {
        return None;
    }
    b.pos += 4;
    let end = b.pos.checked_add(len as usize)?;
    let chunk = d.get(b.pos..end.min(d.len()))?;
    if out.len() + chunk.len() > limit {
        return None;
    }
    out.extend_from_slice(chunk);
    b.pos = end;
    (end <= d.len()).then_some(())
}

fn fixed() -> (Huffman, Huffman) {
    let mut l = [0u8; 288];
    l[..144].fill(8);
    l[144..256].fill(9);
    l[256..280].fill(7);
    l[280..].fill(8);
    (Huffman::new(&l).expect("the fixed code"), Huffman::new(&[5u8; 30]).expect("the fixed distances"))
}

fn dynamic(b: &mut Bits) -> Option<(Huffman, Huffman)> {
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let nlen = b.bits(5)? as usize + 257;
    let ndist = b.bits(5)? as usize + 1;
    let ncode = b.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return None;
    }
    let mut lengths = [0u8; 19];
    for &i in ORDER.iter().take(ncode) {
        lengths[i] = b.bits(3)? as u8;
    }
    let lencode = Huffman::new(&lengths)?;
    let mut all = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = lencode.decode(b)?;
        if sym < 16 {
            all[i] = sym as u8;
            i += 1;
            continue;
        }
        let (value, repeat) = match sym {
            16 => (*all.get(i.checked_sub(1)?)?, 3 + b.bits(2)? as usize),
            17 => (0, 3 + b.bits(3)? as usize),
            18 => (0, 11 + b.bits(7)? as usize),
            _ => return None,
        };
        if i + repeat > nlen + ndist {
            return None;
        }
        all[i..i + repeat].fill(value);
        i += repeat;
    }
    // A block must be able to end.
    if all[256] == 0 {
        return None;
    }
    Some((Huffman::new(&all[..nlen])?, Huffman::new(&all[nlen..])?))
}

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman, limit: usize) -> Option<()> {
    loop {
        let sym = lit.decode(b)? as usize;
        if sym < 256 {
            if out.len() >= limit {
                return None;
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Some(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return None;
            }
            let len = LEN_BASE[s] as usize + b.bits(LEN_EXTRA[s] as u32)? as usize;
            let d = dist.decode(b)? as usize;
            if d >= 30 {
                return None;
            }
            let back = DIST_BASE[d] as usize + b.bits(DIST_EXTRA[d] as u32)? as usize;
            if back > out.len() || out.len() + len > limit {
                return None;
            }
            let from = out.len() - back;
            for k in 0..len {
                let v = out[from + k];
                out.push(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_fixed_and_dynamic_blocks() {
        // Python's zlib.compress(b"Hello, Hello, Hello, Hello!", 9): fixed Huffman, a back reference.
        let z = [0x78, 0xda, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0xd7, 0x51, 0xf0, 0xc0, 0xa4, 0x14, 0x01, 0x7d, 0x2c, 0x08, 0xd6];
        let (out, ok) = inflate(&z, 1 << 20);
        assert!(ok);
        assert_eq!(out, b"Hello, Hello, Hello, Hello!");
        // A stored block: zlib.compress(b"abc", 0).
        let s = [0x78, 0x01, 0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c', 0x02, 0x4d, 0x01, 0x27];
        assert_eq!(inflate(&s, 100), (b"abc".to_vec(), true));
        // Dynamic Huffman, as most PDF streams are (Python's zlib, level 9).
        let (out, ok) = inflate(include_bytes!("../../testdata/dynamic.z"), 1 << 20);
        assert!(ok);
        assert_eq!(out, include_bytes!("../../testdata/dynamic.txt"));
    }

    #[test]
    fn a_limit_stops_a_bomb() {
        let z = [0x78, 0xda, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0xd7, 0x51, 0xf0, 0xc0, 0xa4, 0x14, 0x01, 0x7d, 0x2c, 0x08, 0xd6];
        let (out, ok) = inflate(&z, 10);
        assert!(!ok);
        assert!(out.len() <= 10);
    }

    #[test]
    fn damage_gives_back_what_came_before() {
        let z = [0x78, 0xda, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0xd7, 0x51, 0xf0, 0xc0];
        let (out, ok) = inflate(&z, 1 << 20);
        assert!(!ok);
        assert!(b"Hello, Hello, Hello, Hello!".starts_with(&out));
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for len in [0, 1, 2, 7, 64, 1000] {
            for _ in 0..200 {
                let data: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x as u8
                    })
                    .collect();
                let (out, _) = inflate(&data, 1 << 16);
                assert!(out.len() <= 1 << 16);
            }
        }
    }
}
