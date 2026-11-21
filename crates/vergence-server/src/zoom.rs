// The magnifier (meaning milestone 1, step 7): the server zooms what it
// draws, the whole screen (the Mac's default) or a lens, around a point of
// interest: the pointer, the focused control or the text caret, whichever
// moved last (each can be turned off). Our own windows tell focus and caret
// through their meaning trees (meaning.rs); others are followed by the
// pointer.
//
// How: every element of the screen's frame is scaled around the point
// (smithay's RescaleRenderElement), so the point stays where it is on the
// real screen and what is around it grows. The pointer goes through the
// same scaling, so what is drawn under it is what is under it: clicks need
// no translation. The lens is the same elements cropped to a framed
// rectangle around the point, in front of the screen as it is. Pixels for
// now: textures are drawn larger (sharp once display lists come).
//
// Keys (vergence-seat): Super+Alt+= and Super+Alt+- zoom in and out (in
// turns it on; out past the least level turns it off), Super+Alt+8 on and
// off, Super+Alt+L the lens. desktop.toml [magnifier]: on, level, style
// ("screen" or "lens"), follow-pointer, follow-focus, follow-caret,
// lens-width, lens-height (points); the Accessibility page writes them.
// Zooming is smooth on the GPU unless Reduce motion is on.

use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{
            Id, Kind,
            memory::MemoryRenderBufferRenderElement,
            solid::SolidColorRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
        gles::GlesRenderer,
        utils::CommitCounter,
    },
    utils::{Logical, Physical, Point, Rectangle, Size},
};

use crate::Server;

smithay::backend::renderer::element::render_elements! {
    /// What the magnifier scales: a screen's elements (drm.rs's OutputElements).
    pub Inner<=GlesRenderer>;
    Scene = crate::decor::SceneElement,
    PointerTheme = MemoryRenderBufferRenderElement<GlesRenderer>,
    PointerSurface = WaylandSurfaceRenderElement<GlesRenderer>,
}

/// An element as the magnifier draws it.
pub type Zoomed = CropRenderElement<RescaleRenderElement<Inner>>;

/// The least and greatest levels, and a key's step.
pub const LEAST: f64 = 1.25;
pub const MOST: f64 = 20.0;
const STEP: f64 = 1.25;
/// The lens's frame, in points.
const FRAME: f64 = 3.0;
/// How quickly the drawn level and point reach what is wanted (seconds to
/// cover about two thirds of the way), and the frame step while moving.
const EASE: f64 = 0.07;
const TICK: Duration = Duration::from_millis(16);

/// desktop.toml's [magnifier].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    pub on: bool,
    pub level: f64,
    pub lens: bool,
    pub follow_pointer: bool,
    pub follow_focus: bool,
    pub follow_caret: bool,
    pub lens_size: (f64, f64),
}

impl Default for Config {
    fn default() -> Config {
        Config { on: false, level: 2.0, lens: false, follow_pointer: true, follow_focus: true, follow_caret: true, lens_size: (480.0, 270.0) }
    }
}

pub fn config_from(text: &str) -> Config {
    let d = Config::default();
    let table = text.parse::<toml::Table>().ok();
    let Some(t) = table.as_ref().and_then(|t| t.get("magnifier")).and_then(|m| m.as_table()) else { return d };
    let b = |k: &str, v: bool| t.get(k).and_then(|x| x.as_bool()).unwrap_or(v);
    let f = |k: &str, v: f64| t.get(k).and_then(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64))).filter(|x| x.is_finite()).unwrap_or(v);
    Config {
        on: b("on", d.on),
        level: f("level", d.level).clamp(LEAST, MOST),
        lens: t.get("style").and_then(|s| s.as_str()) == Some("lens"),
        follow_pointer: b("follow-pointer", d.follow_pointer),
        follow_focus: b("follow-focus", d.follow_focus),
        follow_caret: b("follow-caret", d.follow_caret),
        lens_size: (f("lens-width", d.lens_size.0).clamp(120.0, 2000.0), f("lens-height", d.lens_size.1).clamp(80.0, 1500.0)),
    }
}

fn focus_ring_wanted() -> bool {
    let text = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    text.parse::<toml::Table>().ok().and_then(|t| t.get("appearance").and_then(|a| a.get("focus-ring")).and_then(|v| v.as_bool())).unwrap_or(false)
}

fn load_config() -> Config {
    config_from(&crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default())
}

pub struct Zoom {
    /// The server's own focus ring (step 8): desktop.toml [appearance]
    /// focus-ring, off by default.
    pub focus_ring: bool,
    ring_ids: [Id; 4],
    /// The settings as last read (a change to `on`, `level` or `style`
    /// there applies; the keys change only what is shown now).
    config: Config,
    pub on: bool,
    pub lens: bool,
    /// The level wanted while on.
    pub level: f64,
    /// The level drawn now (1 when off and settled).
    pub shown: f64,
    /// The point of interest wanted, and drawn (desktop coordinates).
    pub point: Point<f64, Logical>,
    pub at: Point<f64, Logical>,
    /// What set the point last: "pointer", "focus" or "caret".
    pub by: &'static str,
    last_pointer: Point<f64, Logical>,
    stepped: Instant,
    timer: bool,
    /// The lens's frame and ground (ids kept for damage tracking).
    frame_ids: [Id; 4],
    ground_id: Id,
    commit: CommitCounter,
}

impl Default for Zoom {
    fn default() -> Zoom {
        let c = if cfg!(test) { Config::default() } else { load_config() };
        Zoom {
            focus_ring: !cfg!(test) && focus_ring_wanted(),
            ring_ids: [Id::new(), Id::new(), Id::new(), Id::new()],
            config: c,
            on: c.on,
            lens: c.lens,
            level: c.level,
            shown: 1.0,
            point: Point::default(),
            at: Point::default(),
            by: "pointer",
            last_pointer: Point::default(),
            stepped: Instant::now(),
            timer: false,
            frame_ids: [Id::new(), Id::new(), Id::new(), Id::new()],
            ground_id: Id::new(),
            commit: CommitCounter::default(),
        }
    }
}

/// What a magnifier key does (vergence-seat's KeyOut::Zoom).
pub use vergence_seat::keyboard::ZoomKey;

impl Server {
    /// desktop.toml changed (or the session started).
    pub fn zoom_reload(&mut self) {
        let ring = focus_ring_wanted();
        if ring != self.zoom.focus_ring {
            self.zoom.focus_ring = ring;
            self.redraw_all();
        }
        let c = load_config();
        let old = self.zoom.config;
        if c == old {
            return;
        }
        tracing::info!("magnifier: {c:?}");
        self.zoom.config = c;
        if c.on != old.on {
            self.zoom.on = c.on;
        }
        if c.level != old.level {
            self.zoom.level = c.level;
        }
        if c.lens != old.lens {
            self.zoom.lens = c.lens;
        }
        self.zoom_go();
    }

    /// A magnifier key.
    pub fn zoom_key(&mut self, k: ZoomKey) {
        let z = &mut self.zoom;
        match k {
            ZoomKey::In if !z.on => z.on = true,
            ZoomKey::In => z.level = (z.level * STEP).min(MOST),
            ZoomKey::Out if z.on && z.level / STEP < LEAST - 1e-6 => z.on = false,
            ZoomKey::Out => z.level = (z.level / STEP).max(LEAST),
            ZoomKey::Toggle => z.on = !z.on,
            ZoomKey::Lens => z.lens = !z.lens,
        }
        tracing::info!("magnifier: on {} level {:.2} lens {}", z.on, z.level, z.lens);
        self.zoom_go();
    }

    /// The point of interest moved: the focused control or the caret of the
    /// window with the keyboard (meaning.rs), desktop coordinates.
    pub fn zoom_follow(&mut self, by: &'static str, at: (f64, f64)) {
        let wanted = match by {
            "focus" => self.zoom.config.follow_focus,
            "caret" => self.zoom.config.follow_caret,
            _ => self.zoom.config.follow_pointer,
        };
        if !wanted || !at.0.is_finite() || !at.1.is_finite() {
            return;
        }
        self.zoom.point = (at.0, at.1).into();
        self.zoom.by = by;
        if self.zoom_active() {
            self.zoom_go();
        }
    }

    /// The level a screen (`geo` on the desktop) is magnified at, once the
    /// zoom has settled (display lists step 5): windows with scenes are then
    /// drawn at that many times the screen's density, so the magnified
    /// picture is sharp. None while it moves (the pictures are enlarged then)
    /// or when the screen is not magnified.
    pub fn zoom_sharp(&self, geo: Rectangle<i32, Logical>) -> Option<f64> {
        let z = &self.zoom;
        (!z.timer && z.on && z.shown > 1.0005 && geo.to_f64().contains(z.at)).then_some(z.shown)
    }

    /// Whether anything is magnified now (or still settling).
    pub fn zoom_active(&self) -> bool {
        self.zoom.on || self.zoom.shown > 1.0005
    }

    /// Something changed: step toward it, frame by frame.
    fn zoom_go(&mut self) {
        self.zoom.stepped = Instant::now();
        if !self.motion_wanted() {
            self.zoom.shown = if self.zoom.on { self.zoom.level } else { 1.0 };
            self.zoom.at = self.zoom.point;
            self.redraw_all();
            return;
        }
        if self.zoom.timer {
            return;
        }
        self.zoom.timer = true;
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        let _ = self.loop_handle.insert_source(Timer::immediate(), |_, _, st| {
            let done = st.zoom_step();
            st.redraw_all();
            if done {
                st.zoom.timer = false;
                return TimeoutAction::Drop;
            }
            TimeoutAction::ToDuration(TICK)
        });
    }

    /// One step of the drawn level and point toward the wanted; true once there.
    fn zoom_step(&mut self) -> bool {
        let z = &mut self.zoom;
        let dt = z.stepped.elapsed().as_secs_f64().min(0.1);
        z.stepped = Instant::now();
        let k = 1.0 - (-dt / EASE).exp();
        let target = if z.on { z.level } else { 1.0 };
        z.shown += (target - z.shown) * k;
        z.at += (z.point - z.at).upscale(k);
        let near = (target - z.shown).abs() < 0.002 && (z.point.x - z.at.x).abs() < 0.3 && (z.point.y - z.at.y).abs() < 0.3;
        if near {
            z.shown = target;
            z.at = z.point;
        }
        near
    }

    /// Before each frame: the pointer moved since the last one: it is the
    /// point of interest (if followed).
    pub fn zoom_pointer(&mut self, pointer: Point<f64, Logical>) {
        if pointer == self.zoom.last_pointer {
            return;
        }
        self.zoom.last_pointer = pointer;
        if self.zoom.config.follow_pointer {
            self.zoom.point = pointer;
            self.zoom.by = "pointer";
            if self.zoom_active() {
                if self.motion_wanted() {
                    // Following the hand: no lag behind it.
                    self.zoom.at = pointer;
                }
                self.zoom_go();
            }
        }
    }

    /// Whether this screen (`geo` on the desktop) is magnified now:
    /// None, or Some(lens).
    pub fn zoom_here(&self, geo: Rectangle<i32, Logical>) -> Option<bool> {
        let z = &self.zoom;
        (z.shown > 1.0005 && geo.to_f64().contains(z.at)).then_some(z.lens)
    }

    /// A screen's frame as the magnifier shows it: `front` (its elements,
    /// front first) scaled about the point of interest; for the lens, cropped
    /// to it, framed, in front of `back` (the same elements made again,
    /// unscaled). `geo` is the screen on the desktop.
    pub fn zoom_frame(&self, front: Vec<Inner>, back: Vec<Inner>, geo: Rectangle<i32, Logical>, scale: f64, ground: [f32; 4]) -> Vec<Zoomed> {
        let z = &self.zoom;
        let level = z.shown;
        let origin: Point<i32, Physical> = (z.at - geo.loc.to_f64()).to_physical(scale).to_i32_round();
        let screen: Rectangle<i32, Physical> = Rectangle::new((0, 0).into(), geo.size.to_f64().to_physical(scale).to_i32_round());
        let scaled = |e: Inner, level: f64, crop: Rectangle<i32, Physical>| CropRenderElement::from_element(RescaleRenderElement::from_element(e, origin, level), scale, crop);
        if !z.lens {
            return front.into_iter().filter_map(|e| scaled(e, level, screen)).collect();
        }
        // The lens: centered on the point, kept on the screen.
        let lw = ((z.config.lens_size.0 * scale).round() as i32).min(screen.size.w);
        let lh = ((z.config.lens_size.1 * scale).round() as i32).min(screen.size.h);
        let x = (origin.x - lw / 2).clamp(0, (screen.size.w - lw).max(0));
        let y = (origin.y - lh / 2).clamp(0, (screen.size.h - lh).max(0));
        let lens = Rectangle::new((x, y).into(), Size::from((lw, lh)));
        let f = (FRAME * scale).round().max(1.0) as i32;
        let frame = [
            Rectangle::new((x - f, y - f).into(), Size::from((lw + 2 * f, f))),
            Rectangle::new((x - f, y + lh).into(), Size::from((lw + 2 * f, f))),
            Rectangle::new((x - f, y).into(), Size::from((f, lh))),
            Rectangle::new((x + lw, y).into(), Size::from((f, lh))),
        ];
        let solid = |id: &Id, r: Rectangle<i32, Physical>, c: [f32; 4]| Inner::Scene(crate::decor::SceneElement::Solid(SolidColorRenderElement::new(id.clone(), r, z.commit, c, Kind::Unspecified)));
        let mut out: Vec<Zoomed> = Vec::new();
        // A rescale of 1 is the element itself.
        for (id, r) in z.frame_ids.iter().zip(frame) {
            out.extend(scaled(solid(id, r, [0.23, 0.48, 0.92, 1.0]), 1.0, screen));
        }
        out.extend(front.into_iter().filter_map(|e| scaled(e, level, lens)));
        out.extend(scaled(solid(&z.ground_id, lens, ground), 1.0, screen));
        out.extend(back.into_iter().filter_map(|e| scaled(e, 1.0, screen)));
        out
    }

    /// The focus ring (step 8): around the focused control of the window
    /// with the keyboard, from its meaning tree, in the accent, kept inside
    /// the window; nothing for a control that fills most of it (a page, a
    /// text area: the window itself says that). Placed by the scene just in
    /// front of that window's picture, so windows in front cover it.
    pub fn focus_ring_elements(&self, w: &crate::win::Win, out: Rectangle<i32, Logical>, scale: f64) -> Vec<crate::decor::SceneElement> {
        if !self.zoom.focus_ring || self.input.keyboard_native.as_ref() != Some(w) {
            return Vec::new();
        }
        let Some(n) = w.native() else { return Vec::new() };
        let Some((focus, _, nodes)) = n.meaning() else { return Vec::new() };
        let Some(node) = nodes.iter().find(|m| m.id == focus && focus != 0) else { return Vec::new() };
        let Some(geo) = self.space.element_geometry(w) else { return Vec::new() };
        let (ww, wh) = (geo.size.w as f64, geo.size.h as f64);
        if node.w * node.h > 0.6 * ww * wh || node.w < 1.0 || node.h < 1.0 {
            return Vec::new();
        }
        // Points on the desktop: 2 out from the control, 3 wide; the tree
        // counts from the picture's top (meaning_origin).
        let (gap, t) = (2.0, 3.0);
        let (ox, oy) = self.meaning_origin(w);
        let x0 = (ox + node.x - gap - t).max(geo.loc.x as f64);
        let y0 = (oy + node.y - gap - t).max(geo.loc.y as f64);
        let x1 = (ox + node.x + node.w + gap + t).min(geo.loc.x as f64 + ww);
        let y1 = (oy + node.y + node.h + gap + t).min(geo.loc.y as f64 + wh);
        if x1 - x0 < 2.0 * t || y1 - y0 < 2.0 * t {
            return Vec::new();
        }
        let a = self.accent();
        let color = [a.r as f32 / 255.0, a.g as f32 / 255.0, a.b as f32 / 255.0, 1.0];
        let px = |x: f64, y: f64, w: f64, h: f64| {
            let p0: Point<i32, Physical> = Point::<f64, Logical>::from((x - out.loc.x as f64, y - out.loc.y as f64)).to_physical(scale).to_i32_round();
            let p1: Point<i32, Physical> = Point::<f64, Logical>::from((x + w - out.loc.x as f64, y + h - out.loc.y as f64)).to_physical(scale).to_i32_round();
            Rectangle::<i32, Physical>::new(p0, Size::from(((p1.x - p0.x).max(1), (p1.y - p0.y).max(1))))
        };
        let sides = [px(x0, y0, x1 - x0, t), px(x0, y1 - t, x1 - x0, t), px(x0, y0 + t, t, y1 - y0 - 2.0 * t), px(x1 - t, y0 + t, t, y1 - y0 - 2.0 * t)];
        self.zoom.ring_ids.iter().zip(sides).map(|(id, r)| crate::decor::SceneElement::Solid(SolidColorRenderElement::new(id.clone(), r, self.zoom.commit, color, Kind::Unspecified))).collect()
    }

    /// For `vctl zoom`.
    pub fn zoom_report(&self) -> String {
        let z = &self.zoom;
        format!(
            "{} level {:.2} (drawn {:.2}) {} at {:.0},{:.0} by {}{}",
            if z.on { "on" } else { "off" },
            z.level,
            z.shown,
            if z.lens { "lens" } else { "screen" },
            z.point.x,
            z.point.y,
            z.by,
            if z.timer { ", moving" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_settings() {
        assert_eq!(config_from(""), Config::default());
        let c = config_from("[magnifier]\non = true\nlevel = 3\nstyle = \"lens\"\nfollow-caret = false\nlens-width = 9999\n");
        assert!(c.on && c.lens && !c.follow_caret && c.follow_focus);
        assert_eq!(c.level, 3.0, "an integer level is read too");
        assert_eq!(c.lens_size.0, 2000.0, "kept within bounds");
        assert_eq!(config_from("[magnifier]\nlevel = 500.0\n").level, MOST);
        assert_eq!(config_from("[magnifier]\nlevel = nan\n").level, Config::default().level);
    }
}
