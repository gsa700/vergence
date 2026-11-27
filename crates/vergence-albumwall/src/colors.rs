//! AlbumWall's colors, taken from the covers (ported from the Avalonia app's
//! Contrast.cs, Palette.cs and Ground.cs; docs/albumwall-milestone-1.md,
//! step 3).
//!
//! - **The contrast floor**: text never under 4.5:1 (WCAG 2) against what
//!   is behind it. Every color stays as designed where it already reads;
//!   one that falls short is moved in lightness, away from its ground,
//!   keeping its hue, just far enough (measured 2026-10-21: only the
//!   weakest cases change, the look stays).
//! - **The wall's ground** takes its HUE from the whole library's covers (a
//!   circular mean weighted by saturation: hue wraps, and a near-black
//!   pixel's hue means nothing), at a low saturation (Tint, 2%) and a
//!   lightness the person sets (Light, 10 to 40, 21 by default: the ceiling
//!   is where the body text stops clearing 4.5:1). Library-wide, never per
//!   album: a ground that changed as tiles scrolled past would become what
//!   you watch. The bars are a different material (Chrome: recede, lift,
//!   ink, warm; ink by default).
//! - **An album's palette**, for its panel and the play bar, from one
//!   cover: its hue and saturation (the mean pixel of a cover is mud), a
//!   light tone from the cover's brightest quarter for the play bar; the
//!   lightnesses are assigned, not taken, so every album's text reads.

/// A color, 8-bit sRGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(s: &str) -> Rgb {
        let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}

/// WCAG 2's floor for ordinary text.
pub const TEXT: f64 = 4.5;

fn lin(v: u8) -> f64 {
    let s = v as f64 / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// Relative luminance (WCAG 2).
pub fn luminance(c: Rgb) -> f64 {
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// The contrast ratio of two colors, 1 to 21.
pub fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Hue (degrees), saturation and lightness (0 to 1) of an sRGB color.
pub fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let mut h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    (h, s, l)
}

pub fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(byte(r + m), byte(g + m), byte(b + m))
}

/// `fg` as it is if it reads against `bg` (at least `min`); otherwise moved
/// in lightness, away from the ground, keeping its hue and saturation, until
/// it does, or as far as it goes (white or black).
pub fn floor(fg: Rgb, bg: Rgb, min: f64) -> Rgb {
    if ratio(fg, bg) >= min {
        return fg;
    }
    let (h, s, l0) = to_hsl(fg);
    let up = luminance(fg) >= luminance(bg);
    let mut l = l0;
    while (0.0..=1.0).contains(&l) {
        let c = from_hsl(h, s, l);
        if ratio(c, bg) >= min {
            return c;
        }
        l += if up { 0.01 } else { -0.01 };
    }
    if up { Rgb(255, 255, 255) } else { Rgb(0, 0, 0) }
}

/// The lightest a ground of this hue and saturation may be while `text`
/// still reads on it.
pub fn max_ground_lightness(text: Rgb, hue: f64, sat: f64, min: f64) -> f64 {
    let mut l = 1.0;
    while l > 0.0 {
        if ratio(text, from_hsl(hue, sat, l)) >= min {
            return l;
        }
        l -= 0.005;
    }
    0.0
}

/// A cover shrunk to a few pixels (16 wide), straight RGBA: what the colors
/// are taken from (the shrinking is itself the averaging).
pub struct Sample<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

impl Sample<'_> {
    fn pixels(&self) -> impl Iterator<Item = (usize, usize, Rgb)> + '_ {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .take(self.width * self.height)
            .enumerate()
            .map(|(i, p)| (i % self.width.max(1), i / self.width.max(1), Rgb(p[0], p[1], p[2])))
    }
}

/// A pixel's weight toward a hue: its saturation, less at the hue-blind
/// ends of lightness (near black or white a hue means nothing).
fn hue_weight(s: f64, l: f64) -> f64 {
    s * (1.0 - (2.0 * l - 1.0).abs())
}

/// Sums toward a circular mean of hue: Σ w·cos, Σ w·sin, Σ w, Σ s·w.
#[derive(Debug, Clone, Copy, Default)]
pub struct HueSum {
    x: f64,
    y: f64,
    w: f64,
    s: f64,
}

impl HueSum {
    pub fn add(&mut self, c: Rgb) {
        let (h, s, l) = to_hsl(c);
        let w = hue_weight(s, l);
        if w <= 0.0 {
            return;
        }
        let a = h.to_radians();
        self.x += w * a.cos();
        self.y += w * a.sin();
        self.w += w;
        self.s += s * w;
    }

    pub fn merge(&mut self, o: HueSum) {
        self.x += o.x;
        self.y += o.y;
        self.w += o.w;
        self.s += o.s;
    }

    /// The mean hue, and how much the pixels agree on it (0 nothing, 1 all one hue).
    pub fn hue(&self) -> Option<(f64, f64)> {
        (self.w > 0.0).then(|| (self.y.atan2(self.x).to_degrees().rem_euclid(360.0), self.x.hypot(self.y) / self.w))
    }

    /// The weighted mean saturation: whether there is a color at all.
    pub fn saturation(&self) -> f64 {
        if self.w > 0.0 { self.s / self.w } else { 0.0 }
    }
}

/// The ring the keyboard draws round a cover: light, or dark where dark
/// reads against more of the cover's edge (AlbumWall's KeyRing, 3:1).
pub const RING_LIGHT: Rgb = Rgb(0xF2, 0xED, 0xE4);
pub const RING_DARK: Rgb = Rgb(0x1A, 0x16, 0x12);
const RING_FLOOR: f64 = 3.0;

/// What one cover gives: its hue and saturation, its highlight's lightness,
/// and how much of its outer edge each ring color reads against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    pub hue: f64,
    pub saturation: f64,
    pub highlight: f64,
    pub edge_light: f64,
    pub edge_dark: f64,
}

impl Default for Analysis {
    /// No usable cover.
    fn default() -> Analysis {
        Analysis { hue: 0.0, saturation: 0.0, highlight: 0.74, edge_light: 1.0, edge_dark: 0.0 }
    }
}

/// One cover's colors (Palette.Analyze).
pub fn analyze(sample: &Sample) -> Analysis {
    let mut sum = HueSum::default();
    let mut lightnesses = Vec::with_capacity(sample.width * sample.height);
    let (mut edge, mut light, mut dark) = (0usize, 0usize, 0usize);
    for (x, y, c) in sample.pixels() {
        lightnesses.push(to_hsl(c).2);
        if x == 0 || y == 0 || x + 1 == sample.width || y + 1 == sample.height {
            edge += 1;
            light += (ratio(RING_LIGHT, c) >= RING_FLOOR) as usize;
            dark += (ratio(RING_DARK, c) >= RING_FLOOR) as usize;
        }
        sum.add(c);
    }
    let (edge_light, edge_dark) =
        if edge > 0 { (light as f64 / edge as f64, dark as f64 / edge as f64) } else { (1.0, 0.0) };
    let highlight = highlight(&mut lightnesses);
    let (hue, _) = sum.hue().unwrap_or((0.0, 0.0));
    Analysis { hue, saturation: sum.saturation(), highlight, edge_light, edge_dark }
}

/// The mean lightness of the brightest quarter (not the maximum: one blown
/// white pixel is not a color the album is made of).
fn highlight(l: &mut [f64]) -> f64 {
    if l.is_empty() {
        return 0.74;
    }
    l.sort_by(f64::total_cmp);
    let from = l.len() * 3 / 4;
    l[from..].iter().sum::<f64>() / (l.len() - from) as f64
}

/// An album's colors: its panel and the play bar (AlbumPalette).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub hue: f64,
    pub saturation: f64,
    /// The panel's ground (it fades to nothing at its foot).
    pub panel: Rgb,
    /// The track list on the panel.
    pub surface: Rgb,
    /// Disc headings, the selection ring, what is active.
    pub accent: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    /// A light tone from the cover's bright end: the play bar's ground.
    pub light: Rgb,
    pub on_light: Rgb,
    pub on_light_dim: Rgb,
    pub on_light_hover: Rgb,
    pub edge_light: f64,
    pub edge_dark: f64,
}

/// How far a panel rides with the wall's Light (0.8: a little darker than
/// the wall it interrupts, so it reads as a recess, not a sheet on top).
const RIDE: f64 = 0.8;

impl Palette {
    /// The palette of a cover's analysis, with the wall at `light` percent.
    pub fn of(a: &Analysis, light: u32) -> Palette {
        let lift = (light as f64 - DEFAULT_LIGHT as f64) / 100.0 * RIDE;
        let mut p = if a.saturation <= 0.02 {
            Palette::build(28.0, 0.10, 0.74, lift)
        } else {
            Palette::build(a.hue, a.saturation, a.highlight, lift)
        };
        p.edge_light = a.edge_light;
        p.edge_dark = a.edge_dark;
        p
    }

    /// A warm neutral, for a gray cover or none.
    pub fn neutral(light: u32) -> Palette {
        Palette::of(&Analysis::default(), light)
    }

    /// Lightness is assigned here, not taken from the art, so every album's
    /// panel text clears 4.5:1.
    fn build(hue: f64, sat: f64, highlight: f64, lift: f64) -> Palette {
        let lit = |l: f64| (l + lift).clamp(0.05, 0.42);
        // Below about 0.18 a tint reads as a mistake; above 0.45 the panel
        // competes with the cover it describes.
        let s = sat.clamp(0.18, 0.45);
        let ground = from_hsl(hue, s, lit(0.15));
        let light = from_hsl(hue, (s * 0.55).clamp(0.10, 0.26), highlight.clamp(0.62, 0.80));
        let on = |bg: Rgb, h: f64, sat: f64, l: f64| floor(from_hsl(h, sat, l), bg, TEXT);
        Palette {
            hue,
            saturation: s,
            panel: ground,
            surface: from_hsl(hue, s * 0.85, lit(0.20)),
            accent: on(ground, hue, (s * 1.9).clamp(0.40, 0.70), 0.62),
            // Text a few points toward the album's hue: neutral white on a
            // colored panel reads as a foreign layer.
            text: on(ground, hue, 0.08, 0.95),
            text_dim: on(ground, hue, 0.10, 0.68),
            light,
            on_light: on(light, hue, 0.30, 0.13),
            on_light_dim: on(light, hue, 0.22, 0.34),
            on_light_hover: from_hsl(hue, 0.20, 0.52),
            edge_light: 1.0,
            edge_dark: 0.0,
        }
    }
}

/// The wall's ground when the library gives no hue (no covers, or gray ones): warm amber.
pub const FALLBACK_HUE: f64 = 30.0;
pub const MIN_LIGHT: u32 = 10;
pub const MAX_LIGHT: u32 = 40;
/// Where Dave settled, on two very different screens.
pub const DEFAULT_LIGHT: u32 = 21;
pub const DEFAULT_TINT: u32 = 2;
/// Tint's top, in percent saturation (the Avalonia slider's Maximum).
pub const MAX_TINT: u32 = 14;
/// The body text of every dark window, and the dim text as designed.
pub const BODY_TEXT: Rgb = Rgb(0xF3, 0xEE, 0xE6);
pub const DIM_TEXT: Rgb = Rgb(0xA2, 0x97, 0x8A);

/// How the bars set themselves apart from the wall: lightness relative to
/// the ground (points), their saturation, their edge's lightness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    pub name: &'static str,
    pub bar_delta: f64,
    pub bar_sat: f64,
    pub edge_delta: f64,
}

pub const CHROMES: [Chrome; 4] = [
    // The bars sink below the wall and drain of color.
    Chrome { name: "recede", bar_delta: -5.0, bar_sat: 0.05, edge_delta: 11.0 },
    // A neutral panel laid over the wall.
    Chrome { name: "lift", bar_delta: 9.0, bar_sat: 0.05, edge_delta: 20.0 },
    // Near black, fully neutral: the wall floats (Dave's: "Light 21, Tint 2, INK").
    Chrome { name: "ink", bar_delta: -14.0, bar_sat: 0.03, edge_delta: 9.0 },
    // The library's warmth kept in the bars, further apart in lightness.
    Chrome { name: "warm", bar_delta: 11.0, bar_sat: 0.16, edge_delta: 22.0 },
];
pub const DEFAULT_CHROME: usize = 2;

/// The wall's colors from the library's hue and the person's settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ground {
    pub hue: f64,
    pub ground: Rgb,
    pub bar: Rgb,
    pub edge: Rgb,
    /// Fields and buttons on the bars (they separate from the bar, not the wall).
    pub field: Rgb,
    pub field_edge: Rgb,
    /// The α in the wordmark: the library's hue at a strength nothing else uses.
    pub wordmark: Rgb,
    /// The body and dim text, held to the floor against the wall.
    pub text: Rgb,
    pub text_dim: Rgb,
}

impl Ground {
    /// `light` 10 to 40 (percent), `tint` 0 to 14 (percent saturation),
    /// `chrome` one of [`CHROMES`].
    pub fn new(hue: f64, light: u32, tint: u32, chrome: &Chrome) -> Ground {
        let l = light.clamp(MIN_LIGHT, MAX_LIGHT) as f64;
        let sat = tint.min(MAX_TINT) as f64 / 100.0;
        // No lighter than light text can be read on: the field, the lightest
        // thing on the bars, sits five points above the bar.
        let bar_l = ((l + chrome.bar_delta) / 100.0)
            .clamp(0.03, 0.95)
            .min(max_ground_lightness(BODY_TEXT, hue, chrome.bar_sat, TEXT) - 0.05);
        let edge_l = ((l + chrome.edge_delta) / 100.0).clamp(0.03, 0.95);
        let ground = from_hsl(hue, sat, l / 100.0);
        Ground {
            hue,
            ground,
            bar: from_hsl(hue, chrome.bar_sat, bar_l),
            edge: from_hsl(hue, chrome.bar_sat, edge_l),
            field: from_hsl(hue, chrome.bar_sat, (bar_l + 0.05).clamp(0.03, 0.95)),
            field_edge: from_hsl(hue, chrome.bar_sat, (bar_l + 0.11).clamp(0.03, 0.95)),
            wordmark: from_hsl(hue, 0.62, if bar_l < 0.5 { 0.64 } else { 0.36 }),
            text: floor(BODY_TEXT, ground, TEXT),
            text_dim: floor(DIM_TEXT, ground, TEXT),
        }
    }
}

/// The library's hue from its covers' samples (Ground.HueOf): their summed
/// hues, or the fallback when they agree on no color.
pub fn library_hue(sums: &[HueSum]) -> (f64, f64) {
    let mut all = HueSum::default();
    for s in sums {
        all.merge(*s);
    }
    all.hue().unwrap_or((FALLBACK_HUE, 0.0))
}

/// One cover's sample summed toward the library's hue.
pub fn hue_sum(sample: &Sample) -> HueSum {
    let mut s = HueSum::default();
    for (_, _, c) in sample.pixels() {
        s.add(c);
    }
    s
}

/// At most `most` of `n` covers, spread across the library (albums come
/// sorted by artist: a prefix would be a handful of artists).
pub fn spread(n: usize, most: usize) -> Vec<usize> {
    if n <= most {
        return (0..n).collect();
    }
    let step = n as f64 / most as f64;
    (0..most).map(|i| (i as f64 * step) as usize).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_is_wcags() {
        assert!((ratio(Rgb(0, 0, 0), Rgb(255, 255, 255)) - 21.0).abs() < 0.01);
        assert!((ratio(Rgb(119, 119, 119), Rgb(255, 255, 255)) - 4.48).abs() < 0.01);
        assert_eq!(ratio(Rgb(10, 20, 30), Rgb(10, 20, 30)), 1.0);
    }

    #[test]
    fn hsl_round_trips() {
        for c in [Rgb(255, 0, 0), Rgb(18, 140, 200), Rgb(243, 238, 230), Rgb(20, 22, 30)] {
            let (h, s, l) = to_hsl(c);
            let back = from_hsl(h, s, l);
            assert!((back.0 as i32 - c.0 as i32).abs() <= 1 && (back.2 as i32 - c.2 as i32).abs() <= 1, "{c:?}");
        }
    }

    #[test]
    fn the_floor_moves_only_what_falls_short_and_keeps_its_hue() {
        let ground = Rgb(60, 52, 44);
        assert_eq!(floor(BODY_TEXT, ground, TEXT), BODY_TEXT);
        let dim = floor(DIM_TEXT, from_hsl(30.0, 0.02, 0.40), TEXT);
        assert!(ratio(dim, from_hsl(30.0, 0.02, 0.40)) >= TEXT);
        assert!((to_hsl(dim).0 - to_hsl(DIM_TEXT).0).abs() < 8.0);
    }

    #[test]
    fn every_albums_text_reads_on_its_panel_and_bar() {
        for h in (0..360).step_by(15) {
            for (s, hi) in [(0.2, 0.9), (0.9, 0.62), (0.5, 0.75)] {
                for light in [MIN_LIGHT, DEFAULT_LIGHT, MAX_LIGHT] {
                    let a = Analysis { hue: h as f64, saturation: s, highlight: hi, ..Analysis::default() };
                    let p = Palette::of(&a, light);
                    for (fg, bg) in [
                        (p.text, p.panel),
                        (p.text_dim, p.panel),
                        (p.accent, p.panel),
                        (p.on_light, p.light),
                        (p.on_light_dim, p.light),
                    ] {
                        assert!(ratio(fg, bg) >= TEXT - 0.01, "hue {h} sat {s} light {light}: {:.2}", ratio(fg, bg));
                    }
                }
            }
        }
    }

    #[test]
    fn a_gray_cover_gets_the_warm_neutral() {
        let gray = vec![128u8; 16 * 16 * 4];
        let a = analyze(&Sample { width: 16, height: 16, rgba: &gray });
        let (p, n) = (Palette::of(&a, DEFAULT_LIGHT), Palette::neutral(DEFAULT_LIGHT));
        assert_eq!((p.panel, p.text, p.light), (n.panel, n.text, n.light));
        // Mid gray: both rings read against its edge.
        assert_eq!((p.edge_light, p.edge_dark), (1.0, 1.0));
    }

    #[test]
    fn hue_is_a_circular_mean() {
        let mut s = HueSum::default();
        s.add(from_hsl(350.0, 0.8, 0.5));
        s.add(from_hsl(10.0, 0.8, 0.5));
        let (h, agree) = s.hue().unwrap();
        assert!(h < 2.0 || h > 358.0, "{h}");
        assert!(agree > 0.9);
        assert_eq!(library_hue(&[]), (FALLBACK_HUE, 0.0));
    }

    #[test]
    fn the_wall_text_and_bars_read_at_every_light() {
        for light in MIN_LIGHT..=MAX_LIGHT {
            for chrome in &CHROMES {
                let g = Ground::new(30.0, light, DEFAULT_TINT, chrome);
                assert!(ratio(g.text, g.ground) >= TEXT - 0.01);
                assert!(ratio(g.text_dim, g.ground) >= TEXT - 0.01);
                assert!(ratio(BODY_TEXT, g.bar) >= TEXT - 0.01, "{} at {light}", chrome.name);
            }
        }
        // Dave's: Light 21, Tint 2, ink.
        let g = Ground::new(30.0, DEFAULT_LIGHT, DEFAULT_TINT, &CHROMES[DEFAULT_CHROME]);
        assert!(luminance(g.bar) < luminance(g.ground));
    }

    #[test]
    fn covers_are_sampled_across_the_library() {
        assert_eq!(spread(3, 200), [0, 1, 2]);
        let s = spread(1000, 200);
        assert_eq!((s.len(), s[0], s[1], s[199]), (200, 0, 5, 995));
    }
}
