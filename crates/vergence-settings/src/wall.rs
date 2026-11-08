// The wallpaper settings in desktop.toml, as the settings app edits them
// (2026-11-08: your own pictures, and a wallpaper per screen).
//
//     [wallpaper]                 all screens (the server's wallpaper.rs)
//     image = "..."  mode = "fill"  color = "#202020"
//     [[wallpaper.monitor]]       one screen's own; unset fields come from above
//     id = "<EDID hash>"  label = "ASUS VS229"  image = "..."  mode = "fit"
//
//     [pictures]                  the settings app's own: where to find pictures
//     folders = ["/home/me/Pictures/Walls"]  files = ["/home/me/photo.jpg"]
//
// Edits change only their own keys, so the menu's items, the appearance and
// anything else in the file stay as they are.

use std::path::{Path, PathBuf};

use toml::{Table, Value};

pub const MODES: [&str; 4] = ["fill", "fit", "center", "stretch"];

/// One screen's (or all screens') wallpaper as the page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub image: Option<PathBuf>,
    pub mode: String,
    pub color: String,
    /// For one screen: it has its own entry (else it shows all screens').
    pub own: bool,
}

fn table_mut<'a>(t: &'a mut Table, key: &str) -> &'a mut Table {
    if !t.get(key).is_some_and(Value::is_table) {
        t.insert(key.into(), Value::Table(Table::new()));
    }
    t.get_mut(key).and_then(Value::as_table_mut).unwrap()
}

fn str_of(t: &Table, k: &str) -> Option<String> {
    t.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn monitors(t: &Table) -> Vec<Table> {
    t.get("wallpaper")
        .and_then(|w| w.get("monitor"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_table).cloned().collect())
        .unwrap_or_default()
}

/// What the page shows for all screens (`id` None) or one screen.
pub fn shown(desktop: &Table, id: Option<&str>) -> Shown {
    let base = desktop.get("wallpaper").and_then(Value::as_table).cloned().unwrap_or_default();
    let mut s = Shown {
        image: str_of(&base, "image").filter(|s| !s.is_empty()).map(PathBuf::from),
        mode: str_of(&base, "mode").unwrap_or_else(|| "fill".into()),
        color: str_of(&base, "color").unwrap_or_else(|| "#202020".into()),
        own: false,
    };
    if let Some(id) = id
        && let Some(m) = monitors(desktop).into_iter().find(|m| str_of(m, "id").as_deref() == Some(id))
    {
        s.own = true;
        if let Some(i) = str_of(&m, "image") {
            s.image = (!i.is_empty()).then(|| PathBuf::from(i));
        }
        if let Some(v) = str_of(&m, "mode") {
            s.mode = v;
        }
        if let Some(v) = str_of(&m, "color") {
            s.color = v;
        }
    }
    s
}

/// Set the picture, fit and colour for all screens (`id` None) or one screen
/// (creating its own entry; `label` names it for people reading the file).
pub fn set(desktop: &mut Table, id: Option<&str>, label: &str, image: Option<&Path>, mode: &str, color: &str) {
    let image = image.map(|p| p.display().to_string()).unwrap_or_default();
    let wall = table_mut(desktop, "wallpaper");
    let target: &mut Table = match id {
        None => wall,
        Some(id) => {
            if !wall.get("monitor").is_some_and(Value::is_array) {
                wall.insert("monitor".into(), Value::Array(Vec::new()));
            }
            let list = wall.get_mut("monitor").and_then(Value::as_array_mut).unwrap();
            let pos = list.iter().position(|m| m.get("id").and_then(Value::as_str) == Some(id));
            let i = match pos {
                Some(i) => i,
                None => {
                    let mut t = Table::new();
                    t.insert("id".into(), id.into());
                    list.push(Value::Table(t));
                    list.len() - 1
                }
            };
            let t = list[i].as_table_mut().unwrap();
            t.insert("label".into(), label.into());
            t
        }
    };
    if image.is_empty() && id.is_none() {
        target.remove("image");
    } else {
        target.insert("image".into(), image.into());
    }
    target.insert("mode".into(), mode.into());
    target.insert("color".into(), color.into());
}

/// A screen goes back to all screens' wallpaper.
pub fn clear_own(desktop: &mut Table, id: &str) {
    let wall = table_mut(desktop, "wallpaper");
    if let Some(list) = wall.get_mut("monitor").and_then(Value::as_array_mut) {
        list.retain(|m| m.get("id").and_then(Value::as_str) != Some(id));
        if list.is_empty() {
            wall.remove("monitor");
        }
    }
}

/// The folders and files the user added.
pub fn sources(desktop: &Table) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let list = |k: &str| -> Vec<PathBuf> {
        desktop
            .get("pictures")
            .and_then(|p| p.get(k))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
            .unwrap_or_default()
    };
    (list("folders"), list("files"))
}

pub fn set_sources(desktop: &mut Table, folders: &[PathBuf], files: &[PathBuf]) {
    let p = table_mut(desktop, "pictures");
    let arr = |v: &[PathBuf]| Value::Array(v.iter().map(|x| Value::String(x.display().to_string())).collect());
    p.insert("folders".into(), arr(folders));
    p.insert("files".into(), arr(files));
}

pub fn is_picture(p: &Path) -> bool {
    p.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "svg"))
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')) {
            continue;
        }
        if p.is_dir() && depth < 3 {
            walk(&p, depth + 1, out);
        } else if is_picture(&p) {
            out.push(p);
        }
    }
}

/// Every picture to offer: the system's backgrounds, then the user's folders
/// and files (each group sorted), without repeats.
pub fn pictures(folders: &[PathBuf], files: &[PathBuf]) -> Vec<PathBuf> {
    let mut system = Vec::new();
    for r in ["/usr/local/share/backgrounds", "/usr/share/backgrounds"] {
        walk(Path::new(r), 0, &mut system);
    }
    system.sort();
    let mut own = Vec::new();
    for f in folders {
        walk(f, 0, &mut own);
    }
    own.extend(files.iter().filter(|f| f.is_file()).cloned());
    own.sort();
    let mut out = Vec::new();
    for p in system.into_iter().chain(own) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// A folder's sub-folders and pictures, for the browser (no hidden ones), sorted.
pub fn list_dir(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs = Vec::new();
    let mut pics = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten().take(5000) {
            let p = e.path();
            if p.file_name().and_then(|n| n.to_str()).is_none_or(|n| n.starts_with('.')) {
                continue;
            }
            if p.is_dir() {
                dirs.push(p);
            } else if is_picture(&p) {
                pics.push(p);
            }
        }
    }
    dirs.sort();
    pics.sort();
    (dirs, pics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Table {
        s.parse().unwrap()
    }

    #[test]
    fn one_screen_over_all_screens() {
        let mut d = t("[appearance]\ncolor-scheme = \"dark\"\n[[menu.item]]\nname = \"T\"\nexec = \"foot\"\n[wallpaper]\nimage = \"/a.svg\"\nmode = \"fill\"\ncolor = \"#111111\"\n");
        assert_eq!(shown(&d, Some("m1")), Shown { image: Some("/a.svg".into()), mode: "fill".into(), color: "#111111".into(), own: false });
        set(&mut d, Some("m1"), "ASUS", Some(Path::new("/b.png")), "fit", "#000000");
        assert_eq!(shown(&d, Some("m1")), Shown { image: Some("/b.png".into()), mode: "fit".into(), color: "#000000".into(), own: true });
        // All screens are unchanged, and so is everything else in the file.
        assert_eq!(shown(&d, None).image, Some("/a.svg".into()));
        assert_eq!(shown(&d, Some("m2")).image, Some("/a.svg".into()));
        assert!(d.get("appearance").is_some() && d.get("menu").is_some());
        // Setting it again edits the same entry.
        set(&mut d, Some("m1"), "ASUS", None, "fit", "#000000");
        assert_eq!(monitors(&d).len(), 1);
        assert_eq!(shown(&d, Some("m1")).image, None); // "" = the colour alone
        clear_own(&mut d, "m1");
        assert_eq!(shown(&d, Some("m1")), Shown { image: Some("/a.svg".into()), mode: "fill".into(), color: "#111111".into(), own: false });
        assert!(d["wallpaper"].get("monitor").is_none());
    }

    #[test]
    fn the_file_round_trips_for_the_server() {
        let mut d = Table::new();
        set(&mut d, None, "", Some(Path::new("/a.svg")), "fill", "#111111");
        set(&mut d, Some("m1"), "ASUS", Some(Path::new("/b.png")), "fit", "#000000");
        let text = toml::to_string(&d).unwrap();
        assert!(text.contains("[[wallpaper.monitor]]"), "{text}");
        assert_eq!(shown(&t(&text), Some("m1")).image, Some("/b.png".into()));
    }

    #[test]
    fn sources_round_trip() {
        let mut d = Table::new();
        set_sources(&mut d, &["/p/walls".into()], &["/p/one.jpg".into()]);
        assert_eq!(sources(&d), (vec![PathBuf::from("/p/walls")], vec![PathBuf::from("/p/one.jpg")]));
        assert!(is_picture(Path::new("x.JPG")) && !is_picture(Path::new("x.txt")));
    }
}
