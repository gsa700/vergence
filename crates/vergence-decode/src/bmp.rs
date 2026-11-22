//! BMP pictures, decoded by our own code (files milestone 1, answer 3):
//! the Windows formats in use: a BITMAPINFOHEADER (40 bytes) or its V4/V5
//! successors (108, 124), uncompressed with 1, 4, 8 (a palette), 24 or 32
//! bits a pixel, or bit fields (masks) with 16 or 32; rows bottom-up, or
//! top-down for a negative height. Run-length compression and the old OS/2
//! header are refused. Every size is checked before it is used.

/// The largest side, and the most pixels, accepted.
const MAX_SIDE: u32 = 32768;
const MAX_PIXELS: u64 = 1 << 28;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// One channel from a pixel by its mask, scaled to 0-255 (a missing mask: 255).
fn channel(px: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    let v = (px & mask) >> shift;
    let max = (1u64 << bits) - 1;
    ((v as u64 * 255 + max / 2) / max) as u8
}

/// A BMP file's width, height and straight RGBA pixels.
pub fn decode(b: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let bad = |why: &str| format!("BMP: {why}");
    if b.get(..2) != Some(b"BM") {
        return Err(bad("not a BMP"));
    }
    let offset = u32_at(b, 10).ok_or_else(|| bad("short"))? as usize;
    let header = u32_at(b, 14).ok_or_else(|| bad("short"))?;
    if !matches!(header, 40 | 52 | 56 | 108 | 124) {
        return Err(bad(&format!("header of {header} bytes not supported")));
    }
    let width = u32_at(b, 18).ok_or_else(|| bad("short"))? as i32;
    let height = u32_at(b, 22).ok_or_else(|| bad("short"))? as i32;
    let bpp = u16_at(b, 28).ok_or_else(|| bad("short"))?;
    let compression = u32_at(b, 30).ok_or_else(|| bad("short"))?;
    let top_down = height < 0;
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
        return Err(bad(&format!("size {w} x {h} not accepted")));
    }
    // Masks: bit fields after a 40-byte header, or inside a V4/V5 one.
    let (rm, gm, bm, am) = match (compression, bpp) {
        (0, 32) => (0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0),
        (0, 16) => (0x7c00, 0x03e0, 0x001f, 0),
        (3 | 6, 16 | 32) => {
            let at = 54;
            let m = |i: usize| u32_at(b, at + 4 * i).ok_or_else(|| bad("short masks"));
            (m(0)?, m(1)?, m(2)?, if header >= 56 || compression == 6 { m(3)? } else { 0 })
        }
        (0, 1 | 4 | 8 | 24) => (0, 0, 0, 0),
        _ => return Err(bad(&format!("{bpp} bits with compression {compression} not supported"))),
    };
    // A palette (BGRx) for 1, 4 and 8 bits, after the header.
    let palette: Vec<[u8; 3]> = if bpp <= 8 {
        let colors = match u32_at(b, 46).unwrap_or(0) {
            0 => 1u32 << bpp,
            n => n.min(1 << bpp),
        } as usize;
        let at = 14 + header as usize;
        (0..colors)
            .map(|i| b.get(at + 4 * i..at + 4 * i + 3).map(|c| [c[2], c[1], c[0]]).ok_or_else(|| bad("short palette")))
            .collect::<Result<_, _>>()?
    } else {
        Vec::new()
    };
    let row_bytes = ((w as usize * bpp as usize).div_ceil(32)) * 4;
    let need = offset
        .checked_add(row_bytes.checked_mul(h as usize).ok_or_else(|| bad("too big"))?)
        .ok_or_else(|| bad("too big"))?;
    if b.len() < need {
        return Err(bad("pixels cut short"));
    }
    let mut out = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        let src_row = if top_down { y } else { h as usize - 1 - y };
        let row = &b[offset + src_row * row_bytes..offset + (src_row + 1) * row_bytes];
        let dst = &mut out[y * w as usize * 4..(y + 1) * w as usize * 4];
        for x in 0..w as usize {
            let px: [u8; 4] = match bpp {
                1 | 4 | 8 => {
                    let bit = x * bpp as usize;
                    let byte = row[bit / 8];
                    let idx = (byte >> (8 - bpp as usize - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    let c = palette.get(idx as usize).copied().unwrap_or([0, 0, 0]);
                    [c[0], c[1], c[2], 255]
                }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                16 => {
                    let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32;
                    [channel(v, rm), channel(v, gm), channel(v, bm), if am == 0 { 255 } else { channel(v, am) }]
                }
                _ => {
                    let v = u32::from_le_bytes([row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]]);
                    [channel(v, rm), channel(v, gm), channel(v, bm), if am == 0 { 255 } else { channel(v, am) }]
                }
            };
            dst[x * 4..x * 4 + 4].copy_from_slice(&px);
        }
    }
    Ok((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMP file of `bpp` bits with a 40-byte header, rows given bottom-up already padded.
    fn file(w: i32, h: i32, bpp: u16, palette: &[[u8; 4]], pixels: &[u8]) -> Vec<u8> {
        let offset = 14 + 40 + palette.len() * 4;
        let mut b = b"BM".to_vec();
        b.extend(((offset + pixels.len()) as u32).to_le_bytes());
        b.extend([0u8; 4]);
        b.extend((offset as u32).to_le_bytes());
        b.extend(40u32.to_le_bytes());
        b.extend(w.to_le_bytes());
        b.extend(h.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(bpp.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend((pixels.len() as u32).to_le_bytes());
        b.extend([0u8; 8]);
        b.extend((palette.len() as u32).to_le_bytes());
        b.extend([0u8; 4]);
        for c in palette {
            b.extend(c);
        }
        b.extend(pixels);
        b
    }

    #[test]
    fn twenty_four_bits_bottom_up() {
        // 2 x 2: bottom row blue, green; top row red, white. Rows padded to 8 bytes.
        let px = [255, 0, 0, 0, 255, 0, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0];
        let (w, h, rgba) = decode(&file(2, 2, 24, &[], &px)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&rgba[..8], &[255, 0, 0, 255, 255, 255, 255, 255], "top row first");
        assert_eq!(&rgba[8..], &[0, 0, 255, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn a_palette_and_top_down() {
        // 3 x 1, 8 bits: indexes 1, 0, 1 of a two-color palette.
        let (w, h, rgba) = decode(&file(3, -1, 8, &[[0, 0, 0, 0], [0, 128, 255, 0]], &[1, 0, 1, 0])).unwrap();
        assert_eq!((w, h), (3, 1));
        assert_eq!(rgba, vec![255, 128, 0, 255, 0, 0, 0, 255, 255, 128, 0, 255]);
        // 1 bit: 0b1010_0000 over 3 pixels.
        let (_, _, rgba) =
            decode(&file(3, 1, 1, &[[0, 0, 0, 0], [255, 255, 255, 0]], &[0b1010_0000, 0, 0, 0])).unwrap();
        assert_eq!(rgba.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), vec![255, 0, 255]);
    }

    #[test]
    fn bad_files_are_refused_not_trusted() {
        assert!(decode(b"BM").is_err());
        assert!(decode(b"PK\x03\x04").is_err());
        assert!(decode(&file(100_000, 1, 24, &[], &[0; 4])).is_err(), "too wide");
        assert!(decode(&file(4, 4, 24, &[], &[0; 4])).is_err(), "pixels cut short");
        let mut rle = file(1, 1, 8, &[[0; 4]], &[0; 4]);
        rle[30] = 1;
        assert!(decode(&rle).is_err(), "run-length compression");
    }
}
