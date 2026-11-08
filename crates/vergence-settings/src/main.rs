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

mod disp;
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
    import { ScrollView, ComboBox, LineEdit, Switch, Button, ListView, Slider, Palette } from "std-widgets.slint";

    export struct Wallpaper { name: string, thumb: image }
    export struct Source { label: string, kind: string }
    export struct Entry { name: string, folder: bool, picked: bool }
    // A monitor in the little arrangement: its name, its place and size as
    // fractions of the whole arrangement, its wallpaper's preview.
    export struct ScreenTile { label: string, fx: float, fy: float, fw: float, fh: float, thumb: image, own: bool, scale: string }
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
        // A screen nestled under Screens: smaller and indented.
        in property <bool> child: false;
        callback clicked;
        height: root.child ? 26px : 30px;
        border-radius: 5px;
        background: root.selected ? Theme.accent : (touch.has-hover ? Theme.hover : transparent);
        accessible-role: list-item;
        accessible-label: root.text;
        HorizontalLayout {
            padding-left: root.child ? 26px : 12px;
            padding-right: 12px;
            Text {
                text: root.text;
                vertical-alignment: center;
                overflow: elide;
                font-size: root.child ? 12px : 13px;
                font-weight: root.selected ? 600 : 400;
                color: root.selected ? Theme.on-accent : Palette.foreground;
            }
        }
        touch := TouchArea { clicked => { root.clicked(); } }
    }

    // THE SIZE SLIDER, 75% to 150%, with a detent at 100% (Dave, 2026-11-08: "so
    // it's easy to return to"): within 3.5% of it, it sticks to exactly 100%; a
    // tick marks the spot.
    component SizeSlider inherits Rectangle {
        in-out property <float> value: 1.0;
        callback released();
        width: 220px;
        height: 34px;
        Slider {
            width: parent.width;
            minimum: 0.75;
            maximum: 1.5;
            value <=> root.value;
            changed(v) => {
                if (v != 1.0 && Math.abs(v - 1.0) < 0.035) {
                    root.value = 1.0;
                }
            }
            released => { root.released(); }
        }
        // The tick under 100% (the handle travels inset by about its radius).
        Rectangle {
            x: 10px + (parent.width - 20px) * (1.0 - 0.75) / (1.5 - 0.75) - 1px;
            y: parent.height - 5px;
            width: 2px;
            height: 5px;
            background: root.value == 1.0 ? Theme.accent : Theme.dim;
        }
    }

    // A SCREEN'S SCALE SLIDER, 0.75 to its maximum, with marks at the whole
    // numbers (sharp in every app) and the default; Rust makes it stick to a
    // mark it comes near.
    component ScaleSlider inherits Rectangle {
        in-out property <float> value: 1.0;
        in property <float> maximum: 3.0;
        in property <[float]> marks;
        in property <float> default: 1.0;
        callback moved(float);
        callback released();
        width: 260px;
        height: 34px;
        property <float> minimum: 0.75;
        Slider {
            width: parent.width;
            minimum: root.minimum;
            maximum: root.maximum;
            value <=> root.value;
            changed(v) => { root.moved(v); }
            released => { root.released(); }
        }
        for m in root.marks: Rectangle {
            x: 10px + (parent.width - 20px) * (m - root.minimum) / (root.maximum - root.minimum) - 1px;
            y: parent.height - (m == root.default ? 7px : 5px);
            width: 2px;
            height: m == root.default ? 7px : 5px;
            background: root.value == m ? Theme.accent : Theme.dim;
        }
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

        // The pages, in sidebar order: Appearance (0), one per screen (1..n;
        // "Wallpaper" alone outside a Vergence session), About last, set apart.
        // (Dave, 2026-11-08: "making each Screen its own entry in the left menu
        // ... if one monitor, then one settings tab".)
        in property <[string]> screen-pages: ["Wallpaper"];
        // "Screens" in a Vergence session, "Wallpaper" outside one; the page
        // Screens opens (the primary's).
        in property <string> screens-title: "Wallpaper";
        in property <int> primary-page: 1;
        // The Screens overview (several screens): the arrangement and the size.
        out property <int> overview: 100;
        callback arrange-dropped(int, float, float);
        in-out property <float> size: 1.0;
        callback set-size();
        // A screen's Display section: the scale it is shown at and how it was
        // found; its own scale on a slider (the default: the sharpest near
        // the calculated one) with marks at the whole numbers and the default.
        in property <string> scale-text;
        in property <string> scale-note;
        in-out property <float> base-scale: 1.0;
        in property <float> default-scale: 1.0;
        in property <float> scale-max: 3.0;
        in property <[float]> detents: [1.0, 2.0, 3.0];
        in property <bool> chosen: false;
        callback scale-moved(float);
        callback set-scale();
        callback reset-scale();
        in-out property <int> page: 0;
        property <int> about: root.screen-pages.length + 1;
        callback page-changed();

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

        // A screen's page: the little map of the arrangement; the screen's name;
        // with several screens, whether it is the primary (whose wallpaper is
        // everyone's default) and, if not, whether it follows the primary.
        in property <[ScreenTile]> tiles;
        // The whole arrangement's width over its height.
        in property <float> arrangement-aspect: 2.0;
        in property <string> screen-label: "Wallpaper";
        in property <bool> multi: false;
        in property <bool> is-primary: true;
        in property <string> primary-label;
        in-out property <bool> same-as-primary: true;
        callback own-changed();
        callback make-primary();
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
                        root.page-changed();
                        return accept;
                    }
                    if (event.text == Key.DownArrow) {
                        root.page = Math.min(root.about, root.page + 1);
                        root.page-changed();
                        return accept;
                    }
                    reject
                }
                VerticalLayout {
                    alignment: start;
                    spacing: 2px;
                    SidebarItem {
                        text: "Appearance";
                        selected: root.page == 0;
                        clicked => { root.page = 0; root.page-changed(); sidebar.focus(); }
                    }
                    // SCREENS leads; with several, each screen is nestled under it,
                    // smaller (Dave, 2026-11-08). Screens itself opens the primary's page.
                    SidebarItem {
                        text: root.screens-title;
                        selected: root.screen-pages.length > 1 ? root.page == root.overview : root.page == 1;
                        clicked => { root.page = root.screen-pages.length > 1 ? root.overview : 1; root.page-changed(); sidebar.focus(); }
                    }
                    if root.screen-pages.length > 1: VerticalLayout {
                        spacing: 2px;
                        for name[i] in root.screen-pages: SidebarItem {
                            child: true;
                            text: name;
                            selected: root.page == i + 1;
                            clicked => { root.page = i + 1; root.page-changed(); sidebar.focus(); }
                        }
                    }
                    // About, set apart at the foot: a page about the desktop, not a setting.
                    Rectangle { height: 16px; }
                    SidebarItem {
                        text: "About";
                        selected: root.page == root.about;
                        clicked => { root.page = root.about; root.page-changed(); sidebar.focus(); }
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

                if root.page >= 1 && root.page <= root.screen-pages.length && !root.browsing: ScrollView {
                    VerticalLayout {
                        padding: 22px;
                        spacing: 16px;
                        alignment: start;
                        PageTitle {
                            title: root.screen-label;
                            description: !root.multi ? "Its wallpaper, drawn at the screen's own pixels." : (root.is-primary ? "The primary screen: its wallpaper is the default, shown on every other screen that has none of its own." : "Shows the primary screen's wallpaper (" + root.primary-label + ") unless it has its own.");
                        }
                        // Where this screen is: the arrangement, this one framed in the
                        // accent; a click on another goes to its page.
                        if root.multi: Group {
                            arr := Rectangle {
                                height: 110px;
                                property <length> aw: Math.min(self.width - 24px, (self.height - 20px) * root.arrangement-aspect);
                                property <length> ah: self.aw / Math.max(0.1, root.arrangement-aspect);
                                property <length> ox: (self.width - self.aw) / 2;
                                property <length> oy: (self.height - self.ah) / 2;
                                for t[i] in root.tiles: Rectangle {
                                    x: arr.ox + t.fx * arr.aw + 3px;
                                    y: arr.oy + t.fy * arr.ah + 3px;
                                    width: t.fw * arr.aw - 6px;
                                    height: t.fh * arr.ah - 6px;
                                    border-radius: 5px;
                                    clip: true;
                                    background: #000000;
                                    accessible-role: button;
                                    accessible-label: t.label;
                                    Image { width: parent.width; height: parent.height; source: t.thumb; image-fit: cover; }
                                    Rectangle {
                                        y: parent.height - 20px;
                                        height: 20px;
                                        background: #000000a8;
                                        Text {
                                            width: parent.width - 8px;
                                            text: t.label;
                                            color: #ffffff;
                                            font-size: 10px;
                                            font-weight: 600;
                                            horizontal-alignment: center;
                                            vertical-alignment: center;
                                            overflow: elide;
                                        }
                                    }
                                    Rectangle {
                                        border-radius: 5px;
                                        border-width: root.page == i + 1 ? 3px : 1px;
                                        border-color: root.page == i + 1 ? Theme.accent : (tta.has-hover ? Palette.foreground : Theme.line);
                                    }
                                    tta := TouchArea { clicked => { root.page = i + 1; root.page-changed(); } }
                                }
                            }
                        }
                        // DISPLAY: the scale, from the screen's size and viewing distance
                        // (and the size preference), or set by hand.
                        Group {
                            heading: "Display";
                            Row {
                                first: true;
                                label: "Scale";
                                detail: root.base-scale == root.default-scale ? "The default: the sharpest for this screen." : (Math.round(root.base-scale) == root.base-scale ? "Whole number: sharp in every app." : "In between: soft in some older apps.");
                                ScaleSlider {
                                    value <=> root.base-scale;
                                    maximum: root.scale-max;
                                    marks: root.detents;
                                    default: root.default-scale;
                                    moved(v) => { root.scale-moved(v); }
                                    released => { root.set-scale(); }
                                }
                                Text { text: Math.round(root.base-scale * 100) + "%"; width: 44px; horizontal-alignment: right; vertical-alignment: center; color: Palette.foreground; font-weight: 600; }
                            }
                            Row {
                                label: "Shown at";
                                detail: root.scale-note;
                                if root.chosen: Button { text: "Default"; clicked => { root.reset-scale(); } }
                                Text { text: root.scale-text; color: Theme.dim; vertical-alignment: center; }
                            }
                            if !root.multi: Row {
                                label: "Size";
                                detail: "How big everything is: " + Math.round(root.size * 100) + "%.";
                                SizeSlider { value <=> root.size; released => { root.set-size(); } }
                            }
                        }
                        Group {
                            heading: "Wallpaper";
                            if root.multi && !root.is-primary: Row {
                                first: true;
                                label: "Same as the primary screen";
                                detail: root.same-as-primary ? "Shows " + root.primary-label + "'s wallpaper." : "Off: this screen has its own.";
                                Switch { checked <=> root.same-as-primary; toggled => { root.own-changed(); } }
                            }
                            if root.multi: Row {
                                first: root.is-primary;
                                label: "Primary screen";
                                detail: root.is-primary ? "Its wallpaper is the default for the others." : "Make this the primary: its wallpaper becomes the default.";
                                if root.is-primary: Text { text: "Primary"; color: Theme.dim; vertical-alignment: center; }
                                if !root.is-primary: Button { text: "Make primary"; clicked => { root.make-primary(); } }
                            }
                            Row {
                                first: !root.multi;
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

                // THE SCREENS OVERVIEW (several screens): drag them into place; the
                // one size preference; each screen's scale.
                if root.page == root.overview: ScrollView {
                    VerticalLayout {
                        padding: 22px;
                        spacing: 16px;
                        alignment: start;
                        PageTitle {
                            title: "Screens";
                            description: "Drag the screens to where they sit on your desk. Each one's scale is worked out from its size and how far away it is.";
                        }
                        Group {
                            canvas := Rectangle {
                                height: 240px;
                                property <length> aw: Math.min(self.width - 60px, (self.height - 60px) * root.arrangement-aspect);
                                property <length> ah: self.aw / Math.max(0.1, root.arrangement-aspect);
                                property <length> ox: (self.width - self.aw) / 2;
                                property <length> oy: (self.height - self.ah) / 2;
                                for t[i] in root.tiles: Rectangle {
                                    property <length> dx: 0;
                                    property <length> dy: 0;
                                    x: canvas.ox + t.fx * canvas.aw + 3px + self.dx;
                                    y: canvas.oy + t.fy * canvas.ah + 3px + self.dy;
                                    width: t.fw * canvas.aw - 6px;
                                    height: t.fh * canvas.ah - 6px;
                                    z: drag.pressed ? 1 : 0;
                                    border-radius: 6px;
                                    clip: true;
                                    background: #000000;
                                    accessible-role: button;
                                    accessible-label: t.label;
                                    Image { width: parent.width; height: parent.height; source: t.thumb; image-fit: cover; }
                                    Rectangle {
                                        y: parent.height - 34px;
                                        height: 34px;
                                        background: #000000a8;
                                        VerticalLayout {
                                            alignment: center;
                                            Text { text: t.label; color: #ffffff; font-size: 11px; font-weight: 600; horizontal-alignment: center; overflow: elide; }
                                            Text { text: t.scale; color: #ffffffc0; font-size: 10px; horizontal-alignment: center; }
                                        }
                                    }
                                    Rectangle {
                                        border-radius: 6px;
                                        border-width: drag.pressed ? 3px : 1px;
                                        border-color: drag.pressed ? Theme.accent : (drag.has-hover ? Palette.foreground : Theme.line);
                                    }
                                    drag := TouchArea {
                                        mouse-cursor: self.pressed ? MouseCursor.grabbing : MouseCursor.grab;
                                        moved => {
                                            parent.dx = self.mouse-x - self.pressed-x;
                                            parent.dy = self.mouse-y - self.pressed-y;
                                        }
                                        pointer-event(e) => {
                                            if (e.kind == PointerEventKind.up && (parent.dx != 0 || parent.dy != 0)) {
                                                root.arrange-dropped(i, t.fx + parent.dx / canvas.aw, t.fy + parent.dy / canvas.ah);
                                                parent.dx = 0;
                                                parent.dy = 0;
                                            }
                                        }
                                        double-clicked => { root.page = i + 1; root.page-changed(); }
                                    }
                                }
                            }
                        }
                        Text { text: "Double-click a screen for its own settings."; font-size: 11px; color: Theme.dim; }
                        Group {
                            Row {
                                first: true;
                                label: "Size";
                                detail: "How big everything is, on every screen: " + Math.round(root.size * 100) + "%.";
                                SizeSlider { value <=> root.size; released => { root.set-size(); } }
                            }
                        }
                    }
                }

                // THE PICTURE BROWSER (our own, 2026-11-08): folders to walk into,
                // pictures to pick; "Add" adds the folder shown, or the pictures picked.
                if root.browsing && root.page >= 1 && root.page <= root.screen-pages.length: VerticalLayout {
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

/// A screen's name on its tile: the EDID's name, or the connector when the
/// EDID only names the port type ("HDMI" on the 14", 2026-11-08).
fn screen_label(o: &OutputInfo) -> String {
    let port_type = o.name.split('-').next().unwrap_or("");
    if o.description.is_empty() || o.description == port_type {
        o.name.clone()
    } else {
        o.description.clone()
    }
}

/// A picture's preview if it is already in the cache (never drawn here).
fn thumbnail_if_cached(path: &Path) -> Option<slint::Image> {
    let img = image::open(thumb_cache_path(path)?).ok()?.to_rgba8();
    Some(slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height())))
}

/// The arrangement: each monitor's tile with its wallpaper's preview. A
/// preview not yet drawn is drawn on a thread, then the tiles are refreshed.
fn refresh_tiles(ui: &Settings, st: &State) {
    let desktop = read_desktop();
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for o in &st.outputs {
        x0 = x0.min(o.x);
        y0 = y0.min(o.y);
        x1 = x1.max(o.x + o.width);
        y1 = y1.max(o.y + o.height);
    }
    let (bw, bh) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
    let mut missing = Vec::new();
    let tiles: Vec<ScreenTile> = st
        .outputs
        .iter()
        .map(|o| {
            let shown = wall::shown(&desktop, Some(&o.id));
            let thumb = shown.image.as_deref().and_then(|p| {
                let t = thumbnail_if_cached(p);
                // Each picture is tried once: one that can't be drawn stays blank
                // instead of being tried again forever.
                if t.is_none() && TRIED.with(|tried| tried.borrow_mut().insert(p.to_path_buf())) {
                    missing.push(p.to_path_buf());
                }
                t
            });
            ScreenTile {
                scale: format!("{:.0}%", o.scale * 100.0).into(),
                label: screen_label(o).into(),
                fx: (o.x - x0) as f32 / bw,
                fy: (o.y - y0) as f32 / bh,
                fw: o.width as f32 / bw,
                fh: o.height as f32 / bh,
                thumb: thumb.unwrap_or_default(),
                own: shown.own,
            }
        })
        .collect();
    ui.set_arrangement_aspect(if st.outputs.is_empty() { 2.0 } else { bw / bh });
    ui.set_tiles(ModelRc::from(Rc::new(VecModel::from(tiles))));
    if !missing.is_empty() {
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            for p in missing {
                let _ = cached_thumbnail(&p);
            }
            let _ = slint::invoke_from_event_loop(move || {
                if let (Some(ui), Some(st)) = (weak.upgrade(), SHARED.with(|s| s.borrow().clone())) {
                    // Only once: drawn now, so this refresh finds them all.
                    refresh_tiles(&ui, &st.borrow());
                }
            });
        });
    }
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
    /// The monitor setup the screens make (from the bus): arrangements are kept per setup.
    setup: String,
}

type Shared = Rc<RefCell<State>>;

/// The screen whose page is open, if any.
fn current_output<'a>(ui: &Settings, st: &'a State) -> Option<&'a OutputInfo> {
    let i = usize::try_from(ui.get_page()).ok()?.checked_sub(1)?;
    st.outputs.get(i)
}

/// The primary screen's id.
fn primary_of(desktop: &toml::Table, st: &State) -> Option<String> {
    let ids: Vec<String> = st.outputs.iter().map(|o| o.id.clone()).collect();
    wall::primary(desktop, &ids)
}

/// Where the open page's wallpaper is kept: None for the primary (and the
/// only) screen, whose wallpaper is the default; else (id, label) of its own.
fn screen_id(ui: &Settings, st: &State) -> Option<(String, String)> {
    let o = current_output(ui, st)?;
    if primary_of(&read_desktop(), st).as_deref() == Some(o.id.as_str()) {
        return None;
    }
    Some((o.id.clone(), screen_label(o)))
}

/// Show the edited screen's wallpaper (or all screens').
fn show_scope(ui: &Settings, st: &State) {
    let desktop = read_desktop();
    let id = screen_id(ui, st);
    let s = wall::shown(&desktop, id.as_ref().map(|(i, _)| i.as_str()));
    ui.set_same_as_primary(!s.own);
    let primary = primary_of(&desktop, st);
    ui.set_multi(st.outputs.len() > 1);
    ui.set_is_primary(id.is_none());
    ui.set_primary_label(st.outputs.iter().find(|o| Some(&o.id) == primary.as_ref()).map(screen_label).unwrap_or_default().into());
    let primary_index = st.outputs.iter().position(|o| Some(&o.id) == primary.as_ref()).unwrap_or(0);
    ui.set_primary_page(primary_index as i32 + 1);
    ui.set_screen_label(current_output(ui, st).map(screen_label).unwrap_or_else(|| "Wallpaper".into()).into());
    // The Display section.
    ui.set_size(disp::size(&desktop) as f32);
    if let Some(o) = current_output(ui, st) {
        let (_, chosen) = disp::screen(&desktop, &o.id);
        let max = (o.default_scale.ceil() + 2.0).max(3.0);
        ui.set_scale_text(format!("{:.0}% ({:.4})", o.scale * 100.0, o.scale).into());
        ui.set_scale_note(o.note.clone().into());
        ui.set_base_scale(o.base_scale as f32);
        ui.set_default_scale(o.default_scale as f32);
        ui.set_scale_max(max as f32);
        ui.set_detents(ModelRc::from(Rc::new(VecModel::from(disp::detents(o.default_scale, max).into_iter().map(|d| d as f32).collect::<Vec<_>>()))));
        ui.set_chosen(chosen.is_some());
    } else {
        ui.set_scale_text("".into());
        ui.set_scale_note("Outside a Vergence session the desktop decides.".into());
    }
    ui.set_selected(s.image.as_ref().and_then(|c| st.files.iter().position(|f| f == c)).map(|i| i as i32).unwrap_or(-1));
    ui.set_mode_index(wall::MODES.iter().position(|m| *m == s.mode).unwrap_or(0) as i32);
    ui.set_bg_color(s.color.into());
    refresh_tiles(ui, st);
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

/// The screens changed (the bus's outputs): one sidebar page each, named so
/// that identical monitors are told apart by their port.
fn set_screens(ui: &Settings, st: &mut State, outputs: Vec<OutputInfo>, setup: String) {
    st.setup = setup;
    let page = ui.get_page();
    let old_count = st.outputs.len().max(1) as i32;
    let on_screen = page >= 1 && page <= old_count;
    if page == ui.get_overview() && outputs.len() <= 1 {
        // Down to one screen: its page is the Screens page.
        ui.set_page(1);
    }
    let current = current_output(ui, st).map(|o| o.id.clone());
    st.outputs = outputs;
    let labels: Vec<String> = st.outputs.iter().map(screen_label).collect();
    let names: Vec<SharedString> = if st.outputs.is_empty() {
        vec!["Wallpaper".into()]
    } else {
        st.outputs
            .iter()
            .zip(&labels)
            .map(|(o, l)| {
                let twins = labels.iter().filter(|x| *x == l).count() > 1;
                SharedString::from(if twins && *l != o.name { format!("{l} \u{2014} {}", o.name) } else { l.clone() })
            })
            .collect()
    };
    let count = names.len() as i32;
    ui.set_screen_pages(ModelRc::from(Rc::new(VecModel::from(names))));
    ui.set_screens_title(if st.outputs.is_empty() { "Wallpaper" } else { "Screens" }.into());
    // The primary's wallpaper is the default: if it has an entry of its own
    // (from before there was a primary), that becomes the default instead,
    // which changes nothing on screen.
    let desktop = read_desktop();
    if let Some(p) = primary_of(&desktop, st)
        && wall::shown(&desktop, Some(&p)).own
        && let Err(e) = update_desktop(|d| wall::make_primary(d, None, &p))
    {
        ui.set_status(format!("Could not save: {e}").into());
    }
    if on_screen {
        // The same screen's page if it is still there, else the primary's.
        let desktop = read_desktop();
        let want = current.or_else(|| primary_of(&desktop, st));
        let i = want.and_then(|id| st.outputs.iter().position(|o| o.id == id)).or_else(|| {
            let p = primary_of(&desktop, st)?;
            st.outputs.iter().position(|o| o.id == p)
        });
        ui.set_page(i.map(|i| i as i32 + 1).unwrap_or(1));
    } else if page > old_count && page != ui.get_overview() {
        // About moves with the number of screens.
        ui.set_page(count + 1);
    }
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
                if let ServerMsg::Outputs { outputs, setup, .. } = m {
                    let weak = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(ui) = weak.upgrade() else { return };
                        if let Some(st) = SHARED.with(|s| s.borrow().clone()) {
                            set_screens(&ui, &mut st.borrow_mut(), outputs, setup);
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
            ui.set_same_as_primary(false);
        }
        refresh_tiles(&ui, &st);
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_page_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_browsing(false);
        show_scope(&ui, &s2.borrow());
    });

    // Same as the primary off: its own, starting from what it shows now; on:
    // back to the primary's.
    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_own_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let Some((id, label)) = screen_id(&ui, &st) else { return };
        let on = !ui.get_same_as_primary();
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

    // Make primary: this screen's wallpaper becomes the default; the old
    // primary keeps its picture as its own; nothing on screen changes.
    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_make_primary(move || {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let Some(new) = current_output(&ui, &st).map(|o| o.id.clone()) else { return };
        let desktop = read_desktop();
        let old = primary_of(&desktop, &st).and_then(|p| st.outputs.iter().find(|o| o.id == p).map(|o| (o.id.clone(), screen_label(o))));
        let res = update_desktop(|d| wall::make_primary(d, old.as_ref().map(|(i, l)| (i.as_str(), l.as_str())), &new));
        if let Err(e) = res {
            ui.set_status(format!("Could not save: {e}").into());
        }
        show_scope(&ui, &st);
    });

    // A screen dropped in the overview: put it edge to edge with the nearest
    // screen there, and record the arrangement for this setup.
    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_arrange_dropped(move |i, fx, fy| {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let mut rects: Vec<disp::Rect> =
            st.outputs.iter().map(|o| disp::Rect { id: o.id.clone(), x: o.x, y: o.y, w: o.width, h: o.height }).collect();
        let Some(i) = usize::try_from(i).ok().filter(|i| *i < rects.len()) else { return };
        let (x0, y0) = (rects.iter().map(|r| r.x).min().unwrap_or(0), rects.iter().map(|r| r.y).min().unwrap_or(0));
        let (bw, bh) = (
            rects.iter().map(|r| r.x + r.w).max().unwrap_or(1) - x0,
            rects.iter().map(|r| r.y + r.h).max().unwrap_or(1) - y0,
        );
        let (x, y) = (x0 + (fx * bw as f32).round() as i32, y0 + (fy * bh as f32).round() as i32);
        let snap_pts = (bw.max(bh) / 40).max(20);
        let (nx, ny) = disp::snap(&rects, i, x, y, snap_pts);
        rects[i].x = nx;
        rects[i].y = ny;
        if let Err(e) = update_desktop(|d| disp::set_arrangement(d, &st.setup, &rects)) {
            ui.set_status(format!("Could not save: {e}").into());
        }
    });

    let weak = ui.as_weak();
    ui.on_set_size(move || {
        let Some(ui) = weak.upgrade() else { return };
        let size = (ui.get_size() as f64).clamp(0.75, 1.5);
        if let Err(e) = update_desktop(|d| disp::set_size(d, size)) {
            ui.set_status(format!("Could not save: {e}").into());
        }
    });

    // The scale slider sticks to a mark (a whole number, the default) it comes
    // within 0.06 of; let go, the scale is saved (the default itself is saved as
    // "no choice", so it keeps following the screen's default).
    let weak = ui.as_weak();
    ui.on_scale_moved(move |v| {
        let Some(ui) = weak.upgrade() else { return };
        let marks: Vec<f64> = ui.get_detents().iter().map(f64::from).collect();
        if let Some(m) = disp::stick(v as f64, &marks, 0.06)
            && (m as f32) != v
        {
            ui.set_base_scale(m as f32);
        }
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_set_scale(move || {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let Some(o) = current_output(&ui, &st) else { return };
        let v = (ui.get_base_scale() as f64 * 120.0).round() / 120.0;
        let choice = (v != o.default_scale).then_some(v);
        let (id, label) = (o.id.clone(), screen_label(o));
        if let Err(e) = update_desktop(|d| disp::set_scale(d, &id, &label, choice)) {
            ui.set_status(format!("Could not save: {e}").into());
        }
    });

    let (weak, s2) = (ui.as_weak(), st.clone());
    ui.on_reset_scale(move || {
        let Some(ui) = weak.upgrade() else { return };
        let st = s2.borrow();
        let Some(o) = current_output(&ui, &st) else { return };
        let (id, label) = (o.id.clone(), screen_label(o));
        ui.set_base_scale(o.default_scale as f32);
        if let Err(e) = update_desktop(|d| disp::set_scale(d, &id, &label, None)) {
            ui.set_status(format!("Could not save: {e}").into());
        }
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
    /// Pictures whose previews the tiles have asked to be drawn.
    static TRIED: RefCell<std::collections::HashSet<PathBuf>> = RefCell::new(Default::default());
}
