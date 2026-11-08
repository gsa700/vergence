// Vergence settings (prototype, 2026-11-07): wallpaper first. It writes
// ~/.config/vergence/desktop.toml; the server re-reads that within a second and
// draws the wallpaper at every monitor's own pixels (vergence-server wallpaper.rs).
// Appearance (GTK theme, icons, cursor, fonts) will write settings.toml, the
// XSETTINGS file vergd and the server share.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use slint::{Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

slint::slint! {
    import { TabWidget, ScrollView, ComboBox, LineEdit, Button, HorizontalBox, VerticalBox } from "std-widgets.slint";

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
                    Text { text: "Theme, icons, cursor and fonts: next."; }
                }
            }
        }
    }
}

type WallpaperItem = Wallpaper;

const MODES: [&str; 4] = ["fill", "fit", "center", "stretch"];

#[derive(Debug, Default, Serialize, Deserialize)]
struct DesktopFile {
    #[serde(default)]
    wallpaper: WallpaperConf,
}

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
    let current: DesktopFile = std::fs::read_to_string(desktop_path())
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();

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
    if let Some(i) = current.wallpaper.image.as_ref().and_then(|c| files.iter().position(|f| f == c)) {
        ui.set_selected(i as i32);
    }
    ui.set_mode_index(MODES.iter().position(|m| *m == current.wallpaper.mode).unwrap_or(0) as i32);
    ui.set_bg_color(current.wallpaper.color.clone().into());

    let weak = ui.as_weak();
    ui.on_apply(move || {
        let Some(ui) = weak.upgrade() else { return };
        let image = usize::try_from(ui.get_selected()).ok().and_then(|i| files.get(i).cloned());
        let file = DesktopFile {
            wallpaper: WallpaperConf {
                image,
                mode: MODES[ui.get_mode_index().clamp(0, 3) as usize].into(),
                color: ui.get_bg_color().to_string(),
            },
        };
        let path = desktop_path();
        let res = path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|_| std::fs::write(&path, toml::to_string(&file).unwrap_or_default()));
        ui.set_status(match res {
            Ok(()) => format!("Saved {}", path.display()).into(),
            Err(e) => format!("Could not save: {e}").into(),
        });
    });
    ui.run()?;
    Ok(())
}
