//! The session dialog (a proper dialog for logging out, in place of a second
//! small menu; see docs/vdm-milestone-1.md): every
//! screen dimmed, a panel in the middle of the pointer's screen asking
//! whether to log out, with Lock, Cancel and Log Out (the default: Enter).
//! Escape, Cancel or a press outside the panel closes it. Opened by the
//! launcher's power button, the power key, Ctrl+Alt+Delete and the named
//! action ("Log Out…"). Log Out asks every
//! app to close as its close button would (exit.rs). On the left, Sleep,
//! Restart and Shut Down (VDM step 6), carried out by vdm: Sleep at once
//! (the lock comes on first, by vdm's sleep hook); Restart and Shut Down
//! after the apps have closed as for Log Out, the session's last act.

use std::hash::{Hash, Hasher};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
            },
            gles::GlesRenderer,
            utils::CommitCounter,
        },
    },
    utils::{Logical, Point, Rectangle, Transform},
};
use vergence_scene::{Color, Point as ScenePoint, Rect as SceneRect, Scene};

use crate::{Server, decor::SceneElement};

/// The buttons, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Button {
    Sleep,
    Restart,
    ShutDown,
    Lock,
    Cancel,
    LogOut,
}

const BUTTONS: [(Button, &str); 6] = [
    (Button::Sleep, "Sleep"),
    (Button::Restart, "Restart"),
    (Button::ShutDown, "Shut Down"),
    (Button::Lock, "Lock"),
    (Button::Cancel, "Cancel"),
    (Button::LogOut, "Log Out"),
];
/// The default: Log Out.
const DEFAULT: usize = 5;
/// The power buttons come first, a gap after them.
const POWER_BUTTONS: usize = 3;
// As the chosen mock-up, exactly (see docs/menus-milestone-1.md): a
// 600-point panel, 24 inside, the heading and a line under it,
// then the buttons, each as wide as its word, 8 apart, the power group well
// apart from Lock, Cancel and Log Out.
const W: i32 = 600;
const H: i32 = 150;
const PAD: i32 = 24;
const BUTTON_H: i32 = 34;
const BUTTON_GAP: i32 = 8;
/// The room either side of a button's word (the default's a little more).
const BUTTON_SIDE: f32 = 14.0;
const DEFAULT_SIDE: f32 = 16.0;
const TEXT: f32 = 13.5;

#[derive(Default)]
pub struct SessionDialog {
    pub open: bool,
    /// Restart or Shut Down, once the apps have closed (main.rs asks vdm).
    pub after_exit: Option<vergence_dm::power::Power>,
    /// The button Enter presses (the pointer over one makes it so).
    focus: usize,
    /// Where the panel and its buttons are, as last drawn.
    panel: Rectangle<i32, Logical>,
    buttons: Vec<(usize, Rectangle<i32, Logical>)>,
    dim: Option<(Id, CommitCounter)>,
    picture: Option<(u64, MemoryRenderBuffer)>,
}

impl Server {
    /// Open the dialog: menus close, the screens dim.
    pub fn session_dialog_open(&mut self) {
        if self.session_locked() || crate::greeter() {
            return;
        }
        self.close_menu();
        self.app_menu_close();
        self.launcher_close();
        self.card_close();
        self.drawer_close();
        self.session_dialog.open = true;
        self.session_dialog.focus = DEFAULT;
        tracing::info!("session: the log out dialog");
        self.redraw_all();
    }

    pub fn session_dialog_close(&mut self) {
        if std::mem::take(&mut self.session_dialog.open) {
            self.redraw_all();
        }
    }

    fn session_dialog_do(&mut self, i: usize) {
        self.session_dialog_close();
        match BUTTONS.get(i).map(|b| b.0) {
            Some(Button::Lock) => {
                self.lock_request("the log out dialog");
            }
            Some(Button::LogOut) => {
                tracing::info!("session: log out, the apps asked to close first");
                self.session_dialog.after_exit = None;
                self.begin_exit();
            }
            Some(Button::Sleep) => {
                tracing::info!("session: sleep, by vdm");
                std::thread::spawn(|| {
                    if let Err(e) = vergence_dm::power::ask(vergence_dm::power::Power::Sleep) {
                        tracing::warn!("session: sleep: {e}");
                    }
                });
            }
            Some(b @ (Button::Restart | Button::ShutDown)) => {
                let p = if b == Button::Restart {
                    vergence_dm::power::Power::Restart
                } else {
                    vergence_dm::power::Power::ShutDown
                };
                tracing::info!("session: {}, the apps asked to close first", p.word());
                self.session_dialog.after_exit = Some(p);
                self.begin_exit();
            }
            Some(Button::Cancel) | None => {}
        }
    }

    /// A key while it is open: every key is its (Escape cancels, Enter
    /// presses the default, Tab and the arrows move between the buttons).
    pub fn session_dialog_key(&mut self, sym: u32, pressed: bool) {
        use xkbcommon::xkb::keysyms as k;
        if !pressed {
            return;
        }
        let n = BUTTONS.len();
        let d = &mut self.session_dialog;
        match sym {
            k::KEY_Escape => return self.session_dialog_close(),
            k::KEY_Return | k::KEY_KP_Enter | k::KEY_space => {
                let i = d.focus;
                return self.session_dialog_do(i);
            }
            k::KEY_Tab | k::KEY_Right => d.focus = (d.focus + 1) % n,
            k::KEY_ISO_Left_Tab | k::KEY_Left => d.focus = (d.focus + n - 1) % n,
            _ => return,
        }
        self.redraw_all();
    }

    /// The pointer over a button makes it the one Enter presses.
    pub fn session_dialog_hover(&mut self, at: Point<f64, Logical>) {
        if !self.session_dialog.open {
            return;
        }
        if let Some(&(i, _)) = self.session_dialog.buttons.iter().find(|(_, r)| r.to_f64().contains(at))
            && i != self.session_dialog.focus
        {
            self.session_dialog.focus = i;
            self.redraw_all();
        }
    }

    /// A press: a button does what it says; outside the panel, Cancel.
    pub fn session_dialog_press(&mut self, at: Point<f64, Logical>) {
        if let Some(&(i, _)) = self.session_dialog.buttons.iter().find(|(_, r)| r.to_f64().contains(at)) {
            return self.session_dialog_do(i);
        }
        if !self.session_dialog.panel.to_f64().contains(at) {
            self.session_dialog_close();
        }
    }

    /// In front of everything on a screen: the panel (on the pointer's
    /// screen), then the dimming over the whole of it.
    pub fn session_dialog_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        if out.to_f64().contains(self.pointer_location())
            && let Some(e) = self.session_dialog_panel(renderer, out, scale)
        {
            elements.push(e);
        }
        let (id, commit) = self.session_dialog.dim.get_or_insert_with(|| (Id::new(), CommitCounter::default())).clone();
        let phys =
            Rectangle::<i32, smithay::utils::Physical>::from_size(out.size.to_f64().to_physical(scale).to_i32_round());
        elements.push(SceneElement::Solid(SolidColorRenderElement::new(
            id,
            phys,
            commit,
            [0.0, 0.0, 0.0, 0.45],
            Kind::Unspecified,
        )));
        elements
    }

    fn session_dialog_panel(
        &mut self,
        renderer: &mut GlesRenderer,
        out: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Option<SceneElement> {
        let light = self.light_style();
        let focus = self.session_dialog.focus;
        let accent = self.accent();
        let at = (out.loc.x + (out.size.w - W) / 2, out.loc.y + (out.size.h - H) / 2);
        let panel: Rectangle<i32, Logical> = Rectangle::new(at.into(), (W, H).into());
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        let face = fonts.face("sans-serif", 400, false)?;
        let bold = fonts.face("sans-serif", 600, false).unwrap_or(face);
        // Each button as wide as its word; Sleep, Restart and Shut Down from
        // the left, Lock, Cancel and Log Out (the default) to the right.
        let widths: Vec<i32> = BUTTONS
            .iter()
            .map(|(b, label)| {
                let (f, side) = if *b == Button::LogOut { (bold, DEFAULT_SIDE) } else { (face, BUTTON_SIDE) };
                (fonts.layout(f, TEXT, label).1 + 2.0 * side).ceil() as i32
            })
            .collect();
        let row_y = H - PAD - BUTTON_H;
        let mut local: Vec<(usize, i32, i32)> = Vec::new();
        let mut x = PAD;
        for (i, w) in widths.iter().enumerate().take(POWER_BUTTONS) {
            local.push((i, x, *w));
            x += w + BUTTON_GAP;
        }
        let mut x = W - PAD;
        for i in (POWER_BUTTONS..BUTTONS.len()).rev() {
            x -= widths[i];
            local.push((i, x, widths[i]));
            x -= BUTTON_GAP;
        }
        local.sort_by_key(|(i, _, _)| *i);
        self.session_dialog.panel = panel;
        self.session_dialog.buttons = local
            .iter()
            .map(|(i, x, w)| (*i, Rectangle::new((at.0 + x, at.1 + row_y).into(), (*w, BUTTON_H).into())))
            .collect();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (focus, light, scale.to_bits(), accent.r, accent.g, accent.b).hash(&mut h);
        let key = h.finish();
        if self.session_dialog.picture.as_ref().map(|(k, _)| *k) != Some(key) {
            let s = vergence_look::shelf_look(light);
            let ink = |c: vergence_look::Rgb| Color::rgb(c.0, c.1, c.2);
            let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
            let (w, hh) = (W as f32, H as f32);
            let mut scene = Scene::new(w, hh);
            scene.fill_rounded(SceneRect::new(0.0, 0.0, w, hh), 14.0, ink(s.menu));
            scene.stroke(SceneRect::new(0.0, 0.0, w, hh), 14.0, vergence_look::EDGE, ink(s.edge));
            let m = fonts.metrics(bold, 18.0);
            vergence_render::text(
                &mut scene,
                fonts,
                bold,
                18.0,
                ink(s.menu_ink),
                ScenePoint::new(PAD as f32, (PAD as f32 + m.ascent).round()),
                "Log out of Vergence?",
            );
            let m = fonts.metrics(face, TEXT);
            // The line under the heading: between the menus' two inks.
            let (a, b) = (s.menu_ink, s.menu_ink_off);
            let soft = Color::rgb(
                ((a.0 as u16 + b.0 as u16) / 2) as u8,
                ((a.1 as u16 + b.1 as u16) / 2) as u8,
                ((a.2 as u16 + b.2 as u16) / 2) as u8,
            );
            vergence_render::text(
                &mut scene,
                fonts,
                face,
                TEXT,
                soft,
                ScenePoint::new(PAD as f32, (PAD as f32 + 28.0 + m.ascent).round()),
                "Your apps will be asked to close first.",
            );
            // A plain button: the menu's ink, faint, over the panel.
            let plain = Color::rgba(s.menu_ink.0, s.menu_ink.1, s.menu_ink.2, 20);
            for (i, x, bw) in &local {
                let r = SceneRect::new(*x as f32, row_y as f32, *bw as f32, BUTTON_H as f32);
                let default = BUTTONS[*i].0 == Button::LogOut;
                // Only the default in the accent; the others plain.
                let (ground, text, f) =
                    if default { (accent, Color::rgb(0xff, 0xff, 0xff), bold) } else { (plain, ink(s.menu_ink), face) };
                scene.fill_rounded(r, 8.0, ground);
                if *i == focus {
                    scene.stroke(
                        SceneRect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0),
                        10.0,
                        2.0,
                        ink(s.menu_ink),
                    );
                }
                let label = BUTTONS[*i].1;
                let lw = fonts.layout(f, TEXT, label).1;
                let lm = fonts.metrics(f, TEXT);
                vergence_render::text(
                    &mut scene,
                    fonts,
                    f,
                    TEXT,
                    text,
                    ScenePoint::new(
                        (r.x + (r.w - lw) / 2.0).round(),
                        (r.y + (r.h - lm.ascent - lm.descent) / 2.0 + lm.ascent).round(),
                    ),
                    label,
                );
            }
            let p = vergence_render::Cpu::new().render(&scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)).ok()?;
            let buffer = MemoryRenderBuffer::from_slice(
                &p.data,
                Fourcc::Abgr8888,
                (p.width as i32, p.height as i32),
                1,
                Transform::Normal,
                None,
            );
            self.session_dialog.picture = Some((key, buffer));
        }
        let (_, buffer) = self.session_dialog.picture.as_ref()?;
        let a = (panel.loc - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let b = (panel.loc + panel.size.to_point() - out.loc).to_f64().to_physical(scale).to_i32_round::<i32>();
        let src = Rectangle::<f64, Logical>::from_size(((b.x - a.x) as f64, (b.y - a.y) as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            a.to_f64(),
            buffer,
            None,
            Some(src),
            Some(panel.size),
            Kind::Unspecified,
        )
        .ok()
        .map(SceneElement::Memory)
    }
}
