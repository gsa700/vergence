// Vergence settings (prototype, 2026-11-07): wallpaper first. It writes
// ~/.config/vergence/desktop.toml; the server re-reads that within a second and
// draws the wallpaper at every monitor's own pixels (vergence-server wallpaper.rs).
// Appearance (GTK theme, icons, cursor, fonts) will write settings.toml, the
// XSETTINGS file vergd and the server share.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use slint::{ModelRc, SharedString, StandardListViewItem, VecModel};

slint::slint! {
    import { TabWidget, StandardListView, ComboBox, LineEdit, Button, HorizontalBox, VerticalBox, GroupBox } from "std-widgets.slint";

    export component Settings inherits Window {
        title: "Vergence Settings";
        preferred-width: 640px;
        preferred-height: 460px;

        in property <[StandardListViewItem]> wallpapers;
        in-out property <int> selected: -1;
        in-out property <int> mode-index: 0;
        in-out property <string> bg-color: "#202020";
        in property <string> status;
        callback apply();

        TabWidget {
            Tab {
                title: "Wallpaper";
                VerticalBox {
                    StandardListView {
                        model: root.wallpapers;
                        current-item <=> root.selected;
                        vertical-stretch: 1;
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

const MODES: [&str; 4] = ["fill", "fit", "center", "stretch"];

#[derive(Debug, Default, Serialize, Deserialize)]
struct DesktopFile {
    #[serde(default)]
    wallpaper: Wallpaper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Wallpaper {
    image: Option<PathBuf>,
    mode: String,
    color: String,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Wallpaper { image: None, mode: "fill".into(), color: "#202020".into() }
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = find_wallpapers();
    let current: DesktopFile = std::fs::read_to_string(desktop_path())
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();

    // The Wayland app_id: the server's placement memory knows windows by it.
    let _ = slint::set_xdg_app_id("vergence-settings");
    let ui = Settings::new()?;
    let items: Vec<StandardListViewItem> = files
        .iter()
        .map(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let dir = p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            StandardListViewItem::from(SharedString::from(format!("{name}   ({dir})")))
        })
        .collect();
    ui.set_wallpapers(ModelRc::new(VecModel::from(items)));
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
            wallpaper: Wallpaper {
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
