//! Color spaces (ISO 32000-2 §8.6), each to sRGB. Without color
//! management (an ICC profile is taken by its number of components, CMYK
//! by the simple formula): what most viewers do on screen, and the PDF
//! milestone's plan; profiles can come later.

use crate::Document;
use crate::function::Function;
use crate::object::{Dict, Object};

const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone)]
pub enum ColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Lab {
        white: [f64; 3],
        range: [f64; 4],
    },
    Indexed {
        base: Box<ColorSpace>,
        hival: usize,
        lookup: Vec<u8>,
    },
    /// Separation (one colorant) and DeviceN (several), through their tint
    /// function into the alternate space. `none`: the colorant /None, which
    /// paints nothing.
    Tint {
        n: usize,
        alternate: Box<ColorSpace>,
        function: Option<Function>,
        none: bool,
    },
    /// Pattern: what `scn` names is a pattern; for an uncolored one, the
    /// space of the color it is painted in.
    Pattern(Option<Box<ColorSpace>>),
}

impl ColorSpace {
    /// From a name or array, looking names up in `resources`' /ColorSpace.
    pub fn parse(o: &Object, doc: &Document, resources: Option<&Dict>) -> Option<ColorSpace> {
        Self::parse_at(o, doc, resources, 0)
    }

    fn parse_at(o: &Object, doc: &Document, resources: Option<&Dict>, depth: usize) -> Option<ColorSpace> {
        if depth > MAX_DEPTH {
            return None;
        }
        let o = doc.resolve(o);
        if let Object::Name(n) = &o {
            match n.as_slice() {
                b"DeviceGray" | b"G" | b"CalGray" => return Some(ColorSpace::Gray),
                b"DeviceRGB" | b"RGB" | b"CalRGB" => return Some(ColorSpace::Rgb),
                b"DeviceCMYK" | b"CMYK" => return Some(ColorSpace::Cmyk),
                b"Pattern" => return Some(ColorSpace::Pattern(None)),
                b"I" | b"Indexed" => return None,
                name => {
                    // A resource name: look it up.
                    let spaces = resources.map(|r| doc.get_in(r, "ColorSpace"))?;
                    let found = spaces.as_dict()?.get(std::str::from_utf8(name).ok()?)?.clone();
                    return Self::parse_at(&found, doc, resources, depth + 1);
                }
            }
        }
        let a = o.as_array()?;
        let kind = doc.resolve(a.first()?);
        let arg = |i: usize| a.get(i).map(|x| doc.resolve(x)).unwrap_or(Object::Null);
        match kind.as_name()? {
            b"DeviceGray" | b"CalGray" | b"G" => Some(ColorSpace::Gray),
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(ColorSpace::Rgb),
            b"DeviceCMYK" | b"CMYK" => Some(ColorSpace::Cmyk),
            b"ICCBased" => {
                let d = arg(1);
                let dict = d.as_dict()?;
                if let Some(alt) = dict.get("Alternate")
                    && let Some(cs) = Self::parse_at(alt, doc, resources, depth + 1)
                    && !matches!(cs, ColorSpace::Indexed { .. } | ColorSpace::Pattern(_))
                {
                    return Some(cs);
                }
                match dict.int("N") {
                    Some(1) => Some(ColorSpace::Gray),
                    Some(4) => Some(ColorSpace::Cmyk),
                    _ => Some(ColorSpace::Rgb),
                }
            }
            b"Lab" => {
                let d = arg(1);
                let num = |k: &str, n: usize, default: &[f64]| -> Vec<f64> {
                    let v: Vec<f64> = d
                        .as_dict()
                        .and_then(|d| d.get(k))
                        .and_then(|o| {
                            doc.resolve(o)
                                .as_array()
                                .map(|a| a.iter().filter_map(|x| doc.resolve(x).as_f64()).collect())
                        })
                        .unwrap_or_default();
                    if v.len() >= n { v } else { default.to_vec() }
                };
                let w = num("WhitePoint", 3, &[0.9505, 1.0, 1.089]);
                let r = num("Range", 4, &[-100.0, 100.0, -100.0, 100.0]);
                Some(ColorSpace::Lab { white: [w[0], w[1], w[2]], range: [r[0], r[1], r[2], r[3]] })
            }
            b"Indexed" | b"I" => {
                let base = Self::parse_at(&arg(1), doc, resources, depth + 1)?;
                let hival = arg(2).as_int()?.clamp(0, 255) as usize;
                let lookup = match arg(3) {
                    Object::String(s) => s,
                    Object::Stream(s) => doc.stream(&s).data,
                    _ => return None,
                };
                Some(ColorSpace::Indexed { base: Box::new(base), hival, lookup })
            }
            b"Separation" => {
                let none = arg(1).as_name() == Some(b"None");
                let alternate = Self::parse_at(&arg(2), doc, resources, depth + 1).unwrap_or(ColorSpace::Gray);
                Some(ColorSpace::Tint {
                    n: 1,
                    alternate: Box::new(alternate),
                    function: Function::parse(&arg(3), doc),
                    none,
                })
            }
            b"DeviceN" => {
                let names = arg(1);
                let n = names.as_array().map_or(1, |a| a.len()).clamp(1, 32);
                let none =
                    names.as_array().is_some_and(|a| a.iter().all(|x| doc.resolve(x).as_name() == Some(b"None")));
                let alternate = Self::parse_at(&arg(2), doc, resources, depth + 1).unwrap_or(ColorSpace::Gray);
                Some(ColorSpace::Tint {
                    n,
                    alternate: Box::new(alternate),
                    function: Function::parse(&arg(3), doc),
                    none,
                })
            }
            b"Pattern" => Some(ColorSpace::Pattern(
                a.get(1).and_then(|b| Self::parse_at(b, doc, resources, depth + 1)).map(Box::new),
            )),
            _ => None,
        }
    }

    pub fn components(&self) -> usize {
        match self {
            ColorSpace::Gray | ColorSpace::Indexed { .. } => 1,
            ColorSpace::Rgb | ColorSpace::Lab { .. } => 3,
            ColorSpace::Cmyk => 4,
            ColorSpace::Tint { n, .. } => *n,
            ColorSpace::Pattern(base) => base.as_ref().map_or(0, |b| b.components()),
        }
    }

    /// The color a space starts at when it is chosen.
    pub fn initial(&self) -> Vec<f64> {
        match self {
            ColorSpace::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            ColorSpace::Tint { n, .. } => vec![1.0; *n],
            ColorSpace::Lab { .. } => vec![0.0, 0.0, 0.0],
            other => vec![0.0; other.components().max(1)],
        }
    }

    /// The color as sRGB, 0 to 1; None for a /None colorant (paint nothing).
    pub fn to_rgb(&self, c: &[f64]) -> Option<[f32; 3]> {
        self.rgb_at(c, 0)
    }

    fn rgb_at(&self, c: &[f64], depth: usize) -> Option<[f32; 3]> {
        if depth > MAX_DEPTH {
            return Some([0.0; 3]);
        }
        let v = |i: usize| c.get(i).copied().filter(|x| x.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        Some(match self {
            ColorSpace::Gray => [v(0); 3],
            ColorSpace::Rgb => [v(0), v(1), v(2)],
            ColorSpace::Cmyk => {
                let k = v(3);
                [(1.0 - v(0)) * (1.0 - k), (1.0 - v(1)) * (1.0 - k), (1.0 - v(2)) * (1.0 - k)]
            }
            ColorSpace::Lab { white, range } => lab_to_rgb(c, white, range),
            ColorSpace::Indexed { base, hival, lookup } => {
                let i = (c.first().copied().unwrap_or(0.0).round().max(0.0) as usize).min(*hival);
                let n = base.components();
                let entry: Vec<f64> =
                    (0..n).map(|k| lookup.get(i * n + k).copied().unwrap_or(0) as f64 / 255.0).collect();
                // Lab lookups hold bytes over the space's ranges.
                let entry = match base.as_ref() {
                    ColorSpace::Lab { range, .. } => vec![
                        entry[0] * 100.0,
                        range[0] + entry[1] * (range[1] - range[0]),
                        range[2] + entry[2] * (range[3] - range[2]),
                    ],
                    _ => entry,
                };
                return base.rgb_at(&entry, depth + 1);
            }
            ColorSpace::Tint { alternate, function, none, n } => {
                if *none {
                    return None;
                }
                let input: Vec<f64> = (0..*n).map(|i| c.get(i).copied().unwrap_or(1.0)).collect();
                let out = match function {
                    Some(f) => f.eval(&input),
                    // No usable tint transform: the tint as darkness in gray.
                    None => return Some([1.0 - v(0); 3]),
                };
                return alternate.rgb_at(&out, depth + 1);
            }
            ColorSpace::Pattern(base) => return base.as_ref().map_or(Some([0.0; 3]), |b| b.rgb_at(c, depth + 1)),
        })
    }
}

fn lab_to_rgb(c: &[f64], white: &[f64; 3], range: &[f64; 4]) -> [f32; 3] {
    let l = c.first().copied().unwrap_or(0.0).clamp(0.0, 100.0);
    let a = c.get(1).copied().unwrap_or(0.0).clamp(range[0], range[1]);
    let b = c.get(2).copied().unwrap_or(0.0).clamp(range[2], range[3]);
    let g = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 108.0 / 841.0 * (t - 4.0 / 29.0) };
    let m = (l + 16.0) / 116.0;
    let (x, y, z) = (white[0] * g(m + a / 500.0), white[1] * g(m), white[2] * g(m - b / 200.0));
    // XYZ (D50-ish white taken as given) to linear sRGB, then gamma.
    let lin = [
        3.2406 * x - 1.5372 * y - 0.4986 * z,
        -0.9689 * x + 1.8758 * y + 0.0415 * z,
        0.0557 * x - 0.2040 * y + 1.0570 * z,
    ];
    lin.map(|v| {
        let v = v.clamp(0.0, 1.0);
        (if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }) as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_spaces() {
        assert_eq!(ColorSpace::Gray.to_rgb(&[0.5]), Some([0.5; 3]));
        assert_eq!(ColorSpace::Cmyk.to_rgb(&[0.0, 0.0, 0.0, 1.0]), Some([0.0; 3]));
        assert_eq!(ColorSpace::Cmyk.to_rgb(&[1.0, 0.0, 0.0, 0.0]), Some([0.0, 1.0, 1.0]));
        assert_eq!(ColorSpace::Cmyk.initial(), vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn indexed_and_lab() {
        let cs = ColorSpace::Indexed { base: Box::new(ColorSpace::Rgb), hival: 1, lookup: vec![255, 0, 0, 0, 0, 255] };
        assert_eq!(cs.to_rgb(&[1.0]), Some([0.0, 0.0, 1.0]));
        assert_eq!(cs.to_rgb(&[9.0]), Some([0.0, 0.0, 1.0]), "clamped to hival");
        let lab = ColorSpace::Lab { white: [0.9505, 1.0, 1.089], range: [-100.0, 100.0, -100.0, 100.0] };
        let w = lab.to_rgb(&[100.0, 0.0, 0.0]).unwrap();
        assert!(w.iter().all(|v| *v > 0.98), "{w:?}");
    }

    #[test]
    fn a_none_colorant_paints_nothing() {
        let cs = ColorSpace::Tint { n: 1, alternate: Box::new(ColorSpace::Gray), function: None, none: true };
        assert_eq!(cs.to_rgb(&[1.0]), None);
    }
}
