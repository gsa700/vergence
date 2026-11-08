// Vergence settings (prototype, 2026-11-07): wallpaper and appearance. Both
// write ~/.config/vergence/desktop.toml ([wallpaper], [appearance]), keeping
// every other table (the menu's items) as it is. The server re-reads the file
// within a second: the wallpaper is drawn at every monitor's own pixels
// (vergence-server wallpaper.rs), and appearance changes reach every app at
// once through the session bus and its settings portal (settings.rs, dbus.rs).
// Theme, icons, cursor and fonts (settings.toml, the XSETTINGS file) are next.
//
// THE FORMAT (Dave, 2026-11-08: "similar to the macOS ... designed for desktop
// and not mobiles"; the format of the family's Avalonia Preferences windows):
// a sidebar of pages on the left, About set apart at its foot; the page in a
// framed panel with its title and a sentence; settings in rounded groups, the
// name on the left and the control on the right. Every change applies at once
// (no OK button). A new feature adds a page: a name in `pages` and an
// `if root.page == N` block.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

slint::slint! {
    import { ScrollView, ComboBox, LineEdit, Switch, Palette } from "std-widgets.slint";

    export struct Wallpaper { name: string, thumb: image }
    struct Swatch { name: string, hex: string, color: color }

    // A line in the sidebar: the page's name; the selected one darker than the
    // sidebar, edged and in a heavier weight (the family's Preferences format).
    component SidebarItem inherits Rectangle {
        in property <string> text;
        in property <bool> selected;
        callback clicked;
        height: 30px;
        border-radius: 5px;
        border-width: 1px;
        border-color: root.selected ? Palette.border : transparent;
        background: root.selected ? Palette.alternate-background.darker(0.08) : (touch.has-hover ? Palette.alternate-background.darker(0.04) : transparent);
        accessible-role: list-item;
        accessible-label: root.text;
        HorizontalLayout {
            padding-left: 12px;
            padding-right: 12px;
            Text {
                text: root.text;
                vertical-alignment: center;
                font-size: 13px;
                font-weight: root.selected ? 600 : 400;
                color: Palette.foreground;
            }
        }
        touch := TouchArea { clicked => { root.clicked(); } }
    }

    // A page's heading and the sentence under it.
    component PageTitle inherits VerticalLayout {
        in property <string> title;
        in property <string> description;
        spacing: 6px;
        Text { text: root.title; font-size: 18px; font-weight: 600; color: Palette.foreground; }
        if root.description != "": Text {
            text: root.description;
            font-size: 13px;
            wrap: word-wrap;
            color: Palette.foreground;
            opacity: 0.65;
        }
    }

    // A group of rows in a rounded box (the macOS format), with an optional heading.
    component Group inherits VerticalLayout {
        in property <string> heading;
        spacing: 6px;
        if root.heading != "": Text { text: root.heading; font-size: 12px; font-weight: 600; color: Palette.foreground; opacity: 0.7; }
        Rectangle {
            background: Palette.background;
            border-radius: 8px;
            border-width: 1px;
            border-color: Palette.border;
            VerticalLayout {
                padding: 2px;
                @children
            }
        }
    }

    // One setting: its name (and a line about it) on the left, its control on
    // the right. Every row but a group's first has a hairline above it.
    component Row inherits Rectangle {
        in property <string> label;
        in property <string> detail;
        in property <bool> first: false;
        min-height: 44px;
        if !root.first: Rectangle { x: 12px; y: 0; width: parent.width - 24px; height: 1px; background: Palette.border; opacity: 0.6; }
        HorizontalLayout {
            padding-left: 12px;
            padding-right: 12px;
            padding-top: 6px;
            padding-bottom: 6px;
            spacing: 16px;
            VerticalLayout {
                alignment: center;
                horizontal-stretch: 1;
                Text { text: root.label; font-size: 13px; color: Palette.foreground; }
                if root.detail != "": Text { text: root.detail; font-size: 11px; wrap: word-wrap; color: Palette.foreground; opacity: 0.6; }
            }
            HorizontalLayout {
                alignment: end;
                spacing: 6px;
                @children
            }
        }
    }

    export component Settings inherits Window {
        title: "Vergence Settings";
        preferred-width: 820px;
        preferred-height: 600px;
        min-width: 640px;
        min-height: 480px;
        background: Palette.alternate-background;

        // The pages, in sidebar order; About is the last index, set apart.
        in property <[string]> pages: ["Appearance", "Wallpaper"];
        in-out property <int> page: 0;
        property <int> about: root.pages.length;

        // Appearance.
        in-out property <int> scheme-index: 0;
        in-out property <string> accent: "";
        in-out property <bool> high-contrast: false;
        in-out property <bool> reduced-motion: false;
        in property <string> appearance-status;
        callback apply-appearance();
        property <[Swatch]> swatches: [
            { name: "Blue", hex: "#3b82f6", color: #3b82f6 },
            { name: "Purple", hex: "#8b5cf6", color: #8b5cf6 },
            { name: "Pink", hex: "#ec4899", color: #ec4899 },
            { name: "Red", hex: "#ef4444", color: #ef4444 },
            { name: "Orange", hex: "#e05a00", color: #e05a00 },
            { name: "Yellow", hex: "#eab308", color: #eab308 },
            { name: "Green", hex: "#22c55e", color: #22c55e },
            { name: "Graphite", hex: "#6b7280", color: #6b7280 },
        ];

        // Wallpaper.
        in property <[Wallpaper]> wallpapers;
        in-out property <int> selected: -1;
        in-out property <int> mode-index: 0;
        in-out property <string> bg-color: "#202020";
        in property <string> status;
        callback apply();

        // About.
        in property <string> version;
        in property <string> session;

        HorizontalLayout {
            padding: 12px;
            spacing: 12px;

            // THE PAGES, as a list: Up and Down move through it and the page follows.
            sidebar := FocusScope {
                width: 168px;
                key-pressed(event) => {
                    if (event.text == Key.UpArrow) {
                        root.page = Math.max(0, root.page - 1);
                        return accept;
                    }
                    if (event.text == Key.DownArrow) {
                        root.page = Math.min(root.about, root.page + 1);
                        return accept;
                    }
                    reject
                }
                VerticalLayout {
                    alignment: start;
                    spacing: 2px;
                    for p[i] in root.pages: SidebarItem {
                        text: p;
                        selected: root.page == i;
                        clicked => { root.page = i; sidebar.focus(); }
                    }
                    // About, set apart at the foot: a page about the desktop, not a setting.
                    Rectangle { height: 16px; }
                    SidebarItem {
                        text: "About";
                        selected: root.page == root.about;
                        clicked => { root.page = root.about; sidebar.focus(); }
                    }
                }
            }

            // THE PAGE, in a framed panel; each page scrolls on its own.
            Rectangle {
                horizontal-stretch: 1;
                background: Palette.background.mix(Palette.alternate-background, 0.5);
                border-radius: 6px;
                border-width: 1px;
                border-color: Palette.border;

                if root.page == 0: ScrollView {
                    VerticalLayout {
                        padding: 22px;
                        spacing: 18px;
                        alignment: start;
                        PageTitle {
                            title: "Appearance";
                            description: "Light or dark, the accent colour, contrast and motion. Every app that follows the desktop changes at once.";
                        }
                        Group {
                            Row {
                                first: true;
                                label: "Style";
                                detail: "App default lets each app choose.";
                                ComboBox {
                                    width: 160px;
                                    model: ["App default", "Dark", "Light"];
                                    current-index <=> root.scheme-index;
                                    selected => { root.apply-appearance(); }
                                }
                            }
                            Row {
                                label: "Accent colour";
                                detail: root.accent == "" ? "None: each app uses its own." : root.accent;
                                Rectangle {
                                    width: 22px; height: 22px; border-radius: 11px;
                                    border-width: root.accent == "" ? 2px : 1px;
                                    border-color: root.accent == "" ? Palette.foreground : Palette.border;
                                    background: transparent;
                                    Text { text: "\u{2715}"; font-size: 10px; color: Palette.foreground; opacity: 0.6; horizontal-alignment: center; vertical-alignment: center; }
                                    TouchArea { clicked => { root.accent = ""; root.apply-appearance(); } }
                                }
                                for s in root.swatches: Rectangle {
                                    width: 22px; height: 22px; border-radius: 11px;
                                    background: s.color;
                                    border-width: root.accent == s.hex ? 2px : 0px;
                                    border-color: Palette.foreground;
                                    accessible-role: button;
                                    accessible-label: s.name;
                                    TouchArea { clicked => { root.accent = s.hex; root.apply-appearance(); } }
                                }
                            }
                            Row {
                                label: "Custom accent";
                                detail: "Any colour as #rrggbb; Enter applies it.";
                                LineEdit {
                                    width: 120px;
                                    text <=> root.accent;
                                    placeholder-text: "#rrggbb";
                                    accepted => { root.apply-appearance(); }
                                }
                            }
                        }
                        Group {
                            heading: "Accessibility";
                            Row {
                                first: true;
                                label: "High contrast";
                                detail: "Apps that support it draw stronger edges and text.";
                                Switch { checked <=> root.high-contrast; toggled => { root.apply-appearance(); } }
                            }
                            Row {
                                label: "Reduce motion";
                                detail: "Apps that support it turn off animations.";
                                Switch { checked <=> root.reduced-motion; toggled => { root.apply-appearance(); } }
                            }
                        }
                        if root.appearance-status != "": Text { text: root.appearance-status; font-size: 11px; color: Palette.foreground; opacity: 0.6; wrap: word-wrap; }
                    }
                }

                if root.page == 1: VerticalLayout {
                    padding: 22px;
                    spacing: 14px;
                    PageTitle {
                        title: "Wallpaper";
                        description: "A picture is drawn at every monitor's own pixels. Click one to use it.";
                    }
                    Group {
                        Row {
                            first: true;
                            label: "Fit";
                            ComboBox {
                                width: 140px;
                                model: ["Fill", "Fit", "Centre", "Stretch"];
                                current-index <=> root.mode-index;
                                selected => { root.apply(); }
                            }
                        }
                        Row {
                            label: "Background colour";
                            detail: "Around a picture that doesn't fill the screen, or alone without one.";
                            LineEdit { width: 120px; text <=> root.bg-color; accepted => { root.apply(); } }
                        }
                    }
                    grid := ScrollView {
                        vertical-stretch: 1;
                        property <length> cell-w: 180px;
                        property <length> cell-h: 132px;
                        property <int> cols: Math.max(1, Math.floor(self.visible-width / self.cell-w));
                        content-width: self.visible-width;
                        content-height: Math.ceil(root.wallpapers.length / self.cols) * self.cell-h;
                        for w[i] in root.wallpapers: Rectangle {
                            x: Math.mod(i, grid.cols) * grid.cell-w;
                            y: Math.floor(i / grid.cols) * grid.cell-h;
                            width: grid.cell-w;
                            height: grid.cell-h;
                            Rectangle {
                                x: 6px; y: 6px;
                                width: parent.width - 12px;
                                height: parent.height - 34px;
                                border-radius: 6px;
                                border-width: root.selected == i ? 3px : 1px;
                                border-color: root.selected == i ? Palette.accent-background : Palette.border;
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
                                color: Palette.foreground;
                            }
                            TouchArea { clicked => { root.selected = i; root.apply(); } }
                        }
                    }
                    if root.status != "": Text { text: root.status; font-size: 11px; color: Palette.foreground; opacity: 0.6; }
                }

                if root.page == root.about: VerticalLayout {
                    padding: 22px;
                    spacing: 18px;
                    alignment: start;
                    PageTitle {
                        title: "About";
                        description: "Vergence: every app the right size, in the right place and readable, on any monitor setup.";
                    }
                    Group {
                        Row { first: true; label: "Settings app"; Text { text: root.version; color: Palette.foreground; vertical-alignment: center; } }
                        Row { label: "Session"; Text { text: root.session; color: Palette.foreground; vertical-alignment: center; } }
                    }
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
            Ok(_) => SharedString::new(),
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
            Ok(_) => SharedString::new(),
            Err(e) => format!("Could not save: {e}").into(),
        });
    });
    ui.set_version(format!("version {}", env!("CARGO_PKG_VERSION")).into());
    let session = match (std::env::var("VERGENCE_BUS"), std::env::var("WAYLAND_DISPLAY")) {
        (Ok(_), Ok(w)) => format!("Vergence on {w}"),
        (_, Ok(w)) => format!("another Wayland desktop ({w})"),
        _ => "an X11 desktop".to_string(),
    };
    ui.set_session(session.into());
    ui.run()?;
    Ok(())
}
