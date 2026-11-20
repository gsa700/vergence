//! A document's outline (bookmarks), its destinations and its pages' links
//! (ISO 32000-2 §12.3), for the viewer's sidebar and clicks.
//!
//! A destination becomes a page (from 0) and, when it says one, how far
//! down that page it points (points from the page's top as shown, after
//! /Rotate). Named destinations come from the catalog's /Dests or its
//! /Names /Dests tree. Untrusted: walks are bounded in depth and count and
//! cut cycles; titles and addresses are cut to a sensible length.

use std::collections::HashSet;

use crate::object::{Dict, Object, Ref};
use crate::{Document, Page};

const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 24;
const MAX_TITLE: usize = 512;
const MAX_URI: usize = 4096;

/// Where a destination points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub page: usize,
    /// Points from the top of the page as shown, if the destination says.
    pub top: Option<f32>,
}

/// One outline entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// 0 for the top level.
    pub level: u8,
    pub title: String,
    pub target: Option<Target>,
}

/// What a link does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    GoTo(Target),
    /// An address outside the document (the viewer asks before opening it).
    Uri(String),
}

/// A link on a page: its rectangle (x0, y0, x1, y1 in points, from the
/// page's top left as shown) and what it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub rect: [f32; 4],
    pub action: Action,
}

/// A text string (§7.9.2.2): UTF-16BE or UTF-8 by their marks, else
/// PDFDocEncoding.
pub fn text_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| pdf_doc_char(c)).collect()
}

fn pdf_doc_char(c: u8) -> char {
    const LOW: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
    const HIGH: [char; 33] = ['•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}', '€'];
    match c {
        0x18..=0x1F => LOW[(c - 0x18) as usize],
        0x80..=0xA0 => HIGH[(c - 0x80) as usize],
        _ => c as char,
    }
}

fn cut(mut s: String, most: usize) -> String {
    if s.len() > most {
        let mut i = most;
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        s.truncate(i);
    }
    s
}

/// What it takes to turn destinations into pages.
pub struct Resolver<'a> {
    doc: &'a Document,
    pages: &'a [Page],
    catalog: Dict,
}

impl<'a> Resolver<'a> {
    pub fn new(doc: &'a Document, pages: &'a [Page]) -> Resolver<'a> {
        Resolver { doc, pages, catalog: doc.catalog().unwrap_or_default() }
    }

    fn page_of(&self, o: &Object) -> Option<usize> {
        match o {
            Object::Ref(r) => self.pages.iter().position(|p| p.reference == Some(*r)),
            // Some writers put a page number (from 0) where the page belongs.
            Object::Int(n) if *n >= 0 && (*n as usize) < self.pages.len() => Some(*n as usize),
            _ => None,
        }
    }

    /// A destination: an array, a name or string to look up, or a
    /// dictionary with /D.
    pub fn destination(&self, o: &Object) -> Option<Target> {
        self.destination_at(o, 0)
    }

    fn destination_at(&self, o: &Object, depth: usize) -> Option<Target> {
        if depth > 4 {
            return None;
        }
        let o = match o {
            Object::Ref(_) => self.doc.resolve(o),
            other => other.clone(),
        };
        match &o {
            Object::Array(a) => {
                let page = self.page_of(a.first()?)?;
                let kind = a.get(1).and_then(|k| k.as_name()).unwrap_or(b"Fit");
                // XYZ left top zoom; FitH top; FitBH top; FitR left bottom right top.
                let y = match kind {
                    b"XYZ" => a.get(3),
                    b"FitH" | b"FitBH" => a.get(2),
                    b"FitR" => a.get(5),
                    _ => None,
                }
                .and_then(|v| self.doc.resolve(v).as_f64());
                let x = match kind {
                    b"XYZ" | b"FitR" => a.get(2).and_then(|v| self.doc.resolve(v).as_f64()),
                    _ => None,
                };
                let top = y.map(|y| {
                    let p = &self.pages[page];
                    let b = crate::render::page_box(p);
                    let t = crate::render::page_transform(b, p.rotate, 1.0);
                    let pt = t.apply(vergence_raster::geom::Point { x: x.unwrap_or(b[0]), y });
                    pt.y.max(0.0) as f32
                });
                Some(Target { page, top })
            }
            Object::Name(n) => self.named(n, depth),
            Object::String(s) => self.named(s, depth),
            Object::Dict(d) => self.destination_at(d.get("D")?, depth + 1),
            _ => None,
        }
    }

    /// A named destination: /Dests (names) or the /Names /Dests tree (strings).
    fn named(&self, name: &[u8], depth: usize) -> Option<Target> {
        if let Some(dests) = self.doc.get_in(&self.catalog, "Dests").as_dict()
            && let Some(v) = dests.get(std::str::from_utf8(name).ok()?)
        {
            return self.destination_at(v, depth + 1);
        }
        let names = self.doc.get_in(&self.catalog, "Names");
        let tree = self.doc.get_in(names.as_dict()?, "Dests");
        let v = self.tree_lookup(&tree, name, 0, &mut HashSet::new())?;
        self.destination_at(&v, depth + 1)
    }

    fn tree_lookup(&self, node: &Object, key: &[u8], depth: usize, seen: &mut HashSet<Ref>) -> Option<Object> {
        if depth > MAX_DEPTH {
            return None;
        }
        if let Object::Ref(r) = node
            && !seen.insert(*r)
        {
            return None;
        }
        let node = self.doc.resolve(node);
        let d = node.as_dict()?;
        if let Some(names) = self.doc.get_in(d, "Names").as_array() {
            for pair in names.chunks(2) {
                if let [k, v] = pair
                    && self.doc.resolve(k).as_bytes() == Some(key)
                {
                    return Some(v.clone());
                }
            }
        }
        for kid in self.doc.get_in(d, "Kids").as_array().unwrap_or(&[]) {
            // Limits, when given, skip kids that cannot hold the key.
            let kd = self.doc.resolve(kid);
            if let Some(l) = kd.as_dict().and_then(|k| self.doc.get_in(k, "Limits").as_array().map(|a| a.to_vec()))
                && let (Some(lo), Some(hi)) = (l.first().and_then(|o| self.doc.resolve(o).as_bytes().map(<[u8]>::to_vec)), l.get(1).and_then(|o| self.doc.resolve(o).as_bytes().map(<[u8]>::to_vec)))
                && (key < lo.as_slice() || key > hi.as_slice())
            {
                continue;
            }
            if let Some(v) = self.tree_lookup(kid, key, depth + 1, seen) {
                return Some(v);
            }
        }
        None
    }

    /// An action's target, for GoTo; None for the rest.
    fn goto(&self, a: &Dict) -> Option<Target> {
        match self.doc.get_in(a, "S").as_name()? {
            b"GoTo" => self.destination(a.get("D")?),
            _ => None,
        }
    }
}

/// The outline, depth first, levels from 0.
pub fn outline(doc: &Document, pages: &[Page]) -> Vec<Entry> {
    let r = Resolver::new(doc, pages);
    let mut out = Vec::new();
    let Some(root) = doc.get_in(&r.catalog, "Outlines").as_dict().cloned() else { return out };
    let mut seen = HashSet::new();
    walk(&r, root.get("First"), 0, &mut out, &mut seen);
    out
}

fn walk(r: &Resolver, first: Option<&Object>, level: usize, out: &mut Vec<Entry>, seen: &mut HashSet<Ref>) {
    let mut next = first.cloned();
    while let Some(item) = next.take() {
        if out.len() >= MAX_ENTRIES || level > MAX_DEPTH {
            return;
        }
        let Object::Ref(at) = item else { return };
        if !seen.insert(at) {
            return;
        }
        let o = r.doc.get(at);
        let Some(d) = o.as_dict() else { return };
        let title = r.doc.get_in(d, "Title").as_bytes().map(text_string).unwrap_or_default();
        let title = cut(title.replace(['\r', '\n', '\t'], " ").trim().to_string(), MAX_TITLE);
        let target = match d.get("Dest") {
            Some(dest) => r.destination(dest),
            None => r.doc.get_in(d, "A").as_dict().and_then(|a| r.goto(a)),
        };
        out.push(Entry { level: level.min(255) as u8, title, target });
        walk(r, d.get("First"), level + 1, out, seen);
        next = d.get("Next").cloned();
    }
}

/// A page's links.
pub fn links(doc: &Document, pages: &[Page], index: usize) -> Vec<Link> {
    let Some(page) = pages.get(index) else { return Vec::new() };
    let r = Resolver::new(doc, pages);
    let b = crate::render::page_box(page);
    let t = crate::render::page_transform(b, page.rotate, 1.0);
    let mut out = Vec::new();
    for a in doc.get_in(&page.dict, "Annots").as_array().unwrap_or(&[]).iter().take(4096) {
        let o = doc.resolve(a);
        let Some(d) = o.as_dict() else { continue };
        if doc.get_in(d, "Subtype").as_name() != Some(b"Link") {
            continue;
        }
        let rect: Vec<f64> = doc.get_in(d, "Rect").as_array().unwrap_or(&[]).iter().filter_map(|v| doc.resolve(v).as_f64()).collect();
        let [x0, y0, x1, y1] = rect[..] else { continue };
        let p0 = t.apply(vergence_raster::geom::Point { x: x0, y: y0 });
        let p1 = t.apply(vergence_raster::geom::Point { x: x1, y: y1 });
        let rect = [p0.x.min(p1.x) as f32, p0.y.min(p1.y) as f32, p0.x.max(p1.x) as f32, p0.y.max(p1.y) as f32];
        if !rect.iter().all(|v| v.is_finite()) || rect[2] - rect[0] < 0.5 || rect[3] - rect[1] < 0.5 {
            continue;
        }
        let action = if let Some(dest) = d.get("Dest") {
            r.destination(dest).map(Action::GoTo)
        } else if let Some(a) = doc.get_in(d, "A").as_dict() {
            match doc.get_in(a, "S").as_name() {
                Some(b"GoTo") => r.goto(a).map(Action::GoTo),
                Some(b"URI") => doc.get_in(a, "URI").as_bytes().map(|u| Action::Uri(cut(String::from_utf8_lossy(u).trim().to_string(), MAX_URI))).filter(|a| !matches!(a, Action::Uri(u) if u.is_empty())),
                Some(b"Named") => {
                    let to = match doc.get_in(a, "N").as_name() {
                        Some(b"NextPage") => Some(index + 1),
                        Some(b"PrevPage") => index.checked_sub(1),
                        Some(b"FirstPage") => Some(0),
                        Some(b"LastPage") => pages.len().checked_sub(1),
                        _ => None,
                    };
                    to.filter(|p| *p < pages.len()).map(|page| Action::GoTo(Target { page, top: None }))
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(action) = action {
            out.push(Link { rect, action });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-page document (612 x 792) with an outline (two entries, one
    /// nested, one by a named destination) and links on page 1.
    fn document() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R /Names << /Dests << /Names [(chap2) [4 0 R /XYZ 0 700 0]] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [8 0 R 9 0 R 10 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Outlines /First 6 0 R /Last 7 0 R >>",
            "<< /Title (Intro \\204 start) /Parent 5 0 R /Next 7 0 R /First 11 0 R /Dest [3 0 R /Fit] >>",
            "<< /Title <FEFF004300680061007000740065007200200032> /Parent 5 0 R /Prev 6 0 R /Dest (chap2) >>",
            "<< /Type /Annot /Subtype /Link /Rect [72 700 172 720] /Dest [4 0 R /FitH 500] >>",
            "<< /Type /Annot /Subtype /Link /Rect [72 600 172 620] /A << /S /URI /URI (https://example.org/) >> >>",
            "<< /Type /Annot /Subtype /Link /Rect [72 500 172 520] /A << /S /Named /N /NextPage >> >>",
            "<< /Title (Nested) /Parent 6 0 R /A << /S /GoTo /D [4 0 R /XYZ null 792 null] >> >>",
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    #[test]
    fn text_strings() {
        assert_eq!(text_string(b"plain"), "plain");
        assert_eq!(text_string(&[0xFE, 0xFF, 0x00, 0x41, 0x26, 0x60]), "A♠");
        assert_eq!(text_string(&[0x93, 0x84, 0xA0]), "ﬁ—€");
    }

    #[test]
    fn the_outline_and_its_destinations() {
        let doc = Document::open(document()).unwrap();
        let pages = doc.pages();
        let o = outline(&doc, &pages);
        assert_eq!(o.len(), 3);
        assert_eq!(o[0], Entry { level: 0, title: "Intro — start".into(), target: Some(Target { page: 0, top: None }) });
        assert_eq!(o[1], Entry { level: 1, title: "Nested".into(), target: Some(Target { page: 1, top: Some(0.0) }) });
        // By name, through the Names tree: y 700 is 92 points down.
        assert_eq!(o[2], Entry { level: 0, title: "Chapter 2".into(), target: Some(Target { page: 1, top: Some(92.0) }) });
    }

    #[test]
    fn links_on_a_page() {
        let doc = Document::open(document()).unwrap();
        let pages = doc.pages();
        let l = links(&doc, &pages, 0);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0], Link { rect: [72.0, 72.0, 172.0, 92.0], action: Action::GoTo(Target { page: 1, top: Some(292.0) }) });
        assert_eq!(l[1].action, Action::Uri("https://example.org/".into()));
        assert_eq!(l[2].action, Action::GoTo(Target { page: 1, top: None }));
        assert!(links(&doc, &pages, 1).is_empty());
        assert!(links(&doc, &pages, 9).is_empty());
    }

    #[test]
    fn an_outline_that_loops_ends() {
        let mut d = document();
        // Make the second entry point Next back at the first.
        let s = String::from_utf8_lossy(&d).replace("/Prev 6 0 R /Dest (chap2)", "/Next 6 0 R /Dest (chap2)");
        d = s.into_bytes();
        let doc = Document::open(d).unwrap();
        let pages = doc.pages();
        assert_eq!(outline(&doc, &pages).len(), 3);
    }
}
