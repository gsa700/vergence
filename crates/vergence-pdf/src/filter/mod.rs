//! Stream filters (ISO 32000-2 §7.4): a stream's /Filter (one or an array)
//! with its /DecodeParms, applied in order. Ours, all of them.
//!
//! Image filters (DCTDecode, JPXDecode, CCITTFaxDecode, JBIG2Decode) are
//! not undone here: the bytes stop at them and [`Decoded::image`] names the
//! one left, for the picture code (step 5).

pub mod flate;

use crate::object::{Dict, Object};

/// The most a stream may decode to (a page's content, a font, a picture).
pub const MAX_DECODED: usize = 256 << 20;

/// A stream's bytes after its filters.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub data: Vec<u8>,
    /// An image filter still to be applied, with its parameters.
    pub image: Option<(Vec<u8>, Option<Dict>)>,
    /// False if a filter met damage (the bytes are what came before it) or
    /// an unknown filter (the bytes are as they were before it).
    pub ok: bool,
}

const IMAGE_FILTERS: [&[u8]; 4] = [b"DCTDecode", b"JPXDecode", b"CCITTFaxDecode", b"JBIG2Decode"];

/// Applies `filters` (names) with `parms` (each a dictionary or null) to
/// `data`. Abbreviated names (inline images) are taken too.
pub fn decode(data: &[u8], filters: &[Vec<u8>], parms: &[Option<Dict>]) -> Decoded {
    let mut cur = data.to_vec();
    let mut ok = true;
    for (i, f) in filters.iter().enumerate() {
        let p = parms.get(i).cloned().flatten();
        let name: &[u8] = match f.as_slice() {
            b"AHx" => b"ASCIIHexDecode",
            b"A85" => b"ASCII85Decode",
            b"LZW" => b"LZWDecode",
            b"Fl" => b"FlateDecode",
            b"RL" => b"RunLengthDecode",
            b"CCF" => b"CCITTFaxDecode",
            b"DCT" => b"DCTDecode",
            other => other,
        };
        if IMAGE_FILTERS.contains(&name) {
            return Decoded { data: cur, image: Some((name.to_vec(), p)), ok };
        }
        let (out, good) = match name {
            b"FlateDecode" => {
                let (o, g) = flate::inflate(&cur, MAX_DECODED);
                (predict(o, p.as_ref()), g)
            }
            b"LZWDecode" => {
                let early = p.as_ref().and_then(|d| d.int("EarlyChange")).unwrap_or(1) != 0;
                let (o, g) = lzw(&cur, early, MAX_DECODED);
                (predict(o, p.as_ref()), g)
            }
            b"ASCIIHexDecode" => (ascii_hex(&cur), true),
            b"ASCII85Decode" => ascii85(&cur),
            b"RunLengthDecode" => run_length(&cur, MAX_DECODED),
            b"Crypt" => (cur.clone(), true),
            _ => return Decoded { data: cur, image: None, ok: false },
        };
        cur = out;
        ok &= good;
    }
    Decoded { data: cur, image: None, ok }
}

/// A stream dictionary's /Filter and /DecodeParms as lists (direct values
/// only; the document resolves references before asking).
pub fn filters_of(dict: &Dict) -> (Vec<Vec<u8>>, Vec<Option<Dict>>) {
    let names = match dict.get("Filter").or_else(|| dict.get("F")) {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a.iter().filter_map(|o| o.as_name().map(<[u8]>::to_vec)).collect(),
        _ => Vec::new(),
    };
    let parms = match dict.get("DecodeParms").or_else(|| dict.get("DP")) {
        Some(Object::Dict(d)) => vec![Some(d.clone())],
        Some(Object::Array(a)) => a.iter().map(|o| o.as_dict().cloned()).collect(),
        _ => Vec::new(),
    };
    (names, parms)
}

/// /Predictor 2 (TIFF) and 10-15 (PNG), after Flate or LZW.
fn predict(data: Vec<u8>, parms: Option<&Dict>) -> Vec<u8> {
    let Some(p) = parms else { return data };
    let predictor = p.int("Predictor").unwrap_or(1);
    if predictor <= 1 {
        return data;
    }
    let colors = p.int("Colors").unwrap_or(1).clamp(1, 64) as usize;
    let bpc = p.int("BitsPerComponent").unwrap_or(8).clamp(1, 16) as usize;
    let columns = p.int("Columns").unwrap_or(1).clamp(1, 1 << 24) as usize;
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);
    if row == 0 {
        return data;
    }
    if predictor == 2 {
        // TIFF: only whole bytes per component handled (8 bits, as PDFs use).
        if bpc != 8 {
            return data;
        }
        let mut out = data;
        for line in out.chunks_mut(row) {
            for i in bpp..line.len() {
                line[i] = line[i].wrapping_add(line[i - bpp]);
            }
        }
        return out;
    }
    // PNG: each row has a filter byte first.
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let (kind, line) = (chunk[0], &chunk[1..]);
        let mut cur = vec![0u8; row];
        cur[..line.len()].copy_from_slice(line);
        for i in 0..row {
            let left = if i >= bpp { cur[i - bpp] } else { 0 };
            let up = prev[i];
            let ul = if i >= bpp { prev[i - bpp] } else { 0 };
            cur[i] = match kind {
                1 => cur[i].wrapping_add(left),
                2 => cur[i].wrapping_add(up),
                3 => cur[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
                4 => cur[i].wrapping_add(paeth(left, up, ul)),
                _ => cur[i],
            };
        }
        out.extend_from_slice(&cur[..line.len()]);
        prev = cur;
    }
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// LZW as PDF writes it: codes of 9 to 12 bits, most significant first,
/// 256 clears, 257 ends; `early`: the width grows one code early (the
/// default).
fn lzw(data: &[u8], early: bool, limit: usize) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    // The table as (prefix code, last byte, length); entries 0-255 are bytes.
    let mut table: Vec<(u16, u8, u32)> = (0..=255u16).map(|b| (u16::MAX, b as u8, 1)).collect();
    table.push((0, 0, 0));
    table.push((0, 0, 0));
    let (mut width, mut buf, mut have, mut pos) = (9u32, 0u32, 0u32, 0usize);
    let mut prev: Option<u16> = None;
    let mut scratch = Vec::new();
    loop {
        while have < width {
            // The data ran out before the end code: what came is kept.
            let Some(b) = data.get(pos) else { return (out, true) };
            buf = buf << 8 | *b as u32;
            have += 8;
            pos += 1;
        }
        let code = ((buf >> (have - width)) & ((1 << width) - 1)) as u16;
        have -= width;
        match code {
            256 => {
                table.truncate(258);
                width = 9;
                prev = None;
                continue;
            }
            257 => return (out, true),
            _ => {}
        }
        // The bytes of a code, written backwards through its prefixes.
        let emit = |c: u16, table: &[(u16, u8, u32)], scratch: &mut Vec<u8>| {
            scratch.clear();
            let mut c = c;
            let mut guard = 0;
            while c != u16::MAX && guard < 5000 {
                let (p, b, _) = table[c as usize];
                scratch.push(b);
                c = p;
                guard += 1;
            }
            scratch.reverse();
        };
        if (code as usize) < table.len() {
            emit(code, &table, &mut scratch);
            let first = scratch[0];
            if let Some(p) = prev
                && table.len() < 4096
            {
                let len = table[p as usize].2 + 1;
                table.push((p, first, len));
            }
        } else if code as usize == table.len()
            && let Some(p) = prev
        {
            emit(p, &table, &mut scratch);
            let f = scratch[0];
            scratch.push(f);
            if table.len() < 4096 {
                let len = table[p as usize].2 + 1;
                table.push((p, f, len));
            }
        } else {
            return (out, false);
        }
        if out.len() + scratch.len() > limit {
            return (out, false);
        }
        out.extend_from_slice(&scratch);
        prev = Some(code);
        let grow = table.len() + if early { 1 } else { 0 };
        width = match grow {
            n if n >= 2048 => 12,
            n if n >= 1024 => 11,
            n if n >= 512 => 10,
            _ => 9,
        };
    }
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut half: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        let Some(v) = crate::lexer::hexval(b) else { continue };
        match half.take() {
            Some(h) => out.push(h << 4 | v),
            None => half = Some(v),
        }
    }
    if let Some(h) = half {
        out.push(h << 4);
    }
    out
}

fn ascii85(data: &[u8]) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut n = 0;
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &b in body {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[n] = b - b'!';
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |a, d| a * 85 + *d as u64);
                    if v > u32::MAX as u64 {
                        return (out, false);
                    }
                    out.extend_from_slice(&(v as u32).to_be_bytes());
                    n = 0;
                }
            }
            b if crate::lexer::is_white(b) => {}
            _ => return (out, false),
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u64, |a, d| a * 85 + *d as u64).min(u32::MAX as u64);
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    (out, n != 1)
}

fn run_length(data: &[u8], limit: usize) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let len = data[i];
        i += 1;
        match len {
            128 => return (out, true),
            0..=127 => {
                let n = len as usize + 1;
                let Some(chunk) = data.get(i..i + n) else { return (out, false) };
                out.extend_from_slice(chunk);
                i += n;
            }
            _ => {
                let Some(&b) = data.get(i) else { return (out, false) };
                out.extend(std::iter::repeat_n(b, 257 - len as usize));
                i += 1;
            }
        }
        if out.len() > limit {
            out.truncate(limit);
            return (out, false);
        }
    }
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_filters() {
        assert_eq!(ascii_hex(b"48 65 6c 6C 6f>"), b"Hello");
        // "Man " is 9jqo^ in ASCII85; a short last group; z is four zeros.
        assert_eq!(ascii85(b"<~9jqo^~>"), (b"Man ".to_vec(), true));
        assert_eq!(ascii85(b"9jqo^BlbD-~>"), (b"Man is d".to_vec(), true));
        assert_eq!(ascii85(b"z~>"), (vec![0, 0, 0, 0], true));
    }

    #[test]
    fn run_length_runs_and_copies() {
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'x', 128], 100), (b"abcxxx".to_vec(), true));
    }

    #[test]
    fn lzw_as_pdf_writes_it() {
        // The PDF reference's example: "-----A---B" encodes as
        // 256 45 258 258 65 259 66 257 at 9 bits.
        let codes = [256u16, 45, 258, 258, 65, 259, 66, 257];
        let mut bits: Vec<u8> = Vec::new();
        let (mut buf, mut have) = (0u32, 0u32);
        for c in codes {
            buf = buf << 9 | c as u32;
            have += 9;
            while have >= 8 {
                bits.push((buf >> (have - 8)) as u8);
                have -= 8;
            }
        }
        if have > 0 {
            bits.push((buf << (8 - have)) as u8);
        }
        assert_eq!(lzw(&bits, true, 100), (b"-----A---B".to_vec(), true));
    }

    #[test]
    fn png_predictor_up() {
        // Two rows of 3 bytes, filter 2 (up) on the second.
        let raw = vec![0, 1, 2, 3, 2, 1, 1, 1];
        let p = Dict(vec![(b"Predictor".to_vec(), Object::Int(12)), (b"Columns".to_vec(), Object::Int(3))]);
        assert_eq!(predict(raw, Some(&p)), vec![1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn image_filters_are_left_for_the_picture_code() {
        let d = decode(b"\xff\xd8jpeg", &[b"DCTDecode".to_vec()], &[]);
        assert_eq!(d.image.as_ref().map(|(n, _)| n.as_slice()), Some(&b"DCTDecode"[..]));
        assert_eq!(d.data, b"\xff\xd8jpeg");
    }
}
