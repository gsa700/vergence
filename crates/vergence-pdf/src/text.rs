//! A page's text layer (PDF milestone 1, step 7): every character shown,
//! with its box, in the order the page draws them (which is reading order
//! in nearly every file), and what is built on it: the text joined with
//! the spaces and line breaks its positions imply, finding, and the boxes
//! of a selection.
//!
//! Characters come from [`crate::render::page_text`]: the page run without
//! painting, each glyph's box through the text and page transforms, its
//! text from the font's ToUnicode map or encoding.

/// One character drawn: its text (a ligature can be several) and its box
/// (x0, y0, x1, y1 in points from the page's top left as shown).
#[derive(Debug, Clone, PartialEq)]
pub struct Char {
    pub text: String,
    pub rect: [f32; 4],
}

impl Char {
    fn height(&self) -> f32 {
        (self.rect[3] - self.rect[1]).max(0.1)
    }

    fn mid_y(&self) -> f32 {
        (self.rect[1] + self.rect[3]) / 2.0
    }

    fn is_space(&self) -> bool {
        self.text.chars().all(char::is_whitespace)
    }
}

/// The most characters kept for one page.
pub const MAX_CHARS: usize = 200_000;

/// Whether `c` starts a new line after `p`.
fn new_line(p: &Char, c: &Char) -> bool {
    let h = p.height().max(c.height());
    (c.mid_y() - p.mid_y()).abs() > h * 0.5
}

/// Characters drawn twice in the same place (a "bold" made by overprinting)
/// are kept once.
pub fn dedupe(chars: Vec<Char>) -> Vec<Char> {
    let mut out: Vec<Char> = Vec::with_capacity(chars.len());
    for c in chars {
        if let Some(p) = out.last()
            && p.text == c.text
            && (0..4).all(|i| (p.rect[i] - c.rect[i]).abs() < p.height() * 0.15)
        {
            continue;
        }
        out.push(c);
    }
    out
}

/// The page's text with spaces and line breaks where the positions imply
/// them, and for each character of it (by `char`) the drawn character it
/// came from (None for those inserted).
pub fn join(chars: &[Char]) -> (String, Vec<Option<usize>>) {
    let mut text = String::new();
    let mut from = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 {
            let p = &chars[i - 1];
            if new_line(p, c) {
                text.push('\n');
                from.push(None);
            } else if !p.is_space() && !c.is_space() && c.rect[0] - p.rect[2] > p.height().max(c.height()) * 0.18 {
                text.push(' ');
                from.push(None);
            }
        }
        for ch in c.text.chars() {
            text.push(ch);
            from.push(Some(i));
        }
    }
    (text, from)
}

/// The text of characters `a..=b` (in drawn order), joined as [`join`] does.
pub fn selected(chars: &[Char], a: usize, b: usize) -> String {
    if chars.is_empty() || a > b {
        return String::new();
    }
    let b = b.min(chars.len() - 1);
    join(&chars[a.min(b)..=b]).0
}

/// The boxes of characters `a..=b`, one per run on a line.
pub fn boxes(chars: &[Char], a: usize, b: usize) -> Vec<[f32; 4]> {
    let mut out: Vec<[f32; 4]> = Vec::new();
    if chars.is_empty() || a > b {
        return out;
    }
    let b = b.min(chars.len() - 1);
    let mut prev: Option<&Char> = None;
    for c in &chars[a.min(b)..=b] {
        match (out.last_mut(), prev) {
            (Some(r), Some(p)) if !new_line(p, c) && c.rect[0] >= r[0] - p.height() => {
                *r = [r[0].min(c.rect[0]), r[1].min(c.rect[1]), r[2].max(c.rect[2]), r[3].max(c.rect[3])];
            }
            _ => out.push(c.rect),
        }
        prev = Some(c);
    }
    out
}

/// The character at or nearest to (x, y): one whose line holds y, the
/// closest along it; else the closest of all.
pub fn nearest(chars: &[Char], x: f32, y: f32) -> Option<usize> {
    let dist = |c: &Char| {
        let dx = if x < c.rect[0] {
            c.rect[0] - x
        } else if x > c.rect[2] {
            x - c.rect[2]
        } else {
            0.0
        };
        let dy = if y < c.rect[1] {
            c.rect[1] - y
        } else if y > c.rect[3] {
            y - c.rect[3]
        } else {
            0.0
        };
        (dy, dx)
    };
    let on_line = chars
        .iter()
        .enumerate()
        .filter(|(_, c)| y >= c.rect[1] && y <= c.rect[3])
        .min_by(|a, b| dist(a.1).1.total_cmp(&dist(b.1).1));
    on_line
        .or_else(|| {
            chars.iter().enumerate().min_by(|a, b| {
                let (da, db) = (dist(a.1), dist(b.1));
                (da.0 * 4.0 + da.1).total_cmp(&(db.0 * 4.0 + db.1))
            })
        })
        .map(|(i, _)| i)
}

/// Whether (x, y) is on text: inside a character's box, a little grown.
pub fn on_text(chars: &[Char], x: f32, y: f32) -> bool {
    chars.iter().any(|c| {
        let g = c.height() * 0.25;
        x >= c.rect[0] - g && x <= c.rect[2] + g && y >= c.rect[1] - g * 0.5 && y <= c.rect[3] + g * 0.5
    })
}

/// Folding for finding: lower case, and every run of white space one space.
fn fold(c: char) -> char {
    if c.is_whitespace() { ' ' } else { c.to_lowercase().next().unwrap_or(c) }
}

/// Every place `query` is found (case and white space aside), as ranges
/// of drawn characters `(first, last)`.
pub fn find(chars: &[Char], query: &str) -> Vec<(usize, usize)> {
    let q: Vec<char> = {
        let mut v: Vec<char> = Vec::new();
        for c in query.trim().chars().map(fold) {
            if !(c == ' ' && v.last() == Some(&' ')) {
                v.push(c);
            }
        }
        v
    };
    if q.is_empty() {
        return Vec::new();
    }
    let (text, from) = join(chars);
    // The page's text folded the same way, runs of space as one.
    let mut t: Vec<char> = Vec::new();
    let mut tf: Vec<Option<usize>> = Vec::new();
    for (c, f) in text.chars().map(fold).zip(from) {
        if c == ' ' && t.last() == Some(&' ') {
            continue;
        }
        t.push(c);
        tf.push(f);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= t.len() {
        if t[i..i + q.len()] == q[..] {
            let first = tf[i..i + q.len()].iter().flatten().next();
            let last = tf[i..i + q.len()].iter().rev().flatten().next();
            if let (Some(&a), Some(&b)) = (first, last) {
                out.push((a, b));
            }
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Hello world" on one line (the space a gap), "Next line" below.
    fn page() -> Vec<Char> {
        let mut v = Vec::new();
        let mut put = |s: &str, x0: f32, y: f32| {
            let mut x = x0;
            for c in s.chars() {
                v.push(Char { text: c.to_string(), rect: [x, y, x + 6.0, y + 12.0] });
                x += 6.0;
            }
        };
        put("Hello", 10.0, 20.0);
        put("world", 10.0 + 5.0 * 6.0 + 4.0, 20.0);
        put("Next", 10.0, 36.0);
        put("line", 10.0 + 4.0 * 6.0 + 3.0, 36.0);
        v
    }

    #[test]
    fn joined_with_the_spaces_and_breaks_the_positions_imply() {
        let (text, from) = join(&page());
        assert_eq!(text, "Hello world\nNext line");
        assert_eq!(from[5], None);
        assert_eq!(from[6], Some(5));
    }

    #[test]
    fn finding_folds_case_and_white_space() {
        let p = page();
        assert_eq!(find(&p, "WORLD"), vec![(5, 9)]);
        // Across the line break.
        assert_eq!(find(&p, "world next"), vec![(5, 13)]);
        assert_eq!(find(&p, "o"), vec![(4, 4), (6, 6)]);
        assert!(find(&p, "absent").is_empty());
        assert!(find(&p, "  ").is_empty());
    }

    #[test]
    fn a_selection_s_text_and_boxes() {
        let p = page();
        assert_eq!(selected(&p, 6, 11), "orld\nNe");
        let b = boxes(&p, 6, 11);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], [50.0, 20.0, 74.0, 32.0]);
        assert_eq!(b[1], [10.0, 36.0, 22.0, 48.0]);
    }

    #[test]
    fn nearest_and_on_text() {
        let p = page();
        assert_eq!(nearest(&p, 13.0, 25.0), Some(0));
        assert_eq!(nearest(&p, 500.0, 25.0), Some(9));
        assert_eq!(nearest(&p, 12.0, 40.0), Some(10));
        assert!(on_text(&p, 13.0, 25.0));
        assert!(!on_text(&p, 300.0, 300.0));
    }

    #[test]
    fn overprinted_characters_kept_once() {
        let c = |x: f32| Char { text: "B".into(), rect: [x, 0.0, x + 6.0, 12.0] };
        assert_eq!(dedupe(vec![c(0.0), c(0.3), c(7.0)]).len(), 2);
    }
}
