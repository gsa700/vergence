// The Theme and fonts page's data (2026-11-08): what is installed, and the
// desktop's choices in ~/.config/vergence/settings.toml (the XSETTINGS file
// vergd and the server share; the server republishes it to apps live and
// passes the same values on through the settings portal).
//
//     Net/ThemeName          applications (GTK) theme
//     Net/IconThemeName      icons
//     Gtk/CursorThemeName    pointer
//     Gtk/FontName           interface font, "Family Size"
//     Gtk/MonospaceFontName  monospace font, "Family Size"

use std::path::{Path, PathBuf};

use vergence_xsettings::{Settings, Value};

pub const THEME: &str = "Net/ThemeName";
pub const ICONS: &str = "Net/IconThemeName";
pub const CURSOR: &str = "Gtk/CursorThemeName";
pub const FONT: &str = "Gtk/FontName";
pub const MONO: &str = "Gtk/MonospaceFontName";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn settings_path() -> PathBuf {
    home().join(".config/vergence/settings.toml")
}

pub fn read() -> Settings {
    std::fs::read_to_string(settings_path()).ok().and_then(|t| vergence_xsettings::from_toml(&t).ok()).unwrap_or_default()
}

pub fn get(s: &Settings, key: &str) -> String {
    match s.get(key) {
        Some(Value::Str(v)) => v.clone(),
        _ => String::new(),
    }
}

/// Set one key and write the file (beside it, then renamed); everything else
/// in it stays.
pub fn set(key: &str, value: &str) -> Result<(), String> {
    let path = settings_path();
    let mut s = match std::fs::read_to_string(&path) {
        Ok(t) => vergence_xsettings::from_toml(&t).map_err(|e| format!("{}: {e}; not overwriting it", path.display()))?,
        Err(_) => Settings::default(),
    };
    s.set(key, Value::Str(value.to_string()));
    let text = format!(
        "# Vergence XSETTINGS: the desktop's theme, icons, fonts and pointer (and what\n# vergd publishes on Xorg). Written by the settings app's Theme page; edit\n# freely: the server applies changes within a second.\n\n{}",
        vergence_xsettings::to_toml(&s)
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.new");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

fn theme_dirs(kind: &str) -> Vec<PathBuf> {
    let h = home();
    match kind {
        "themes" => vec![h.join(".themes"), h.join(".local/share/themes"), "/usr/local/share/themes".into(), "/usr/share/themes".into()],
        _ => vec![h.join(".icons"), h.join(".local/share/icons"), "/usr/local/share/icons".into(), "/usr/share/icons".into()],
    }
}

/// Names of the subfolders of `dirs` for which `keep` holds, sorted, no repeats.
fn names(dirs: &[PathBuf], keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() && keep(&p)
                && let Some(n) = p.file_name().and_then(|n| n.to_str())
                && !out.iter().any(|x| x == n)
            {
                out.push(n.to_string());
            }
        }
    }
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Applications themes: those with a GTK 3 part.
pub fn gtk_themes() -> Vec<String> {
    names(&theme_dirs("themes"), |p| p.join("gtk-3.0").is_dir())
}

/// Icon themes: an index.theme that names icon folders (not just a pointer theme).
pub fn icon_themes() -> Vec<String> {
    names(&theme_dirs("icons"), |p| {
        std::fs::read_to_string(p.join("index.theme")).is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("Directories=")))
    })
}

/// Pointer themes: a cursors folder.
pub fn cursor_themes() -> Vec<String> {
    names(&theme_dirs("icons"), |p| p.join("cursors").is_dir())
}

/// Font families from fontconfig (`mono`: monospaced ones only), sorted.
pub fn font_families(mono: bool) -> Vec<String> {
    let pattern = if mono { ":spacing=mono" } else { ":" };
    let out = std::process::Command::new("fc-list").args([pattern, "family"]).output();
    let mut families: Vec<String> = out
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
        .lines()
        // "Noto Sans,Noto Sans Regular" -> the first name.
        .filter_map(|l| l.split(',').next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    families.sort_by_key(|n| n.to_lowercase());
    families.dedup();
    families
}

/// "Noto Sans Bold 10.5" -> ("Noto Sans Bold", 10.5); no size -> 10.
pub fn split_font(name: &str) -> (String, f32) {
    let name = name.trim();
    match name.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f32>().is_ok_and(|s| s > 0.0) => (family.to_string(), size.parse().unwrap()),
        _ => (name.to_string(), 10.0),
    }
}

pub fn join_font(family: &str, size: f32) -> String {
    if size.fract() == 0.0 { format!("{family} {}", size as i32) } else { format!("{family} {size}") }
}

/// The sizes offered, with `current` among them.
pub fn sizes(current: f32) -> Vec<f32> {
    let mut s: Vec<f32> = vec![8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 18.0, 20.0];
    if !s.contains(&current) {
        s.push(current);
        s.sort_by(f32::total_cmp);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_names() {
        assert_eq!(split_font("Sans 10"), ("Sans".into(), 10.0));
        assert_eq!(split_font("Noto Sans Bold 10.5"), ("Noto Sans Bold".into(), 10.5));
        assert_eq!(split_font("Cantarell"), ("Cantarell".into(), 10.0));
        assert_eq!(join_font("Noto Sans", 11.0), "Noto Sans 11");
        assert_eq!(join_font("Noto Sans", 10.5), "Noto Sans 10.5");
        assert!(sizes(10.5).contains(&10.5));
        assert_eq!(sizes(10.0).len(), 10);
    }
}
