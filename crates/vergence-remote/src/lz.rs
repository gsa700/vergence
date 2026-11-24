//! Compression for frames' rectangles (remote milestone 1, question 4: our
//! own first, measured, before borrowing). The format is LZ4's *block*
//! format, a short published layout (sequences of literals and matches),
//! implemented here from its description: no borrowed code, and a format
//! others can read.
//!
//! A block is a series of sequences. Each starts with a token byte: its high
//! four bits the literals' count, its low four the match's length minus 4;
//! 15 in either means more length bytes follow (each adding its value, a
//! byte under 255 ending them). Then the literals, then the match's offset
//! (two bytes, little-endian, 1 to 65,535 back), then the match's extra
//! length bytes. The last sequence has literals only, and the last 5 bytes
//! of a block are always literals.
//!
//! Decoding is the side that faces untrusted input (the viewer decodes what
//! the host sends): every length and offset is checked, the output's size
//! is fixed in advance, and a block that does not end exactly there is an
//! error.

const MIN_MATCH: usize = 4;
/// The last bytes of a block that are always literals, and how near the end
/// a match may start (the format's rules).
const LAST_LITERALS: usize = 5;
const MF_LIMIT: usize = 12;
const HASH_BITS: u32 = 16;

fn read_u32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn hash(v: u32) -> usize {
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
}

fn put_length(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

/// `data` compressed as one block.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let n = data.len();
    let mut out = Vec::with_capacity(n / 2 + 16);
    let mut table = vec![0usize; 1 << HASH_BITS];
    let mut anchor = 0;
    let mut i = 0;
    if n >= MF_LIMIT {
        let limit = n - MF_LIMIT;
        while i <= limit {
            let v = read_u32(data, i);
            let h = hash(v);
            let candidate = table[h];
            table[h] = i + 1;
            let found = candidate > 0 && {
                let c = candidate - 1;
                i - c <= 65_535 && read_u32(data, c) == v
            };
            if !found {
                i += 1;
                continue;
            }
            let c = candidate - 1;
            // The match's length, keeping the last bytes for literals.
            let max = n - LAST_LITERALS;
            let mut len = MIN_MATCH;
            while i + len < max && data[c + len] == data[i + len] {
                len += 1;
            }
            let lit = i - anchor;
            let ml = len - MIN_MATCH;
            let token = ((lit.min(15) as u8) << 4) | ml.min(15) as u8;
            out.push(token);
            if lit >= 15 {
                put_length(&mut out, lit - 15);
            }
            out.extend_from_slice(&data[anchor..i]);
            out.extend_from_slice(&((i - c) as u16).to_le_bytes());
            if ml >= 15 {
                put_length(&mut out, ml - 15);
            }
            i += len;
            anchor = i;
        }
    }
    let lit = n - anchor;
    out.push((lit.min(15) as u8) << 4);
    if lit >= 15 {
        put_length(&mut out, lit - 15);
    }
    out.extend_from_slice(&data[anchor..]);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bad(pub &'static str);

/// A length's extra bytes, from `*i`.
fn get_length(b: &[u8], i: &mut usize, base: usize) -> Result<usize, Bad> {
    let mut n = base;
    if base == 15 {
        loop {
            let v = *b.get(*i).ok_or(Bad("a length runs past the end"))?;
            *i += 1;
            n = n.checked_add(v as usize).ok_or(Bad("a length too long"))?;
            if v != 255 {
                break;
            }
        }
    }
    Ok(n)
}

/// A block decoded into exactly `size` bytes; anything else is an error.
pub fn decompress(block: &[u8], size: usize) -> Result<Vec<u8>, Bad> {
    let mut out: Vec<u8> = Vec::with_capacity(size);
    let mut i = 0;
    loop {
        let token = *block.get(i).ok_or(Bad("the block ends with no last sequence"))?;
        i += 1;
        let lit = get_length(block, &mut i, (token >> 4) as usize)?;
        let end = i.checked_add(lit).ok_or(Bad("literals too long"))?;
        let src = block.get(i..end).ok_or(Bad("literals run past the end"))?;
        if out.len() + lit > size {
            return Err(Bad("more bytes than the rectangle holds"));
        }
        out.extend_from_slice(src);
        i = end;
        if i == block.len() {
            // The last sequence: literals only.
            break;
        }
        let off = block.get(i..i + 2).ok_or(Bad("an offset runs past the end"))?;
        let off = u16::from_le_bytes([off[0], off[1]]) as usize;
        i += 2;
        if off == 0 || off > out.len() {
            return Err(Bad("a match before the start"));
        }
        let ml = get_length(block, &mut i, (token & 15) as usize)? + MIN_MATCH;
        if out.len() + ml > size {
            return Err(Bad("more bytes than the rectangle holds"));
        }
        // Byte by byte: a match may overlap what it is copying (a run).
        let from = out.len() - off;
        for k in 0..ml {
            let v = out[from + k];
            out.push(v);
        }
    }
    if out.len() != size {
        return Err(Bad("fewer bytes than the rectangle holds"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screenish(w: usize, h: usize) -> Vec<u8> {
        // Flat ground, a "window" of another color, lines of "text".
        let mut v = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let px = if (100..400).contains(&x) && (50..300).contains(&y) {
                    if y % 20 < 12 && (x / 3 + y) % 7 < 4 { [20, 20, 20, 255] } else { [250, 250, 250, 255] }
                } else {
                    [30, 30, 34, 255]
                };
                v.extend_from_slice(&px);
            }
        }
        v
    }

    #[test]
    fn round_trips() {
        let mut seed = 7u32;
        let mut noise = vec![0u8; 50_000];
        for b in noise.iter_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *b = (seed >> 16) as u8;
        }
        for data in [
            Vec::new(),
            vec![1],
            vec![9; 11],
            vec![9; 100_000],
            noise,
            screenish(640, 360),
            b"abcabcabcabcabcabcabcabcabcabcabcabc".to_vec(),
        ] {
            let c = compress(&data);
            assert_eq!(decompress(&c, data.len()).unwrap(), data);
        }
    }

    #[test]
    fn a_screen_compresses_well() {
        let data = screenish(1920, 1080);
        let c = compress(&data);
        assert!(c.len() * 20 < data.len(), "{} of {}", c.len(), data.len());
    }

    #[test]
    fn hostile_blocks_are_refused() {
        let good = compress(&screenish(64, 64));
        let size = 64 * 64 * 4;
        assert!(decompress(&good, size - 1).is_err(), "too small a size");
        assert!(decompress(&good, size + 1).is_err(), "too large a size");
        assert!(decompress(&good[..good.len() - 1], size).is_err(), "truncated");
        assert!(decompress(&[], size).is_err());
        // A match before the start; an offset of 0; a length that never ends.
        assert!(decompress(&[0x10, b'a', 5, 0, 0x00], 10).is_err());
        assert!(decompress(&[0x10, b'a', 0, 0, 0x00], 10).is_err());
        assert!(decompress(&[0xF0, 255, 255, 255], 10).is_err());
        // A huge match into a small rectangle.
        assert!(decompress(&[0x1F, b'a', 1, 0, 255, 255, 255, 0, 0x00], 16).is_err());
    }
}
