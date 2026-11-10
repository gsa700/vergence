// The shelf (whitepaper §6.9; docs/shelf-milestone-1.md, step 1).
//
// One strip on the right edge of the primary screen, drawn by the server. A
// screen with the shelf has a WORK AREA, the screen less the strip: maximise,
// default placement and fitting, the desktop menu and notification cards keep
// inside it, and a remembered window the strip would cover is moved clear.
// Where the strip is and what it leaves is vergence-shell's arithmetic; this
// file knows which screen, draws, and (in later steps) routes input.
//
// The strip is drawn with the project's own drawing code: a scene
// (vergence-scene) rendered by vergence-render's CPU renderer at each
// screen's exact pixels, kept until something in it changes. It is the first
// thing the server draws that way; title bars, the menu and notifications
// follow when the server's compositing moves onto the renderer.
//
// ~/.config/vergence/desktop.toml:
//
//     [shelf]
//     hide = false     # read; does nothing until step 6
//
// The screen is the primary one ([displays] primary, else the first in
// arrangement order): vergence_profile::primary, the settings app's rule.

use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}},
            gles::GlesRenderer,
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Rect as SceneRect, Scene};
use vergence_shell::Rect;

use crate::{Server, decor::SceneElement};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShelfConfig {
    pub hide: bool,
}

#[derive(Debug, Default, Deserialize)]
struct DesktopFile {
    #[serde(default)]
    shelf: ShelfConfig,
    #[serde(default)]
    displays: Primary,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Primary {
    primary: Option<String>,
}

/// What desktop.toml says about the shelf: its own table, and which screen is primary.
pub fn load() -> (ShelfConfig, Option<String>) {
    crate::wallpaper::desktop_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| match toml::from_str::<DesktopFile>(&t) {
            Ok(f) => (f.shelf, f.displays.primary),
            Err(e) => {
                tracing::warn!("desktop.toml [shelf]: {e}");
                Default::default()
            }
        })
        .unwrap_or_default()
}

#[derive(Default)]
pub struct Shelf {
    pub config: ShelfConfig,
    /// `[displays] primary`, as recorded (it may not be connected).
    pub primary: Option<String>,
    /// The strip's picture for a height in points and a scale, kept until either changes.
    picture: Option<((i32, u64), MemoryRenderBuffer)>,
    /// Where the strip was last reported in the log, so a move is said once.
    said: Option<Rectangle<i32, Logical>>,
}

impl Shelf {
    /// The shelf as desktop.toml has it, with nothing drawn yet.
    pub fn load() -> Shelf {
        let (config, primary) = load();
        Shelf { config, primary, ..Default::default() }
    }
}

fn rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

fn rectangle(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

impl Server {
    /// Read the shelf's settings again (at start, and when desktop.toml changes).
    pub fn shelf_reload(&mut self) {
        let (config, primary) = load();
        if config != self.shelf.config || primary != self.shelf.primary {
            self.shelf.config = config;
            self.shelf.primary = primary;
            self.shelf_moved();
        }
    }

    /// The screen the shelf is on: the primary one.
    pub fn shelf_screen(&self) -> Option<Rectangle<i32, Logical>> {
        let outputs = self.bus_outputs();
        let ids: Vec<String> = outputs.iter().map(|o| o.id.clone()).collect();
        let primary = vergence_profile::primary(self.shelf.primary.as_deref(), &ids)?;
        let o = outputs.iter().find(|o| o.id == primary)?;
        Some(Rectangle::new((o.x, o.y).into(), (o.width, o.height).into()))
    }

    /// The strip, if a screen has it.
    pub fn shelf_rect(&self) -> Option<Rectangle<i32, Logical>> {
        vergence_shell::strip(rect(self.shelf_screen()?)).map(rectangle)
    }

    /// What a screen leaves for windows: all of it, less the strip if the shelf is on it.
    pub fn work_area(&self, screen: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        if self.shelf_screen() == Some(screen) { rectangle(vergence_shell::work_area(rect(screen))) } else { screen }
    }

    /// A window's place moved clear of the strip, if the strip would cover part of it.
    pub fn clear_of_shelf(&self, window: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        match self.shelf_screen() {
            Some(screen) => rectangle(vergence_shell::clear_of_strip(rect(window), rect(screen))),
            None => window,
        }
    }

    /// True for a point on the strip: the windows behind it get nothing of the pointer.
    pub fn shelf_at(&self, at: Point<f64, Logical>) -> bool {
        self.shelf_rect().is_some_and(|r| r.to_f64().contains(at))
    }

    /// The screens or the settings changed: say where the strip is now, and
    /// bring maximised windows on its screen inside the work area.
    pub fn shelf_moved(&mut self) {
        let now = self.shelf_rect();
        if now == self.shelf.said {
            return;
        }
        self.shelf.said = now;
        match now {
            Some(r) => tracing::info!("shelf: strip at {},{} {}x{} points", r.loc.x, r.loc.y, r.size.w, r.size.h),
            None => tracing::info!("shelf: no screen for it"),
        }
        // A maximised window fills its screen's work area, which has just changed.
        let maximised: Vec<_> = self.restore.keys().cloned().collect();
        for w in maximised {
            self.toggle_maximize(&w);
            self.toggle_maximize(&w);
        }
    }

    /// For `vctl shelf`.
    pub fn shelf_report(&self) -> String {
        match (self.shelf_rect(), self.shelf_screen()) {
            (Some(r), Some(s)) => {
                let w = self.work_area(s);
                format!(
                    "strip {},{} {}x{} on screen {},{} {}x{}; work area {},{} {}x{}; hide {} (not yet acted on)",
                    r.loc.x, r.loc.y, r.size.w, r.size.h, s.loc.x, s.loc.y, s.size.w, s.size.h, w.loc.x, w.loc.y, w.size.w, w.size.h, self.shelf.config.hide
                )
            }
            _ => "no shelf: no screen wide enough is connected".into(),
        }
    }

    /// The strip's picture on `out`, if it is on that screen.
    pub fn shelf_elements(&mut self, renderer: &mut GlesRenderer, out: Rectangle<i32, Logical>, scale: f64) -> Vec<SceneElement> {
        let Some(strip) = self.shelf_rect().filter(|r| r.overlaps(out)) else { return Vec::new() };
        let key = (strip.size.h, scale.to_bits());
        if self.shelf.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            // The strip as a scene, in points: its ground, and a line along the side
            // that faces the windows.
            let (w, h) = (strip.size.w as f32, strip.size.h as f32);
            let colour = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let mut scene = Scene::new(w, h);
            scene.fill(SceneRect::new(0.0, 0.0, w, h), colour(vergence_look::SHELF));
            scene.fill(SceneRect::new(0.0, 0.0, vergence_look::EDGE, h), colour(vergence_look::SHELF_EDGE));
            // No text in it yet, so no fonts are looked for.
            let mut fonts = vergence_text::Fonts::new(vergence_text::db::FontDb::default());
            let buffer = match vergence_render::Cpu::new().render(&scene, &mut fonts, scale as f32, colour(vergence_look::SHELF)) {
                Ok(p) => MemoryRenderBuffer::from_slice(&p.data, Fourcc::Abgr8888, (p.width as i32, p.height as i32), 1, Transform::Normal, None),
                Err(e) => {
                    tracing::warn!("shelf: its picture could not be drawn: {e}");
                    return Vec::new();
                }
            };
            self.shelf.picture = Some((key, buffer));
        }
        let Some((_, buffer)) = &self.shelf.picture else { return Vec::new() };
        // Placed on whole pixels, as everything the server draws is.
        let a = (strip.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (strip.loc + strip.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        match MemoryRenderBufferRenderElement::from_buffer(renderer, a.to_f64(), buffer, None, Some(src), Some(strip.size), Kind::Unspecified) {
            Ok(el) => vec![SceneElement::Memory(el)],
            Err(_) => Vec::new(),
        }
    }
}
