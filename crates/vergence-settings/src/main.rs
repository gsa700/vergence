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

mod wall;

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use slint::{ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use vergence_protocol::bus::{ClientMsg, OutputInfo, ServerMsg, Topic};

slint::slint! {
    import { ScrollView, ComboBox, LineEdit, Switch, Button, ListView, Palette } from "std-widgets.slint";

    export struct Wallpaper { name: string, thumb: image }
    export struct Source { label: string, kind: string }
    export struct Entry { name: string, folder: bool, picked: bool }
    struct Swatch { name: string, hex: string, color: color }

    // THE COLOURS (Dave, 2026-11-08, from a macOS System Settings screenshot:
    // "the left gray right lighter model. The current window is REALLY white"):
    // a warm grey sidebar the full height of the window, a lighter page beside
    // it, groups as soft grey cards without outlines, the selected page filled
    // with the accent. The same layers in dark greys for the dark style.
    export global Theme {
        // The desktop's accent (desktop.toml [appearance] accent), set from Rust
        // and on every change; without one, the toolkit's own.
        in-out property <bool> has-accent: false;
        in-out property <color> accent-color: #3b82f6;
        out property <color> accent: has-accent ? accent-color : Palette.accent-background;
        out property <color> on-accent: has-accent ? (accent-color.to-hsv().value > 0.85 && accent-color.to-hsv().saturation < 0.5 ? #1d1d1f : #ffffff) : Palette.accent-foreground;
        out property <bool> dark: Palette.color-scheme == ColorScheme.dark;
        out property <color> sidebar: dark ? #262628 : #e7e5e2;
        out property <color> page: dark ? #1c1c1e : #f7f7f6;
        out property <color> card: dark ? #2c2c2e : #ecebea;
        out property <color> line: dark ? #3a3a3c : #d9d8d6;
        out property <color> hover: dark ? #ffffff14 : #0000000d;
        out property <color> dim: dark ? #a1a1a6 : #6e6e73;
    }

    // A line in the sidebar: the page's name; the selected one filled with the
    // accent (macOS), in a heavier weight.
    component SidebarItem inherits Rectangle {
        in property <string> text;
        in property <bool> selected;
        callback clicked;
        height: 30px;
        border-radius: 5px;
        background: root.selected ? Theme.accent : (touch.has-hover ? Theme.hover : transparent);
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
                color: root.selected ? Theme.on-accent : Palette.foreground;
            }
        }
        touch := TouchArea { clicked => { root.clicked(); } }
    }

    // A page's heading and the sentence under it, centred in a card at the top
    // of the page (macOS).
    component PageTitle inherits Rectangle {
        in property <string> title;
        in property <string> description;
        background: Theme.card;
        border-radius: 10px;
        VerticalLayout {
            padding: 18px;
            spacing: 6px;
            Text { text: root.title; font-size: 20px; font-weight: 700; horizontal-alignment: center; color: Palette.foreground; }
            if root.description != "": Text {
                text: root.description;
                font-size: 13px;
                wrap: word-wrap;
                horizontal-alignment: center;
                color: Theme.dim;
            }
        }
    }

    // A group of rows in a rounded box (the macOS format), with an optional heading.
    component Group inherits VerticalLayout {
        in property <string> heading;
        spacing: 6px;
        if root.heading != "": Text { text: root.heading; font-size: 12px; font-weight: 600; color: Theme.dim; }
        Rectangle {
            background: Theme.card;
            border-radius: 10px;
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
        if !root.first: Rectangle { x: 12px; y: 0; width: parent.width - 24px; height: 1px; background: Theme.line; }
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
                // One line, cut with an ellipsis: in a row, a wrapped line's height
                // isn't known, and the text ran out of its box (2026-11-08).
                if root.detail != "": Text { text: root.detail; font-size: 11px; overflow: elide; color: Theme.dim; }
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
        background: Theme.page;

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

        // Wallpaper: the screens (All screens first), the one being edited and
        // whether it has its own; the pictures; the user's folders and files.
        in property <[string]> screens: ["All screens"];
        in-out property <int> screen-index: 0;
        in-out property <bool> own: false;
        callback screen-changed();
        callback own-changed();
        in property <[Wallpaper]> wallpapers;
        in-out property <int> selected: -1;
        in-out property <int> mode-index: 0;
        in-out property <string> bg-color: "#202020";
        in property <string> status;
        callback apply();
        in property <[Source]> sources;
        callback remove-source(int);
        // The picture browser: open while `browsing`; "folder" or "pictures".
        in-out property <bool> browsing: false;
        in property <string> browse-mode;
        in property <string> browse-path;
        in property <[Entry]> entries;
        in property <int> picked-count;
        callback browse(string);
        callback browse-open(int);
        callback browse-up();
        callback browse-add();
        callback browse-cancel();

        // About.
        in property <string> version;
        in property <string> session;

        HorizontalLayout {
            // THE PAGES, as a list: Up and Down move through it and the page follows.
            // The sidebar runs the full height, edge to edge, in its own grey.
            Rectangle {
                width: 200px;
                background: Theme.sidebar;
                Rectangle { x: parent.width - 1px; width: 1px; background: Theme.line; }
            sidebar := FocusScope {
                x: 10px;
                y: 12px;
                width: parent.width - 20px;
                height: parent.height - 24px;
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
            }

            // THE PAGE, on the lighter ground; each page scrolls on its own.
            Rectangle {
                horizontal-stretch: 1;
                background: Theme.page;

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

                if root.page == 1 && !root.browsing: ScrollView {
                    VerticalLayout {
                        padding: 22px;
                        spacing: 16px;
                        alignment: start;
                        PageTitle {
                            title: "Wallpaper";
                            description: "A picture is drawn at every screen's own pixels. Choose one for all screens, or give a screen its own.";
                        }
                        Group {
                            Row {
                                first: true;
                                label: "Screen";
                                detail: root.screen-index == 0 ? "Every screen without its own wallpaper." : (root.own ? "This screen has its own wallpaper." : "This screen shows the one for all screens.");
                                ComboBox {
                                    width: 300px;
                                    model: root.screens;
                                    current-index <=> root.screen-index;
                                    selected => { root.screen-changed(); }
                                }
                            }
                            if root.screen-index > 0: Row {
                                label: "Its own wallpaper";
                                detail: "Off: the same as all screens.";
                                Switch { checked <=> root.own; toggled => { root.own-changed(); } }
                            }
                            Row {
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
                                detail: "Around a picture, or alone without one.";
                                LineEdit { width: 120px; text <=> root.bg-color; accepted => { root.apply(); } }
                            }
                        }
                        grid := Rectangle {
                            property <length> cell-w: 172px;
                            property <length> cell-h: 128px;
                            property <int> cols: Math.max(1, Math.floor(self.width / self.cell-w));
                            height: Math.ceil(root.wallpapers.length / self.cols) * self.cell-h;
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
                                    border-color: root.selected == i ? Theme.accent : Palette.border;
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
                        Group {
                            heading: "Your pictures";
                            for src[i] in root.sources: Row {
                                first: i == 0;
                                label: src.label;
                                detail: src.kind;
                                Button { text: "Remove"; clicked => { root.remove-source(i); } }
                            }
                            Row {
                                first: root.sources.length == 0;
                                label: root.sources.length == 0 ? "Add your own pictures" : "Add more";
                                detail: "Pictures added to a folder later appear too.";
                                Button { text: "Add a folder\u{2026}"; clicked => { root.browse("folder"); } }
                                Button { text: "Add pictures\u{2026}"; clicked => { root.browse("pictures"); } }
                            }
                        }
                        if root.status != "": Text { text: root.status; font-size: 11px; color: Palette.foreground; opacity: 0.6; wrap: word-wrap; }
                    }
                }

                // THE PICTURE BROWSER (our own, 2026-11-08): folders to walk into,
                // pictures to pick; "Add" adds the folder shown, or the pictures picked.
                if root.page == 1 && root.browsing: VerticalLayout {
                    padding: 22px;
                    spacing: 12px;
                    PageTitle {
                        title: root.browse-mode == "folder" ? "Add a folder" : "Add pictures";
                        description: root.browse-mode == "folder" ? "Open the folder you want, then add it. Every picture in it and its folders is offered." : "Click pictures to pick them, in as many folders as you like, then add them.";
                    }
                    HorizontalLayout {
                        spacing: 8px;
                        Button { text: "\u{2191} Up"; clicked => { root.browse-up(); } }
                        Text { text: root.browse-path; vertical-alignment: center; overflow: elide; color: Palette.foreground; horizontal-stretch: 1; }
                    }
                    Rectangle {
                        vertical-stretch: 1;
                        background: Palette.background;
                        border-radius: 8px;
                        border-width: 1px;
                        border-color: Palette.border;
                        ListView {
                            for e[i] in root.entries: Rectangle {
                                height: 30px;
                                background: e.picked ? Palette.selection-background : (ta.has-hover ? Palette.alternate-background : transparent);
                                HorizontalLayout {
                                    padding-left: 12px;
                                    spacing: 8px;
                                    Text { text: e.folder ? "\u{1F4C1}" : (e.picked ? "\u{2713}" : ""); width: 20px; vertical-alignment: center; color: Palette.foreground; }
                                    Text { text: e.name; vertical-alignment: center; overflow: elide; color: e.picked ? Palette.selection-foreground : Palette.foreground; }
                                }
                                ta := TouchArea { clicked => { root.browse-open(i); } }
                            }
                        }
                    }
                    HorizontalLayout {
                        spacing: 8px;
                        alignment: end;
                        Button { text: "Cancel"; clicked => { root.browse-cancel(); } }
                        Button {
                            primary: true;
                            enabled: root.browse-mode == "folder" || root.picked-count > 0;
                            text: root.browse-mode == "folder" ? "Add this folder" : (root.picked-count == 1 ? "Add 1 picture" : "Add " + root.picked-count + " pictures");
                            clicked => { root.browse-add(); }
                        }
                    }
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

fn desktop_path() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/vergence/desktop.toml")
}

/// desktop.toml as a table (empty if missing or unreadable).
fn read_desktop() -> toml::Table {
    std::fs::read_to_string(desktop_path()).ok().and_then(|t| t.parse().ok()).unwrap_or_default()
}

/// Change desktop.toml in place: read it fresh, let `f` edit the table, write
/// it back (beside it, then renamed, so the server never reads half a file).
/// Refuses to overwrite a file it can't parse.
fn update_desktop(f: impl FnOnce(&mut toml::Table)) -> Result<(), String> {
    let path = desktop_path();
    let mut table = match std::fs::read_to_string(&path) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| format!("{} is not valid TOML ({e}); not overwriting it", path.display()))?,
        Err(_) => toml::Table::new(),
    };
    f(&mut table);
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.new");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

const SCHEMES: [&str; 3] = ["default", "dark", "light"];

/// Preview size in pixels: 16:9, dense enough to be sharp at the 14"'s 3.28.
const THUMB_W: u32 = 560;
const THUMB_H: u32 = 315;

/// Previews are kept in ~/.cache/vergence/thumbnails, named by the picture's
/// path, size and time, so they are drawn once (an 8K JPEG takes a second or
/// more to decode) and the page opens with them at once.
fn thumb_cache_path(path: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok()?.hash(&mut h);
    (THUMB_W, THUMB_H).hash(&mut h);
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache/vergence/thumbnails").join(format!("{:016x}.png", h.finish())))
}

/// A picture's preview: from the cache, else drawn and cached.
fn cached_thumbnail(path: &Path) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let cache = thumb_cache_path(path);
    if let Some(c) = &cache
        && let Ok(img) = image::open(c)
    {
        let rgba = img.to_rgba8();
        return Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()));
    }
    let px = thumbnail(path, THUMB_W, THUMB_H)?;
    if let Some(c) = cache {
        let _ = c.parent().map(std::fs::create_dir_all);
        let tmp = c.with_extension("png.new");
        // The format by name: the ".new" name says nothing (saving failed silently).
        if image::save_buffer_with_format(&tmp, px.as_bytes(), px.width(), px.height(), image::ExtendedColorType::Rgba8, image::ImageFormat::Png).is_ok() {
            let _ = std::fs::rename(&tmp, &c);
        }
    }
    Some(px)
}

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

/// A screen as the Screen list names it.
fn screen_label(o: &OutputInfo) -> String {
    let px = |v: i32| (v as f64 * o.scale).round() as i64;
    let desc = if o.description.is_empty() { "Monitor" } else { o.description.as_str() };
    format!("{desc} \u{2014} {}, {}\u{d7}{}", o.name, px(o.width), px(o.height))
}

/// The page's state behind the widgets.
#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    outputs: Vec<OutputInfo>,
    folders: Vec<PathBuf>,
    picked_files: Vec<PathBuf>,
    /// The browser: the folder shown, its sub-folders then pictures, and the
    /// pictures picked so far (in any folder).
    browse_dir: PathBuf,
    browse_entries: Vec<(PathBuf, bool)>,
    browse_picked: Vec<PathBuf>,
}

type Shared = Rc<RefCell<State>>;

/// The screen being edited: None for all screens.
fn screen_id(ui: &Settings, st: &State) -> Option<(String, String)> {
    let i = usize::try_from(ui.get_screen_index()).ok()?.checked_sub(1)?;
    st.outputs.get(i).map(|o| (o.id.clone(), o.description.clone()))
}

/// Show the edited screen's wallpaper (or all screens').
fn show_scope(ui: &Settings, st: &State) {
    let desktop = read_desktop();
    let id = screen_id(ui, st);
    let s = wall::shown(&desktop, id.as_ref().map(|(i, _)| i.as_str()));
    ui.set_own(s.own);
    ui.set_selected(s.image.as_ref().and_then(|c| st.files.iter().position(|f| f == c)).map(|i| i as i32).unwrap_or(-1));
    ui.set_mode_index(wall::MODES.iter().position(|m| *m == s.mode).unwrap_or(0) as i32);
    ui.set_bg_color(s.color.into());
}

/// The pictures on offer (system and the user's), with previews drawn on a
/// thread and shown as they come.
fn load_pictures(ui: &Settings, st: &mut State) {
    let desktop = read_desktop();
    let (folders, files) = wall::sources(&desktop);
    st.files = wall::pictures(&folders, &files);
    st.folders = folders.clone();
    st.picked_files = files.clone();
    let items: Vec<WallpaperItem> = st
        .files
        .iter()
        .map(|p| WallpaperItem {
            name: SharedString::from(p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            thumb: slint::Image::default(),
        })
        .collect();
    ui.set_wallpapers(ModelRc::from(Rc::new(VecModel::from(items))));
    let sources: Vec<Source> = folders
        .iter()
        .map(|f| Source { label: f.display().to_string().into(), kind: "Folder".into() })
        .chain(files.iter().map(|f| Source { label: f.display().to_string().into(), kind: "Picture".into() }))
        .collect();
    ui.set_sources(ModelRc::from(Rc::new(VecModel::from(sources))));
    // Previews on a few threads at once (half the cores, at most 4), each
    // shown as soon as it is ready.
    let list = Arc::new(st.files.clone());
    let next = Arc::new(AtomicUsize::new(0));
    let threads = std::thread::available_parallelism().map(|n| (n.get() / 2).clamp(1, 4)).unwrap_or(2);
    for _ in 0..threads {
        let (list, next, weak) = (list.clone(), next.clone(), ui.as_weak());
        std::thread::spawn(move || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(f) = list.get(i) else { break };
                let Some(px) = cached_thumbnail(f) else { continue };
                let weak = weak.clone();
                let path = f.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let m = ui.get_wallpapers();
                    // Only if the list is still the one this thread was drawing.
                    if let Some(mut row) = m.row_data(i)
                        && row.name.as_str() == path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                    {
                        row.thumb = slint::Image::from_rgba8(px);
                        m.set_row_data(i, row);
                    }
                });
            }
        });
    }
    show_scope(ui, st);
}

fn set_screens(ui: &Settings, st: &mut State, outputs: Vec<OutputInfo>) {
    let current = screen_id(ui, st).map(|(id, _)| id);
    st.outputs = outputs;
    let mut names: Vec<SharedString> = vec!["All screens".into()];
    names.extend(st.outputs.iter().map(|o| SharedString::from(screen_label(o))));
    ui.set_screens(ModelRc::from(Rc::new(VecModel::from(names))));
    // Keep editing the same screen if it is still there.
    let index = current.and_then(|id| st.outputs.iter().position(|o| o.id == id)).map(|i| i as i32 + 1).unwrap_or(0);
    ui.set_screen_index(index);
    show_scope(ui, st);
}

fn show_browser(ui: &Settings, st: &mut State) {
    let (dirs, pics) = wall::list_dir(&st.browse_dir);
    let folder_mode = ui.get_browse_mode() == "folder";
    st.browse_entries = dirs.into_iter().map(|d| (d, true)).chain(pics.into_iter().map(|p| (p, false))).collect();
    let entries: Vec<Entry> = st
        .browse_entries
        .iter()
        .map(|(p, folder)| Entry {
            name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into(),
            folder: *folder,
            picked: !folder && !folder_mode && st.browse_picked.contains(p),
        })
        .collect();
    ui.set_entries(ModelRc::from(Rc::new(VecModel::from(entries))));
    ui.set_browse_path(st.browse_dir.display().to_string().into());
    ui.set_picked_count(st.browse_picked.len() as i32);
}

/// Tell the window the desktop's accent ("#rrggbb", or none).
fn show_accent(ui: &Settings, hex: &str) {
    let rgb = hex
        .strip_prefix('#')
        .filter(|h| h.len() == 6)
        .and_then(|h| u32::from_str_radix(h, 16).ok());
    let theme = ui.global::<Theme>();
    match rgb {
        Some(v) => {
            theme.set_accent_color(slint::Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8));
            theme.set_has_accent(true);
        }
        None => theme.set_has_accent(false),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let desktop = read_desktop();
    let appearance = desktop.get("appearance").and_then(|a| a.as_table()).cloned().unwrap_or_default();

    // The Wayland app_id: the server's placement memory knows windows by it.
    let _ = slint::set_xdg_app_id("vergence-settings");
    let ui = Settings::new()?;
    let st: Shared = Rc::new(RefCell::new(State::default()));
    load_pictures(&ui, &mut st.borrow_mut());

    // The screens, from the session bus (Vergence's own protocol): asked once,
    // then followed, so a monitor plugged in appears in the list. Outside a
    // Vergence session there is no bus, and only All screens is offered.
    {
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let Ok(mut bus) = vergence_protocol::Client::connect("vergence-settings") else { return };
            if bus.request(|serial| ClientMsg::Subscribe { serial, topic: Topic::Outputs }).is_err() {
                return;
            }
            while let Ok(m) = bus.next_event() {
                if let ServerMsg::Outputs { outputs, .. } = m {
                    let weak = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(ui) = weak.upgrade() else { return };
                        if let Some(st) = SHARED.with(|s| s.borrow().clone()) {
                            set_screens(&ui, &mut st.borrow_mut(), outputs);
                        }
                    });
                }
            }
        });
    }
    SHARED.with(|s| *s.borrow_mut() = Some(st.clone()));

    let text = |k: &str| appearance.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    ui.set_scheme_index(SCHEMES.iter().position(|s| *s == text("color-scheme")).unwrap_or(0) as i32);
    ui.set_accent(text("accent").into());
    show_accent(&ui, &text("accent"));
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
            t.insert("accent".into(), accent.clone().into());
        }
        t.insert("contrast".into(), if ui.get_high_contrast() { "high" } else { "normal" }.into());
        t.insert("reduced-motion".into(), ui.get_reduced_motion().into());
        show_accent(&ui, &accent);
        let res = update_desktop(|d| {
            d.insert("appearance".into(), toml::Value::Table(t));
        });
        ui.set_appearance_status(match res {
            Ok(()) => SharedString::new(),
            Err(e) => format!("Could not save: {e}").into(),
        });
    });

    // A picture, fit or colour chosen: for the screen being edited (which then
    // has its own wallpaper), or for all screens.
    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_apply(move || {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let image = usize::try_from(ui.get_selected()).ok().and_then(|i| st.files.get(i).cloned());
        let mode = wall::MODES[ui.get_mode_index().clamp(0, 3) as usize];
        let color = ui.get_bg_color().to_string();
        let id = screen_id(&ui, &st);
        let res = update_desktop(|d| match &id {
            Some((id, label)) => wall::set(d, Some(id), label, image.as_deref(), mode, &color),
            None => wall::set(d, None, "", image.as_deref(), mode, &color),
        });
        ui.set_status(match res {
            Ok(()) => SharedString::new(),
            Err(e) => format!("Could not save: {e}").into(),
        });
        if id.is_some() {
            ui.set_own(true);
        }
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_screen_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        show_scope(&ui, &s2.borrow());
    });

    // Its own wallpaper on: start from what it shows now; off: back to all screens'.
    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_own_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let Some((id, label)) = screen_id(&ui, &st) else { return };
        let on = ui.get_own();
        let shown = wall::shown(&read_desktop(), Some(&id));
        let res = update_desktop(|d| {
            if on {
                wall::set(d, Some(&id), &label, shown.image.as_deref(), &shown.mode, &shown.color);
            } else {
                wall::clear_own(d, &id);
            }
        });
        if let Err(e) = res {
            ui.set_status(format!("Could not save: {e}").into());
        }
        show_scope(&ui, &st);
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_remove_source(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s2.borrow_mut();
        let i = i.max(0) as usize;
        let (mut folders, mut files) = (st.folders.clone(), st.picked_files.clone());
        if i < folders.len() {
            folders.remove(i);
        } else if i - folders.len() < files.len() {
            files.remove(i - folders.len());
        }
        if let Err(e) = update_desktop(|d| wall::set_sources(d, &folders, &files)) {
            ui.set_status(format!("Could not save: {e}").into());
        }
        load_pictures(&ui, &mut st);
    });

    // The browser opens in ~/Pictures (or home).
    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_browse(move |mode| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s2.borrow_mut();
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let pics = home.join("Pictures");
        st.browse_dir = if pics.is_dir() { pics } else { home };
        st.browse_picked.clear();
        ui.set_browse_mode(mode);
        show_browser(&ui, &mut st);
        ui.set_browsing(true);
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_browse_open(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s2.borrow_mut();
        let Some((path, folder)) = st.browse_entries.get(i.max(0) as usize).cloned() else { return };
        if folder {
            st.browse_dir = path;
        } else if ui.get_browse_mode() == "pictures" {
            match st.browse_picked.iter().position(|p| *p == path) {
                Some(k) => {
                    st.browse_picked.remove(k);
                }
                None => st.browse_picked.push(path),
            }
        }
        show_browser(&ui, &mut st);
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_browse_up(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s2.borrow_mut();
        if let Some(parent) = st.browse_dir.parent().map(Path::to_path_buf) {
            st.browse_dir = parent;
            show_browser(&ui, &mut st);
        }
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_browse_add(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s2.borrow_mut();
        let (mut folders, mut files) = (st.folders.clone(), st.picked_files.clone());
        if ui.get_browse_mode() == "folder" {
            if !folders.contains(&st.browse_dir) {
                folders.push(st.browse_dir.clone());
            }
        } else {
            for p in &st.browse_picked {
                if !files.contains(p) {
                    files.push(p.clone());
                }
            }
        }
        if let Err(e) = update_desktop(|d| wall::set_sources(d, &folders, &files)) {
            ui.set_status(format!("Could not save: {e}").into());
        }
        ui.set_browsing(false);
        load_pictures(&ui, &mut st);
    });

    let weak = ui.as_weak();
    ui.on_browse_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_browsing(false);
        }
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

thread_local! {
    /// The page state, for callbacks queued from other threads.
    static SHARED: RefCell<Option<Shared>> = const { RefCell::new(None) };
}
