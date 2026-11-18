//! Finding an application's icon by name, in the icon themes a system has.
//!
//! The lookup the icon theme specification describes, as much of it as a
//! shelf needs: the theme the settings name, the themes it inherits from,
//! then `hicolor` (every system's fallback), then loose files in
//! `pixmaps`. Within a theme a scalable picture (SVG) is preferred, so the
//! icon is sharp at any scale; else the fixed size nearest what is wanted,
//! erring on the large side.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// The most of an `index.theme` that is read, and how far themes may
/// inherit from one another.
const MAX_INDEX: usize = 512 * 1024;
const MAX_THEMES: usize = 8;

/// One directory of a theme, as its index describes it.
#[derive(Debug, Clone, PartialEq)]
struct Dir {
    path: String,
    /// The icons' size in it, in pixels, and the scale they are meant for.
    size: u32,
    scale: u32,
}

/// What an `index.theme` says: its directories, and the themes it inherits from.
fn index(text: &str) -> (Vec<Dir>, Vec<String>) {
    let text = text.get(..text.len().min(MAX_INDEX)).unwrap_or("");
    let (mut dirs, mut inherits): (Vec<Dir>, Vec<String>) = (Vec::new(), Vec::new());
    let mut listed: HashSet<String> = HashSet::new();
    let mut group = String::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name.to_string();
            if listed.contains(&group) {
                dirs.push(Dir { path: group.clone(), size: 0, scale: 1 });
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        if group == "Icon Theme" {
            match key {
                "Directories" | "ScaledDirectories" => listed.extend(value.split(',').map(str::trim).filter(|d| !d.is_empty() && !d.contains("..")).map(str::to_string)),
                "Inherits" => inherits.extend(value.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)),
                _ => {}
            }
        } else if let Some(dir) = dirs.last_mut().filter(|d| d.path == group) {
            match key {
                "Size" => dir.size = value.parse().unwrap_or(0),
                "Scale" => dir.scale = value.parse().unwrap_or(1).max(1),
                _ => {}
            }
        }
    }
    dirs.retain(|d| d.size > 0);
    (dirs, inherits)
}

/// Where icon themes live: `~/.icons`, then `icons` in each data directory.
fn theme_roots(data_dirs: &[PathBuf], home: Option<&Path>) -> Vec<PathBuf> {
    home.map(|h| h.join(".icons")).into_iter().chain(data_dirs.iter().map(|d| d.join("icons"))).collect()
}

/// The file of the icon `name` nearest `pixels` in one theme, if it has
/// one, and the themes that theme inherits from. A theme's index is the
/// first found among the roots; its directories are looked for under every
/// root (a package may add icons to a theme whose index another directory
/// holds: `/usr/local/share/icons/hicolor` beside `/usr/share`'s).
fn in_theme(roots: &[PathBuf], theme: &str, name: &str, pixels: u32) -> (Option<PathBuf>, Vec<String>) {
    let Some(text) = roots.iter().find_map(|root| std::fs::read_to_string(root.join(theme).join("index.theme")).ok()) else {
        return (None, Vec::new());
    };
    let (dirs, inherits) = index(&text);
    let mut best: Option<(u64, PathBuf)> = None;
    // Every directory of the theme, not only its applications': an entry may name
    // an icon of any kind ("printer", "preferences-desktop-theme").
    for dir in &dirs {
        for root in roots {
            for (ext, vector) in [("svg", true), ("png", false)] {
                let file = root.join(theme).join(&dir.path).join(format!("{name}.{ext}"));
                if !file.is_file() {
                    continue;
                }
                // Lower is better: a drawing (SVG), then the nearest size, larger before
                // smaller. A PNG is ranked by its size even in a directory the index calls
                // scalable: hicolor's 256x256 is one, and a 256-pixel picture is not what
                // a 32-pixel icon wants when there is a 32.
                let real = dir.size * dir.scale;
                let rank = if vector { 0 } else if real >= pixels { 1 + (real - pixels) as u64 } else { 100_000 + (pixels - real) as u64 };
                if best.as_ref().is_none_or(|(r, _)| rank < *r) {
                    best = Some((rank, file));
                }
            }
        }
    }
    (best.map(|(_, file)| file), inherits)
}

/// The file to draw for the icon `name` at about `pixels` across. `name`
/// may be a path, as some entries give one; then it is that file if it is
/// there. `theme` is the settings' icon theme (or ""). None if nothing
/// has it: the shelf then draws a tile with the application's first letter.
pub fn find_in(data_dirs: &[PathBuf], home: Option<&Path>, theme: &str, name: &str, pixels: u32) -> Option<PathBuf> {
    if name.is_empty() || name.len() > 512 {
        return None;
    }
    if name.starts_with('/') {
        let path = Path::new(name);
        return path.is_file().then(|| path.to_path_buf());
    }
    if name.contains('/') || name.contains("..") {
        return None;
    }
    let roots = theme_roots(data_dirs, home);
    // The named theme, what it inherits from (breadth first), and hicolor last of all.
    let mut queue: Vec<String> = if theme.is_empty() { Vec::new() } else { vec![theme.to_string()] };
    let mut tried: HashSet<String> = HashSet::new();
    let mut at = 0;
    while at < queue.len() && tried.len() < MAX_THEMES {
        let theme = queue[at].clone();
        at += 1;
        if theme == "hicolor" || !tried.insert(theme.clone()) {
            continue;
        }
        let (found, inherits) = in_theme(&roots, &theme, name, pixels);
        if found.is_some() {
            return found;
        }
        queue.extend(inherits);
    }
    if let (Some(found), _) = in_theme(&roots, "hicolor", name, pixels) {
        return Some(found);
    }
    // Loose files: an application that installed one picture and no theme.
    for dir in data_dirs {
        for ext in ["svg", "png"] {
            let file = dir.join("pixmaps").join(format!("{name}.{ext}"));
            if file.is_file() {
                return Some(file);
            }
        }
    }
    None
}

/// One of Vergence's own icons (tools/make-icons.py, 2026-11-18): `group`
/// "places" (home, trash, trash-full, folder, disk, usb ...) or "kinds"
/// (text, picture, pdf ...). Installed in share/vergence/icons/<group>
/// (install.sh); a checkout's own data/icons/<group> beside its build
/// (target/release/../../data) for a development session.
pub fn vergence_icon(group: &str, name: &str) -> Option<PathBuf> {
    let file = format!("{group}/{name}.svg");
    let mut dirs: Vec<PathBuf> = crate::apps::data_dirs().into_iter().map(|d| d.join("vergence/icons")).collect();
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        dirs.insert(0, bin.join("../share/vergence/icons"));
        dirs.push(bin.join("../../data/icons"));
    }
    dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// [`find_in`] this machine's data directories and the user's home.
pub fn find(theme: &str, name: &str, pixels: u32) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    find_in(&crate::apps::data_dirs(), home.as_deref(), theme, name, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::Scratch;

    const HICOLOR: &str = "[Icon Theme]\nName=Hicolor\nDirectories=16x16/apps,48x48/apps,256x256/apps,scalable/apps,48x48/actions,32x32@2/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[48x48/apps]\nSize=48\nType=Fixed\n\n[256x256/apps]\nSize=256\nMinSize=64\nMaxSize=256\nType=Scalable\n\n[scalable/apps]\nSize=64\nMinSize=8\nMaxSize=512\nType=Scalable\n\n[48x48/actions]\nSize=48\n\n[32x32@2/apps]\nSize=32\nScale=2\n";

    fn system() -> Scratch {
        let s = Scratch::new("icons");
        s.write("share/icons/hicolor/index.theme", HICOLOR);
        for size in ["16x16", "48x48", "256x256"] {
            s.write(&format!("share/icons/hicolor/{size}/apps/firefox.png"), "png");
        }
        s.write("share/icons/hicolor/48x48/apps/htop.png", "png");
        s.write("share/icons/hicolor/scalable/apps/htop.svg", "svg");
        s.write("share/icons/hicolor/32x32@2/apps/retina.png", "png");
        s.write("share/icons/hicolor/16x16/apps/retina.png", "png");
        s.write("share/icons/hicolor/48x48/actions/firefox-action.png", "png");
        s.write("share/icons/Fancy/index.theme", "[Icon Theme]\nName=Fancy\nInherits=Plain,hicolor\nDirectories=48/apps\n\n[48/apps]\nSize=48\n");
        s.write("share/icons/Fancy/48/apps/firefox.png", "png");
        s.write("share/icons/Plain/index.theme", "[Icon Theme]\nName=Plain\nDirectories=64/apps\n\n[64/apps]\nSize=64\n");
        s.write("share/icons/Plain/64/apps/files.png", "png");
        s.write("share/pixmaps/oldapp.png", "png");
        s.write("share/pixmaps/direct.png", "png");
        s
    }

    #[test]
    fn the_nearest_size_and_the_scalable_first() {
        let s = system();
        let dirs = [s.0.join("share")];
        let find = |theme: &str, name: &str, px: u32| find_in(&dirs, None, theme, name, px).map(|p| p.strip_prefix(&s.0).unwrap().to_string_lossy().into_owned());
        // Fixed sizes: the nearest that is large enough; the largest when none is.
        assert_eq!(find("", "firefox", 32).as_deref(), Some("share/icons/hicolor/48x48/apps/firefox.png"));
        assert_eq!(find("", "firefox", 96).as_deref(), Some("share/icons/hicolor/256x256/apps/firefox.png"));
        assert_eq!(find("", "firefox", 16).as_deref(), Some("share/icons/hicolor/16x16/apps/firefox.png"));
        assert_eq!(find("", "firefox", 1000).as_deref(), Some("share/icons/hicolor/256x256/apps/firefox.png"));
        // A scalable one wins whatever the size.
        assert_eq!(find("", "htop", 48).as_deref(), Some("share/icons/hicolor/scalable/apps/htop.svg"));
        // A directory's scale counts: 32 at scale 2 is 64 pixels.
        assert_eq!(find("", "retina", 60).as_deref(), Some("share/icons/hicolor/32x32@2/apps/retina.png"));
        // The named theme first, then what it inherits from, then hicolor, then pixmaps.
        assert_eq!(find("Fancy", "firefox", 32).as_deref(), Some("share/icons/Fancy/48/apps/firefox.png"));
        assert_eq!(find("Fancy", "files", 32).as_deref(), Some("share/icons/Plain/64/apps/files.png"));
        assert_eq!(find("Fancy", "htop", 32).as_deref(), Some("share/icons/hicolor/scalable/apps/htop.svg"));
        assert_eq!(find("NoSuchTheme", "firefox", 32).as_deref(), Some("share/icons/hicolor/48x48/apps/firefox.png"));
        assert_eq!(find("", "oldapp", 32).as_deref(), Some("share/pixmaps/oldapp.png"));
        // A second data directory adds icons to a theme whose index the first holds.
        s.write("local/icons/hicolor/scalable/apps/ours.svg", "svg");
        let both = [s.0.join("local"), s.0.join("share")];
        assert_eq!(find_in(&both, None, "", "ours", 32), Some(s.0.join("local/icons/hicolor/scalable/apps/ours.svg")));
        assert_eq!(find_in(&both, None, "", "firefox", 32), Some(s.0.join("share/icons/hicolor/48x48/apps/firefox.png")));
        // An icon of another kind than an application's is an icon all the same.
        assert_eq!(find("", "firefox-action", 32).as_deref(), Some("share/icons/hicolor/48x48/actions/firefox-action.png"));
        // Not there at all, not a name.
        assert_eq!(find("", "nothing", 32), None);
        assert_eq!(find("", "", 32), None);
        assert_eq!(find("", "../pixmaps/direct", 32), None);
        assert_eq!(find("", "sub/direct", 32), None);
        // A path is that file, if it is there.
        let direct = s.0.join("share/pixmaps/direct.png");
        assert_eq!(find_in(&dirs, None, "", direct.to_str().unwrap(), 32), Some(direct));
        assert_eq!(find_in(&dirs, None, "", "/no/such/icon.png", 32), None);
    }

    #[test]
    fn a_theme_that_inherits_from_itself_ends() {
        let s = Scratch::new("icons-loop");
        s.write("share/icons/A/index.theme", "[Icon Theme]\nInherits=B\nDirectories=\n");
        s.write("share/icons/B/index.theme", "[Icon Theme]\nInherits=A,A,B\nDirectories=\n");
        assert_eq!(find_in(&[s.0.join("share")], None, "A", "anything", 32), None);
    }

    #[test]
    fn an_index_is_read_as_far_as_it_makes_sense() {
        let (dirs, inherits) = index(HICOLOR);
        assert_eq!(dirs.len(), 6);
        assert!(dirs.iter().any(|d| d.path == "scalable/apps" && d.size == 64));
        assert!(dirs.iter().any(|d| d.path == "32x32@2/apps" && d.scale == 2));
        assert!(inherits.is_empty());
        // Directories the index does not list, sizes that are not numbers, paths that climb out.
        let (dirs, inherits) = index("[Icon Theme]\nInherits= a , b\nDirectories=ok,../up,nosize\n[ok]\nSize=24\n[unlisted]\nSize=24\n[nosize]\nSize=big\n[../up]\nSize=24\n");
        assert_eq!(dirs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), ["ok"]);
        assert_eq!(inherits, ["a", "b"]);
        assert_eq!(index(""), (Vec::new(), Vec::new()));
    }
}
