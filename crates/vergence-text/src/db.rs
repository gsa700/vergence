//! The fonts on this machine: which files hold which faces, the face that
//! best matches a family, weight and slant, and the face to fall back on for
//! a character the chosen one lacks.
//!
//! Font directories are walked and each file's faces indexed with sfnt.rs,
//! which reads a few kilobytes a file. A face's character coverage is read
//! the first time a fallback search asks about it, and kept.
//!
//! There is no fontconfig here and no configuration language. A family is
//! found by its name; the three generic names (monospace, sans-serif, serif,
//! and the spellings desktop settings use: "Monospace", "Sans") try a short
//! list of families in order and then any face of the right kind.

use std::{
    cell::OnceCell,
    path::{Path, PathBuf},
};

use crate::sfnt::{self, Coverage, FaceInfo};

/// The most font files indexed, and how deep directories are followed.
const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 8;

/// The usual families, best first. Vergence's own two lead (Dave, 2026-11-16:
/// a uniform look everywhere, "really important"): Inter for text and the
/// interface, JetBrains Mono for code, both shipped in share/vergence/fonts
/// and found there when the system has none of its own.
const MONOSPACE: [&str; 7] = [
    "JetBrains Mono",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Adwaita Mono",
    "Source Code Pro",
    "Cascadia Mono",
];
const SANS: [&str; 7] =
    ["Inter", "Inter Variable", "Noto Sans", "DejaVu Sans", "Liberation Sans", "Adwaita Sans", "Cantarell"];
const SERIF: [&str; 3] = ["Noto Serif", "DejaVu Serif", "Liberation Serif"];

pub struct Entry {
    pub path: PathBuf,
    pub info: FaceInfo,
    coverage: OnceCell<Coverage>,
}

impl Entry {
    pub fn new(path: PathBuf, info: FaceInfo) -> Entry {
        Entry { path, info, coverage: OnceCell::new() }
    }

    /// An entry whose coverage is already known (tests).
    pub fn with_coverage(path: PathBuf, info: FaceInfo, coverage: Coverage) -> Entry {
        let cell = OnceCell::new();
        let _ = cell.set(coverage);
        Entry { path, info, coverage: cell }
    }

    /// The characters this face has (empty if its file can no longer be read).
    pub fn coverage(&self) -> &Coverage {
        self.coverage.get_or_init(|| {
            let Some((off, len)) = self.info.cmap else { return Coverage::default() };
            let Ok(mut file) = std::fs::File::open(&self.path) else { return Coverage::default() };
            sfnt::Source::read_at(&mut file, off, len as usize).map(|t| sfnt::coverage(&t)).unwrap_or_default()
        })
    }
}

#[derive(Default)]
pub struct FontDb {
    pub faces: Vec<Entry>,
}

impl FontDb {
    /// Every font under the usual directories: the user's, then the system's
    /// (`/usr/share/fonts`, and `/usr/local/share/fonts` where FreeBSD puts
    /// them), and each `fonts` directory under `XDG_DATA_DIRS`.
    pub fn system() -> FontDb {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let data =
                std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
            dirs.push(data.join("fonts"));
            dirs.push(home.join(".fonts"));
        }
        if let Some(list) = std::env::var_os("XDG_DATA_DIRS") {
            dirs.extend(std::env::split_paths(&list).map(|d| d.join("fonts")));
        }
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
        // Vergence's own, after the system's (a family the system has too is
        // the system's copy): beside the program ($prefix/share/vergence/fonts),
        // a checkout's data/fonts, and the usual prefixes.
        if let Ok(exe) = std::env::current_exe()
            && let Some(bin) = exe.parent()
        {
            dirs.push(bin.join("../share/vergence/fonts"));
            dirs.push(bin.join("../../data/fonts"));
        }
        dirs.push("/usr/local/share/vergence/fonts".into());
        dirs.push("/usr/share/vergence/fonts".into());
        let mut seen = Vec::new();
        dirs.retain(|d| {
            let fresh = !seen.contains(d);
            seen.push(d.clone());
            fresh
        });
        FontDb::scan(&dirs)
    }

    /// Every font under these directories, in a fixed order (directories as
    /// given, files by name), so the same machine always answers the same.
    pub fn scan(dirs: &[PathBuf]) -> FontDb {
        let mut files = Vec::new();
        for d in dirs {
            walk(d, 0, &mut files);
        }
        let mut faces = Vec::new();
        for path in files {
            let Ok(mut file) = std::fs::File::open(&path) else { continue };
            for info in sfnt::faces(&mut file) {
                faces.push(Entry::new(path.clone(), info));
            }
        }
        FontDb { faces }
    }

    /// The face for a family (or a generic name), as close as the family
    /// comes to the weight and slant asked for.
    pub fn find(&self, family: &str, weight: u16, italic: bool) -> Option<usize> {
        let wanted = family.trim();
        let generic: Option<(&[&str], bool)> = match wanted.to_ascii_lowercase().as_str() {
            "monospace" | "mono" => Some((&MONOSPACE, true)),
            "sans-serif" | "sans" | "system-ui" | "" => Some((&SANS, false)),
            "serif" => Some((&SERIF, false)),
            _ => None,
        };
        let Some((list, mono)) = generic else { return self.in_family(wanted, weight, italic) };
        list.iter().find_map(|f| self.in_family(f, weight, italic)).or_else(|| {
            // None of the usual families: any face of the right kind, the
            // nearest in weight and slant, the first of those.
            let family = self
                .faces
                .iter()
                .filter(|e| e.info.monospace == mono)
                .min_by_key(|e| distance(&e.info, weight, italic))
                .map(|e| e.info.family.clone());
            family.and_then(|f| self.in_family(&f, weight, italic))
        })
    }

    fn in_family(&self, family: &str, weight: u16, italic: bool) -> Option<usize> {
        self.faces
            .iter()
            .enumerate()
            .filter(|(_, e)| e.info.family.eq_ignore_ascii_case(family))
            .min_by_key(|(_, e)| distance(&e.info, weight, italic))
            .map(|(i, _)| i)
    }

    /// A face that has `c`, for text set in the face `base`: one as like it
    /// as there is (fixed-pitch for fixed-pitch, the same foundry's name, the
    /// same weight and slant).
    pub fn fallback(&self, c: char, base: usize) -> Option<usize> {
        let like = &self.faces.get(base)?.info;
        let foundry = like.family.split(' ').next().unwrap_or("");
        let mut order: Vec<(u32, u32, usize)> = self
            .faces
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != base)
            .map(|(i, e)| {
                let rank = (e.info.monospace != like.monospace) as u32 * 4
                    + (e.info.family.split(' ').next() != Some(foundry)) as u32 * 2
                    + (e.info.italic != like.italic) as u32;
                (
                    rank,
                    (e.info.width as i32 - like.width as i32).unsigned_abs() * 10_000
                        + weight_distance(&e.info, like.weight),
                    i,
                )
            })
            .collect();
        order.sort_unstable();
        // Coverage is read from a face's file only when the search reaches it.
        order.into_iter().map(|(_, _, i)| i).find(|i| self.faces[*i].coverage().contains(c))
    }
}

/// How far a face is from a weight (0 if a variable font can be it exactly).
fn weight_distance(info: &FaceInfo, weight: u16) -> u32 {
    match info.weight_axis {
        Some((min, max)) => (weight.clamp(min, max) as i32 - weight as i32).unsigned_abs(),
        None => (info.weight as i32 - weight as i32).unsigned_abs(),
    }
}

/// Slant first, then weight.
/// How far a face is from what is asked: the slant first, then the width
/// (a family's normal width always before its condensed or expanded faces:
/// kepler, 2026-11-16, set every text in Noto Sans Condensed, its file
/// sorting before Regular and the width not looked at; Dave: "the display
/// seems a little off ... slightly taller than wide"), then the weight.
fn distance(info: &FaceInfo, weight: u16, italic: bool) -> u32 {
    (info.italic != italic) as u32 * 100_000
        + (info.width as i32 - 5).unsigned_abs() * 10_000
        + weight_distance(info, weight)
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, depth + 1, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x)))
            && out.len() < MAX_FILES
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfnt::testfont::{Spec, font};

    fn entry(spec: Spec) -> Entry {
        let bytes = font(&spec);
        let info = sfnt::faces(&mut &bytes[..]).remove(0);
        let (off, len) = info.cmap.unwrap();
        let cov = sfnt::coverage(&bytes[off as usize..off as usize + len as usize]);
        Entry::with_coverage(format!("/fonts/{}-{}-{}.ttf", spec.family, spec.weight, spec.italic).into(), info, cov)
    }

    fn db() -> FontDb {
        FontDb {
            faces: vec![
                entry(Spec { family: "Liberation Mono", monospace: true, ..Spec::default() }),
                entry(Spec { family: "Noto Sans Mono", monospace: true, ..Spec::default() }),
                entry(Spec { family: "Noto Sans Mono", monospace: true, weight: 700, ..Spec::default() }),
                entry(Spec { family: "Noto Sans", ..Spec::default() }),
                entry(Spec { family: "Noto Sans", italic: true, ..Spec::default() }),
                entry(Spec { family: "Noto Sans Hebrew", ranges: &[(0x5d0, 0x5ea)], ..Spec::default() }),
                entry(Spec { family: "Other Symbols", ranges: &[(0x2500, 0x257f), (0x5d0, 0x5ea)], ..Spec::default() }),
                entry(Spec { family: "Example VF", weight_axis: Some((100, 900)), ..Spec::default() }),
            ],
        }
    }

    fn name(db: &FontDb, i: Option<usize>) -> Option<(&str, u16, bool)> {
        i.map(|i| (db.faces[i].info.family.as_str(), db.faces[i].info.weight, db.faces[i].info.italic))
    }

    #[test]
    fn a_family_by_name_weight_and_slant() {
        let db = db();
        assert_eq!(name(&db, db.find("Noto Sans Mono", 400, false)), Some(("Noto Sans Mono", 400, false)));
        assert_eq!(name(&db, db.find("noto sans mono", 700, false)), Some(("Noto Sans Mono", 700, false)));
        // The nearest weight the family has; slant before weight.
        assert_eq!(name(&db, db.find("Noto Sans Mono", 900, false)), Some(("Noto Sans Mono", 700, false)));
        assert_eq!(name(&db, db.find("Noto Sans", 700, true)), Some(("Noto Sans", 400, true)));
        assert_eq!(db.find("No Such Family", 400, false), None);
    }

    /// A family's condensed face, listed first (its file sorts before
    /// Regular, as Noto Sans's do on FreeBSD), is never the plain text's.
    #[test]
    fn the_normal_width_before_a_condensed_face() {
        let db = FontDb {
            faces: vec![
                entry(Spec { family: "Noto Sans", width: 3, ..Spec::default() }),
                entry(Spec { family: "Noto Sans", width: 3, weight: 700, ..Spec::default() }),
                entry(Spec { family: "Noto Sans", ..Spec::default() }),
                entry(Spec { family: "Noto Sans", weight: 700, ..Spec::default() }),
            ],
        };
        let regular = db.find("sans-serif", 400, false).unwrap();
        assert_eq!(db.faces[regular].info.width, 5);
        let bold = db.find("Noto Sans", 600, false).unwrap();
        assert_eq!(
            (db.faces[bold].info.width, db.faces[bold].info.weight),
            (5, 700),
            "a normal-width bold before a condensed one"
        );
    }

    #[test]
    fn generic_names() {
        let db = db();
        assert_eq!(name(&db, db.find("monospace", 400, false)), Some(("Noto Sans Mono", 400, false)));
        assert_eq!(name(&db, db.find("Monospace", 700, false)), Some(("Noto Sans Mono", 700, false)));
        assert_eq!(name(&db, db.find("Sans", 400, false)), Some(("Noto Sans", 400, false)));
        // Without the usual families, any fixed-pitch face.
        let only = FontDb {
            faces: vec![
                entry(Spec { family: "Plain", ..Spec::default() }),
                entry(Spec { family: "Odd Mono", monospace: true, ..Spec::default() }),
            ],
        };
        assert_eq!(name(&only, only.find("monospace", 400, false)), Some(("Odd Mono", 400, false)));
        assert_eq!(FontDb::default().find("monospace", 400, false), None);
    }

    #[test]
    fn a_variable_font_is_every_weight_in_its_range() {
        let db = db();
        let i = db.find("Example VF", 650, false).unwrap();
        assert_eq!(weight_distance(&db.faces[i].info, 650), 0);
        assert_eq!(weight_distance(&db.faces[i].info, 1000), 100);
    }

    #[test]
    fn fallback_prefers_a_face_like_the_one_it_stands_in_for() {
        let db = db();
        let mono = db.find("Noto Sans Mono", 400, false).unwrap();
        // Hebrew: nobody fixed-pitch has it; of the rest, the same foundry's.
        assert_eq!(name(&db, db.fallback('א', mono)).map(|n| n.0), Some("Noto Sans Hebrew"));
        // Box drawing: only one face has it.
        assert_eq!(name(&db, db.fallback('─', mono)).map(|n| n.0), Some("Other Symbols"));
        assert_eq!(db.fallback('漢', mono), None);
        // The base itself is never the answer; its own bold is the nearest other face.
        assert_eq!(name(&db, db.fallback('A', mono)), Some(("Noto Sans Mono", 700, false)));
    }
}
