//! Image XObjects and inline images (ISO 32000-2 §8.9) into premultiplied
//! RGBA for the rasterizer: samples of 1 to 16 bits per component in any
//! color space, /Decode arrays, stencil masks (/ImageMask), color-key
//! masks (/Mask as an array), explicit masks (/Mask as a stream) and soft
//! masks (/SMask). JPEG (DCTDecode) through vergence-decode's decoder
//! (the image crate, borrowed), its samples then going the same way as
//! any; CCITT fax (ccitt.rs, ours) the same way as 1-bit samples; JPEG
//! 2000 is a placeholder for now (Dave, 2026-11-19); JBIG2 through
//! jbig2.rs (ours), arithmetic-coded streams so far. What cannot be
//! decoded comes back as [`Decoded::Undecoded`].

use crate::color::ColorSpace;
use crate::filter;
use crate::function::Bits;
use crate::object::{Dict, Object};
use crate::Document;
use vergence_raster::Image;

/// The most samples an image may have.
pub const MAX_PIXELS: usize = 1 << 26;

pub enum Decoded {
    /// Colors with their alpha.
    Color(Image),
    /// A stencil: alpha only (0 to 255 per pixel), painted in the fill color.
    Stencil { width: usize, height: usize, alpha: Vec<u8> },
    /// Encoded with a picture codec not read yet (named), or unreadable.
    Undecoded(Option<Vec<u8>>),
}

/// The full names of an inline image's abbreviated keys and values.
pub fn expand_inline(d: &Dict) -> Dict {
    let key = |k: &[u8]| -> Vec<u8> {
        match k {
            b"BPC" => b"BitsPerComponent".to_vec(),
            b"CS" => b"ColorSpace".to_vec(),
            b"D" => b"Decode".to_vec(),
            b"DP" => b"DecodeParms".to_vec(),
            b"F" => b"Filter".to_vec(),
            b"H" => b"Height".to_vec(),
            b"IM" => b"ImageMask".to_vec(),
            b"I" => b"Interpolate".to_vec(),
            b"W" => b"Width".to_vec(),
            other => other.to_vec(),
        }
    };
    let value = |v: &Object| -> Object {
        match v {
            Object::Name(n) => Object::Name(match n.as_slice() {
                b"G" => b"DeviceGray".to_vec(),
                b"RGB" => b"DeviceRGB".to_vec(),
                b"CMYK" => b"DeviceCMYK".to_vec(),
                b"I" => b"Indexed".to_vec(),
                b"AHx" => b"ASCIIHexDecode".to_vec(),
                b"A85" => b"ASCII85Decode".to_vec(),
                b"LZW" => b"LZWDecode".to_vec(),
                b"Fl" => b"FlateDecode".to_vec(),
                b"RL" => b"RunLengthDecode".to_vec(),
                b"CCF" => b"CCITTFaxDecode".to_vec(),
                b"DCT" => b"DCTDecode".to_vec(),
                other => other.to_vec(),
            }),
            Object::Array(a) => Object::Array(a.iter().map(|x| if let Object::Name(_) = x { value_name(x) } else { x.clone() }).collect()),
            other => other.clone(),
        }
    };
    Dict(d.0.iter().map(|(k, v)| (key(k), value(v))).collect())
}

fn value_name(x: &Object) -> Object {
    match x {
        Object::Name(n) if n == b"I" => Object::Name(b"Indexed".to_vec()),
        Object::Name(n) if n == b"RGB" => Object::Name(b"DeviceRGB".to_vec()),
        Object::Name(n) if n == b"G" => Object::Name(b"DeviceGray".to_vec()),
        Object::Name(n) if n == b"CMYK" => Object::Name(b"DeviceCMYK".to_vec()),
        Object::Name(n) if n == b"Fl" => Object::Name(b"FlateDecode".to_vec()),
        Object::Name(n) if n == b"DCT" => Object::Name(b"DCTDecode".to_vec()),
        Object::Name(n) if n == b"A85" => Object::Name(b"ASCII85Decode".to_vec()),
        other => other.clone(),
    }
}

/// An image from its dictionary and its bytes after the non-picture
/// filters (`data`; `image_filter` names a picture codec left to apply).
pub fn decode(doc: &Document, dict: &Dict, data: &[u8], image_filter: Option<&[u8]>, parms: Option<&Dict>, resources: Option<&Dict>) -> Decoded {
    decode_at(doc, dict, data, image_filter, parms, resources, 0)
}

/// `depth` 1 for a mask: a mask's own masks are never followed (a file
/// could make one its own).
fn decode_at(doc: &Document, dict: &Dict, data: &[u8], image_filter: Option<&[u8]>, parms: Option<&Dict>, resources: Option<&Dict>, depth: usize) -> Decoded {
    if image_filter == Some(b"DCTDecode") {
        return jpeg(doc, dict, data, resources, depth);
    }
    if image_filter == Some(b"CCITTFaxDecode") {
        return fax(doc, dict, data, parms, resources, depth);
    }
    if image_filter == Some(b"JBIG2Decode") {
        return jbig2(doc, dict, data, parms, resources, depth);
    }
    if let Some(f) = image_filter {
        return Decoded::Undecoded(Some(f.to_vec()));
    }
    let num = |k: &str| doc.get_in(dict, k).as_int();
    let (Some(w), Some(h)) = (num("Width"), num("Height")) else { return Decoded::Undecoded(None) };
    if w <= 0 || h <= 0 || (w as usize).saturating_mul(h as usize) > MAX_PIXELS {
        return Decoded::Undecoded(None);
    }
    let (w, h) = (w as usize, h as usize);
    let stencil = matches!(doc.get_in(dict, "ImageMask"), Object::Bool(true));
    let decode_arr: Vec<f64> = match doc.get_in(dict, "Decode") {
        Object::Array(a) => a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect(),
        _ => Vec::new(),
    };
    if stencil {
        // One bit per sample; by default a 0 bit paints.
        let paint_on = if decode_arr.first().copied().unwrap_or(0.0) == 1.0 { 1 } else { 0 };
        let mut bits = Bits::new(data);
        let mut alpha = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let b = bits.read(1).unwrap_or(1 - paint_on);
                alpha[y * w + x] = if b == paint_on { 255 } else { 0 };
            }
            bits.align();
        }
        return Decoded::Stencil { width: w, height: h, alpha };
    }
    let cs = ColorSpace::parse(&doc.get_in(dict, "ColorSpace"), doc, resources).unwrap_or(ColorSpace::Gray);
    let bpc = num("BitsPerComponent").unwrap_or(8);
    if ![1, 2, 4, 8, 16].contains(&bpc) {
        return Decoded::Undecoded(None);
    }
    let n = cs.components().max(1);
    let max = ((1u32 << bpc) - 1) as f64;
    // The default decode: 0..1 for each component, 0..2^bpc-1 for Indexed.
    let range = |i: usize| -> (f64, f64) {
        match (decode_arr.get(2 * i), decode_arr.get(2 * i + 1)) {
            (Some(a), Some(b)) => (*a, *b),
            _ if matches!(cs, ColorSpace::Indexed { .. }) => (0.0, max),
            _ => (0.0, 1.0),
        }
    };
    let ranges: Vec<(f64, f64)> = (0..n).map(range).collect();
    // A color-key mask: samples within these raw ranges are transparent.
    let key: Option<Vec<u64>> = match doc.get_in(dict, "Mask") {
        Object::Array(a) => Some(a.iter().filter_map(|x| doc.resolve(x).as_int()).map(|v| v.max(0) as u64).collect()),
        _ => None,
    };
    let mut out = vec![0u8; w * h * 4];
    let mut bits = Bits::new(data);
    let mut raw = vec![0u64; n];
    let mut comps = vec![0f64; n];
    // A cache for 8-bit single-component images (most gray and indexed ones).
    let mut cache: Vec<Option<[u8; 3]>> = if n == 1 && bpc <= 8 { vec![None; 256] } else { Vec::new() };
    // The common case, 8 bits a sample with the default decode and no color
    // key, a byte at a time (step 9: a 36M-pixel RGB photo and its soft mask
    // took 3 s bit by bit). Rows of 8-bit samples are byte-aligned, so the
    // samples are simply consecutive; missing ones read as 0, as below.
    // One-component images of 1, 2 or 4 bits too: each row starts on a
    // byte, so a sample is a shift and a mask (a 33M-pixel fax page).
    let fast = key.is_none() && decode_arr.is_empty() && ((n == 1 && bpc <= 8) || (bpc == 8 && matches!(cs, ColorSpace::Rgb | ColorSpace::Cmyk)));
    let sample = |i: usize| data.get(i).copied().unwrap_or(0);
    let (bpc_u, stride) = (bpc as usize, (w * bpc as usize).div_ceil(8));
    for y in 0..if fast { h } else { 0 } {
        for x in 0..w {
            let (p, o) = ((y * w + x) * n, (y * w + x) * 4);
            let rgb = match cs {
                _ if n == 1 => {
                    let bit = x * bpc_u;
                    let r = (sample(y * stride + bit / 8) >> (8 - bpc_u - bit % 8)) & (max as u8);
                    comps[0] = range(0).0 + r as f64 * (range(0).1 - range(0).0) / max;
                    *cache[r as usize].get_or_insert_with(|| to_bytes(cs.to_rgb(&comps).unwrap_or([1.0; 3])))
                }
                ColorSpace::Rgb => [sample(p), sample(p + 1), sample(p + 2)],
                ColorSpace::Cmyk => {
                    // The same arithmetic as the general path's, so the same bytes.
                    let v = |i: usize| (sample(i) as f64 / max) as f32;
                    let k = v(p + 3);
                    to_bytes([0, 1, 2].map(|c| (1.0 - v(p + c)) * (1.0 - k)))
                }
                // Not reached: the fast path takes one component, RGB or CMYK.
                _ => [255; 3],
            };
            out[o..o + 3].copy_from_slice(&rgb);
            out[o + 3] = 255;
        }
    }
    // Colors already worked out, for 8-bit images of 2 to 4 components in a
    // space that costs a function or a formula a pixel (step 9: a DeviceN
    // cover ran its PostScript function for every pixel, 2 s a page).
    let memo_on = bpc == 8 && (2..=4).contains(&n) && matches!(cs, ColorSpace::Tint { .. } | ColorSpace::Lab { .. });
    let mut memo: std::collections::HashMap<u32, [u8; 3]> = std::collections::HashMap::new();
    for y in 0..if fast { 0 } else { h } {
        for x in 0..w {
            for (k, r) in raw.iter_mut().enumerate() {
                *r = bits.read(bpc as u32).unwrap_or(0);
                let (d0, d1) = ranges[k];
                comps[k] = d0 + *r as f64 * (d1 - d0) / max;
            }
            let keyed = key.as_ref().is_some_and(|k| k.len() >= 2 * n && raw.iter().enumerate().all(|(i, v)| *v >= k[2 * i] && *v <= k[2 * i + 1]));
            let rgb = if !cache.is_empty() {
                let slot = raw[0] as usize;
                *cache[slot].get_or_insert_with(|| to_bytes(cs.to_rgb(&comps).unwrap_or([1.0; 3])))
            } else if memo_on {
                let k = raw.iter().fold(0u32, |a, v| a << 8 | *v as u32);
                match memo.get(&k) {
                    Some(c) => *c,
                    None => {
                        let c = to_bytes(cs.to_rgb(&comps).unwrap_or([1.0; 3]));
                        if memo.len() < MEMO_MAX {
                            memo.insert(k, c);
                        }
                        c
                    }
                }
            } else {
                to_bytes(cs.to_rgb(&comps).unwrap_or([1.0; 3]))
            };
            let i = (y * w + x) * 4;
            if !keyed {
                out[i..i + 3].copy_from_slice(&rgb);
                out[i + 3] = 255;
            }
        }
        bits.align();
    }
    // A soft mask or an explicit (stencil) mask: alpha from another image.
    let smask = doc.get_in(dict, "SMask");
    let mask = if depth > 0 {
        None
    } else {
        match &smask {
        Object::Stream(_) => Some((smask.clone(), true)),
            _ => match doc.get_in(dict, "Mask") {
                m @ Object::Stream(_) => Some((m, false)),
                _ => None,
            },
        }
    };
    if let Some((Object::Stream(s), soft)) = mask {
        let d = doc.stream(&s);
        let alpha = match decode_at(doc, &s.dict, &d.data, d.image.as_ref().map(|(n, _)| n.as_slice()), d.image.as_ref().and_then(|(_, p)| p.as_ref()), resources, depth + 1) {
            Decoded::Color(img) if soft => Some((img.width, img.height, img.data.as_chunks::<4>().0.iter().map(|p| p[0]).collect::<Vec<u8>>())),
            Decoded::Stencil { width, height, alpha } => Some((width, height, alpha)),
            _ => None,
        };
        if let Some((mw, mh, a)) = alpha {
            // Resampled to the image's size if it differs.
            for y in 0..h {
                for x in 0..w {
                    let (sx, sy) = (x * mw / w, y * mh / h);
                    let i = (y * w + x) * 4;
                    out[i + 3] = ((out[i + 3] as u32 * a[sy * mw + sx] as u32) / 255) as u8;
                }
            }
        }
    }
    // Premultiplied.
    for p in out.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    Decoded::Color(Image { width: w, height: h, data: out })
}

/// A JPEG: its samples, then the usual path with the dictionary adjusted
/// to them (8 bits; the PDF's color space kept when its component count
/// matches, else gray, RGB or CMYK by the JPEG's own count; CMYK as
/// stored, the /Decode array applied as for any image).
fn jpeg(doc: &Document, dict: &Dict, data: &[u8], resources: Option<&Dict>, depth: usize) -> Decoded {
    let Ok((w, h, channels, samples)) = vergence_decode::jpeg_samples(data) else { return Decoded::Undecoded(Some(b"DCTDecode".to_vec())) };
    let cs = ColorSpace::parse(&doc.get_in(dict, "ColorSpace"), doc, resources);
    let keep = cs.as_ref().is_some_and(|c| c.components() == channels as usize);
    let mut d: Dict = Dict(dict.0.iter().filter(|(k, _)| !matches!(k.as_slice(), b"Width" | b"Height" | b"BitsPerComponent" | b"Filter" | b"DecodeParms")).cloned().collect());
    if !keep {
        d.0.retain(|(k, _)| k != b"ColorSpace" && k != b"Decode");
        let name: &[u8] = match channels {
            1 => b"DeviceGray",
            4 => b"DeviceCMYK",
            _ => b"DeviceRGB",
        };
        d.0.push((b"ColorSpace".to_vec(), Object::Name(name.to_vec())));
    }
    d.0.push((b"Width".to_vec(), Object::Int(w as i64)));
    d.0.push((b"Height".to_vec(), Object::Int(h as i64)));
    d.0.push((b"BitsPerComponent".to_vec(), Object::Int(8)));
    decode_at(doc, &d, &samples, None, None, resources, depth)
}

/// JBIG2: the page decoded (with /JBIG2Globals), turned into 1-bit rows
/// with 0 black, as PDF's samples are, sized to the image's own Width and
/// Height.
fn jbig2(doc: &Document, dict: &Dict, data: &[u8], parms: Option<&Dict>, resources: Option<&Dict>, depth: usize) -> Decoded {
    let globals = parms.and_then(|p| match doc.get_in(p, "JBIG2Globals") {
        Object::Stream(s) => Some(doc.stream(&s).data),
        _ => None,
    });
    let Some(page) = crate::jbig2::decode(globals.as_deref(), data) else { return Decoded::Undecoded(Some(b"JBIG2Decode".to_vec())) };
    let w = doc.get_in(dict, "Width").as_int().filter(|w| *w > 0).map_or(page.w, |w| w as usize).min(1 << 16);
    let h = doc.get_in(dict, "Height").as_int().filter(|h| *h > 0).map_or(page.h, |h| h as usize).min(1 << 16);
    let stride = w.div_ceil(8);
    let mut samples = vec![0xFFu8; stride * h];
    for y in 0..h.min(page.h) {
        for x in 0..w.min(page.w) {
            if page.px[y * page.w + x] == 1 {
                samples[y * stride + x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }
    let mut d: Dict = Dict(dict.0.iter().filter(|(k, _)| !matches!(k.as_slice(), b"Width" | b"Height" | b"BitsPerComponent" | b"Filter" | b"DecodeParms")).cloned().collect());
    d.0.push((b"Width".to_vec(), Object::Int(w as i64)));
    d.0.push((b"Height".to_vec(), Object::Int(h as i64)));
    d.0.push((b"BitsPerComponent".to_vec(), Object::Int(1)));
    decode_at(doc, &d, &samples, None, None, resources, depth)
}

/// CCITT fax: decoded to 1-bit rows (a 0 bit black unless BlackIs1), then
/// the usual path at 1 bit per component.
fn fax(doc: &Document, dict: &Dict, data: &[u8], parms: Option<&Dict>, resources: Option<&Dict>, depth: usize) -> Decoded {
    let empty = Dict::default();
    let p = parms.unwrap_or(&empty);
    let int = |k: &str| doc.get_in(p, k).as_int();
    let flag = |k: &str, default: bool| match doc.get_in(p, k) {
        Object::Bool(b) => b,
        _ => default,
    };
    let rows = int("Rows").or_else(|| doc.get_in(dict, "Height").as_int()).unwrap_or(0).max(0) as usize;
    let params = crate::ccitt::Params {
        k: int("K").unwrap_or(0),
        columns: int("Columns").unwrap_or(1728).max(1) as usize,
        rows,
        end_of_line: flag("EndOfLine", false),
        byte_align: flag("EncodedByteAlign", false),
        end_of_block: flag("EndOfBlock", true),
        black_is_1: flag("BlackIs1", false),
    };
    let samples = crate::ccitt::decode(data, &params);
    let height = samples.len() / params.columns.div_ceil(8).max(1);
    if height == 0 {
        return Decoded::Undecoded(Some(b"CCITTFaxDecode".to_vec()));
    }
    let mut d: Dict = Dict(dict.0.iter().filter(|(k, _)| !matches!(k.as_slice(), b"Width" | b"Height" | b"BitsPerComponent" | b"Filter" | b"DecodeParms")).cloned().collect());
    d.0.push((b"Width".to_vec(), Object::Int(params.columns as i64)));
    d.0.push((b"Height".to_vec(), Object::Int(height as i64)));
    d.0.push((b"BitsPerComponent".to_vec(), Object::Int(1)));
    decode_at(doc, &d, &samples, None, None, resources, depth)
}

/// The most colors remembered for one image.
const MEMO_MAX: usize = 1 << 20;

fn to_bytes(c: [f32; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// An image XObject's stream, decoded.
pub fn from_stream(doc: &Document, s: &crate::object::Stream, resources: Option<&Dict>) -> Decoded {
    let d = doc.stream(s);
    decode(doc, &s.dict, &d.data, d.image.as_ref().map(|(n, _)| n.as_slice()), d.image.as_ref().and_then(|(_, p)| p.as_ref()), resources)
}

/// An inline image's dictionary and data.
pub fn inline(doc: &Document, dict: &Dict, data: &[u8], resources: Option<&Dict>) -> Decoded {
    let dict = expand_inline(dict);
    let (names, parms) = filter::filters_of(&dict);
    let d = filter::decode(data, &names, &parms);
    decode(doc, &dict, &d.data, d.image.as_ref().map(|(n, _)| n.as_slice()), d.image.as_ref().and_then(|(_, p)| p.as_ref()), resources)
}
