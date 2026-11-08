// Vergence settings (prototype, 2026-11-07): wallpaper and appearance. Both
// write ~/.config/vergence/desktop.toml ([wallpaper], [appearance]), keeping
// every other table (the menu's items) as it is. The server re-reads the file
// within a second: the wallpaper is drawn at every monitor's own pixels
// (vergence-server wallpaper.rs), and appearance changes reach every app at
// once through the session bus and its settings portal (settings.rs, dbus.rs).
// Theme, icons, cursor and fonts (settings.toml, the XSETTINGS file) are next.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use slint::{Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

slint::slint! {
    import { TabWidget, ScrollView, ComboBox, LineEdit, Button, CheckBox, HorizontalBox, VerticalBox } from "std-widgets.slint";

    export struct Wallpaper { name: string, thumb: image }

    export component Settings inherits Window {
        title: "Vergence Settings";
        preferred-width: 760px;
        preferred-height: 560px;

        in property <[Wallpaper]> wallpapers;
        in-out property <int> selected: -1;
        in-out property <int> mode-index: 0;
        in-out property <string> bg-color: "#202020";
        in property <string> status;
        callback apply();
        in-out property <int> scheme-index: 0;
        in-out property <string> accent: "";
        in-out property <bool> high-contrast: false;
        in-out property <bool> reduced-motion: false;
        in property <string> appearance-status;
        callback apply-appearance();

        TabWidget {
            Tab {
                title: "Wallpaper";
                VerticalBox {
                    grid := ScrollView {
                        vertical-stretch: 1;
                        property <length> cell-w: 180px;
                        property <length> cell-h: 132px;
                        property <int> cols: max(1, floor(self.visible-width / self.cell-w));
                        content-width: self.visible-width;
                        content-height: ceil(root.wallpapers.length / self.cols) * self.cell-h;
                        for w[i] in root.wallpapers: Rectangle {
                            x: mod(i, grid.cols) * grid.cell-w;
                            y: floor(i / grid.cols) * grid.cell-h;
                            width: grid.cell-w;
                            height: grid.cell-h;
                            Rectangle {
                                x: 6px; y: 6px;
                                width: parent.width - 12px;
                                height: parent.height - 34px;
                                border-radius: 4px;
                                border-width: root.selected == i ? 3px : 1px;
                                border-color: root.selected == i ? #3b82f6 : #8888;
                                background: #00000010;
                                clip: true;
                                Image {
                                    x: 3px; y: 3px;
                                    width: parent.width - 6px;
                                    height: parent.height - 6px;
                                    source: w.thumb;
                                    image-fit: cover;
                                }
                            }
                            Text {
                                x: 6px; y: parent.height - 26px;
                                width: parent.width - 12px;
                                text: w.name;
                                horizontal-alignment: center;
                                overflow: elide;
                                font-size: 11px;
                            }
                            TouchArea {
                                clicked => { root.selected = i; }
                                double-clicked => { root.selected = i; root.apply(); }
                            }
                        }
                    }
                    HorizontalBox {
                        Text { text: "Fit"; vertical-alignment: center; }
                        ComboBox {
                            model: ["Fill", "Fit", "Centre", "Stretch"];
                            current-index <=> root.mode-index;
                        }
                        Text { text: "Colour"; vertical-alignment: center; }
                        LineEdit { text <=> root.bg-color; width: 120px; }
                        Button { text: "Apply"; primary: true; clicked => { root.apply(); } }
                    }
                    Text { text: root.status; color: #888; }
                }
            }
            Tab {
                title: "Appearance";
                VerticalBox {
                    alignment: start;
                    HorizontalBox {
                        Text { text: "Style"; vertical-alignment: center; min-width: 120px; }
                        ComboBox {
                            model: ["Default", "Dark", "Light"];
                            current-index <=> root.scheme-index;
                            selected => { root.apply-appearance(); }
                        }
                    }
                    HorizontalBox {
                        Text { text: "Accent colour"; vertical-alignment: center; min-width: 120px; }
                        LineEdit { text <=> root.accent; placeholder-text: "#3b82f6 (empty: none)"; width: 200px; accepted => { root.apply-appearance(); } }
                    }
                    CheckBox { text: "High contrast"; checked <=> root.high-contrast; toggled => { root.apply-appearance(); } }
                    CheckBox { text: "Reduce motion"; checked <=> root.reduced-motion; toggled => { root.apply-appearance(); } }
                    HorizontalBox {
                        Button { text: "Apply"; primary: true; clicked => { root.apply-appearance(); } }
                    }
                    Text { text: root.appearance-status; color: #888; }
                    Text { text: "Theme, icons, cursor and fonts: next."; color: #888; }
                }
            }
        }
    }
}

type WallpaperItem = Wallpaper;

const MODES: [&str; 4] = ["fill", "fit", "center", "stretch"];


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct WallpaperConf {
    image: Option<PathBuf>,
    mode: String,
    color: String,
}

impl Default for WallpaperConf {
    fn default() -> Self {
        WallpaperConf { image: None, mode: "fill".into(), color: "#202020".into() }
    }
}

fn desktop_path() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/vergence/desktop.toml")
}

/// desktop.toml as a table (empty if missing or unreadable).
fn read_desktop() -> toml::Table {
    std::fs::read_to_string(desktop_path()).ok().and_then(|t| t.parse().ok()).unwrap_or_default()
}

/// Replace one table of desktop.toml, keeping the others (the menu's items).
fn write_desktop_table(name: &str, value: toml::Value) -> Result<PathBuf, String> {
    let path = desktop_path();
    let mut table = match std::fs::read_to_string(&path) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| format!("{} is not valid TOML ({e}); not overwriting it", path.display()))?,
        Err(_) => toml::Table::new(),
    };
    table.insert(name.into(), value);
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Write beside it and rename, so the server never reads half a file.
    let tmp = path.with_extension("toml.new");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())?;
    Ok(path)
}

const SCHEMES: [&str; 3] = ["default", "dark", "light"];

/// Pictures the server can draw, from the usual places.
fn find_wallpapers() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/usr/local/share/backgrounds"), PathBuf::from("/usr/share/backgrounds")];
    if let Some(h) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(h).join("Pictures"));
    }
    let mut out = Vec::new();
    for r in roots {
        walk(&r, 0, &mut out);
    }
    out.sort();
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 3 {
            walk(&p, depth + 1, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
            matches!(x.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "svg")
        }) {
            out.push(p);
        }
    }
}

/// Preview size in pixels: 16:9, dense enough to be sharp at the 14"'s 3.28.
const THUMB_W: u32 = 560;
const THUMB_H: u32 = 315;

/// A w x h preview of a picture, filling (cropped), as the server draws "fill".
fn thumbnail(path: &Path, w: u32, h: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let is_svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let mut canvas = resvg::tiny_skia::Pixmap::new(w, h)?;
    if is_svg {
        let data = std::fs::read(path).ok()?;
        let opt = resvg::usvg::Options { resources_dir: path.parent().map(|p| p.to_path_buf()), ..Default::default() };
        let tree = resvg::usvg::Tree::from_data(&data, &opt).ok()?;
        let (sw, sh) = (tree.size().width(), tree.size().height());
        let s = (w as f32 / sw).max(h as f32 / sh);
        let ts = resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, (w as f32 - sw * s) / 2.0, (h as f32 - sh * s) / 2.0);
        resvg::render(&tree, ts, &mut canvas.as_mut());
    } else {
        let img = image::open(path).ok()?;
        let filled = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
        return Some(SharedPixelBuffer::clone_from_slice(filled.as_raw(), w, h));
    }
    // tiny-skia is premultiplied; Slint wants straight alpha (opaque here anyway).
    let mut px = canvas.take();
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(SharedPixelBuffer::clone_from_slice(&px, w, h))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = find_wallpapers();
    let desktop = read_desktop();
    let wallpaper: WallpaperConf =
        desktop.get("wallpaper").cloned().and_then(|w| w.try_into().ok()).unwrap_or_default();
    let appearance = desktop.get("appearance").and_then(|a| a.as_table()).cloned().unwrap_or_default();

    // The Wayland app_id: the server's placement memory knows windows by it.
    let _ = slint::set_xdg_app_id("vergence-settings");
    let ui = Settings::new()?;
    let items: Vec<WallpaperItem> = files
        .iter()
        .map(|p| WallpaperItem {
            name: SharedString::from(p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            thumb: slint::Image::default(),
        })
        .collect();
    let model = std::rc::Rc::new(VecModel::from(items));
    ui.set_wallpapers(ModelRc::from(model.clone()));
    // Previews, drawn on a thread and shown as they come (the window opens at once).
    {
        let files = files.clone();
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            for (i, f) in files.iter().enumerate() {
                let Some(px) = thumbnail(f, THUMB_W, THUMB_H) else { continue };
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let m = ui.get_wallpapers();
                    if let Some(mut row) = m.row_data(i) {
                        row.thumb = slint::Image::from_rgba8(px);
                        m.set_row_data(i, row);
                    }
                });
            }
        });
    }
    if let Some(i) = wallpaper.image.as_ref().and_then(|c| files.iter().position(|f| f == c)) {
        ui.set_selected(i as i32);
    }
    ui.set_mode_index(MODES.iter().position(|m| *m == wallpaper.mode).unwrap_or(0) as i32);
    ui.set_bg_color(wallpaper.color.clone().into());
    let text = |k: &str| appearance.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    ui.set_scheme_index(SCHEMES.iter().position(|s| *s == text("color-scheme")).unwrap_or(0) as i32);
    ui.set_accent(text("accent").into());
    ui.set_high_contrast(text("contrast") == "high");
    ui.set_reduced_motion(appearance.get("reduced-motion").and_then(|v| v.as_bool()).unwrap_or(false));

    let weak = ui.as_weak();
    ui.on_apply_appearance(move || {
        let Some(ui) = weak.upgrade() else { return };
        let accent = ui.get_accent().trim().to_string();
        let valid = accent.is_empty()
            || (accent.len() == 7 && accent.starts_with('#') && accent[1..].chars().all(|c| c.is_ascii_hexdigit()));
        if !valid {
            ui.set_appearance_status("Accent colour: write it as #rrggbb, or leave it empty".into());
            return;
        }
        let mut t = toml::Table::new();
        t.insert("color-scheme".into(), SCHEMES[ui.get_scheme_index().clamp(0, 2) as usize].into());
        if !accent.is_empty() {
            t.insert("accent".into(), accent.into());
        }
        t.insert("contrast".into(), if ui.get_high_contrast() { "high" } else { "normal" }.into());
        t.insert("reduced-motion".into(), ui.get_reduced_motion().into());
        ui.set_appearance_status(match write_desktop_table("appearance", toml::Value::Table(t)) {
            Ok(p) => format!("Saved {}; apps follow within a second", p.display()).into(),
            Err(e) => format!("Could not save: {e}").into(),
        });
    });

    let weak = ui.as_weak();
    ui.on_apply(move || {
        let Some(ui) = weak.upgrade() else { return };
        let image = usize::try_from(ui.get_selected()).ok().and_then(|i| files.get(i).cloned());
        let conf = WallpaperConf {
            image,
            mode: MODES[ui.get_mode_index().clamp(0, 3) as usize].into(),
            color: ui.get_bg_color().to_string(),
        };
        let res = toml::Value::try_from(&conf).map_err(|e| e.to_string()).and_then(|v| write_desktop_table("wallpaper", v));
        ui.set_status(match res {
            Ok(p) => format!("Saved {}", p.display()).into(),
            Err(e) => format!("Could not save: {e}").into(),
        });
    });
    ui.run()?;
    Ok(())
}
