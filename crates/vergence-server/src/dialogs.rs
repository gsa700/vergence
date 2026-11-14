// Dialogs and palettes belong to their parent (docs/window-styles-milestone-1.md,
// step 3): a slim bar with only close (decor.rs), opened over the parent,
// going where it goes, in front of it whenever it comes forward, minimized and
// back with it, and not listed apart from it (the dock's list, Alt+Tab). A
// modal dialog dims its parent and takes every press on it: the press brings
// the dialog forward and flashes its border (Dave, 2026-11-14: (a) to dimming).

use std::time::{Duration, Instant};

use smithay::utils::{IsAlive, Logical, Point, Rectangle};

use crate::{Server, style::Kind, win::Win};

/// How long a modal dialog's border flashes when its parent is pressed.
pub const FLASH: Duration = Duration::from_millis(300);

/// Where a dialog was put relative to its parent, and both places when last looked at.
#[derive(Debug, Clone)]
pub struct Follow {
    parent: Win,
    parent_at: Point<i32, Logical>,
    at: Point<i32, Logical>,
}

#[derive(Default)]
pub struct Dialogs {
    pub follow: std::collections::HashMap<Win, Follow>,
    /// A modal dialog whose border flashes, until when.
    pub flash: Option<(Win, Instant)>,
}

impl Server {
    /// The dialogs and palettes whose parent is `parent` (in the space or minimized).
    pub fn children_of(&self, parent: &Win) -> Vec<Win> {
        self.space
            .elements()
            .cloned()
            .chain(self.shelf.minimized.iter().map(|(w, _)| w.clone()))
            .filter(|w| w.alive() && w != parent)
            .filter(|w| {
                let s = self.window_style(w);
                matches!(s.kind, Kind::Dialog | Kind::Palette) && s.parent.as_ref() == Some(parent)
            })
            .collect()
    }

    /// The modal dialog that blocks `parent`, if any.
    pub fn modal_of(&self, parent: &Win) -> Option<Win> {
        self.children_of(parent).into_iter().find(|c| self.window_style(c).modal && self.space.element_geometry(c).is_some())
    }

    /// A dialog or palette with a parent is the parent's, not a window of its own.
    pub fn belongs_to_parent(&self, w: &Win) -> bool {
        let s = self.window_style(w);
        matches!(s.kind, Kind::Dialog | Kind::Palette) && s.parent.is_some()
    }

    /// A dialog just shown: over the middle of its parent, kept on the
    /// parent's screen, and from then on going with it.
    pub fn place_dialog(&mut self, w: &Win) {
        let s = self.window_style(w);
        if s.kind != Kind::Dialog {
            return;
        }
        let (Some(p), Some(geo)) = (s.parent, self.space.element_geometry(w)) else { return };
        let Some(pg) = self.space.element_geometry(&p) else { return };
        let mut at: Point<i32, Logical> = (pg.loc.x + (pg.size.w - geo.size.w) / 2, pg.loc.y + (pg.size.h - geo.size.h) / 2).into();
        let mid = (pg.loc.x + pg.size.w / 2, pg.loc.y + pg.size.h / 2);
        if let Some(screen) = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|o| o.contains(mid)) {
            let area = self.work_area(screen);
            let top = if self.has_title(w) { self.bar_h(w) } else { 0 };
            at.x = at.x.min(area.loc.x + area.size.w - geo.size.w).max(area.loc.x);
            at.y = at.y.min(area.loc.y + area.size.h - geo.size.h).max(area.loc.y + top);
        }
        self.move_window(w, at);
        self.dialogs.follow.insert(w.clone(), Follow { parent: p.clone(), parent_at: pg.loc, at });
    }

    /// Put a window's content at `at` (X11 windows are told).
    fn move_window(&mut self, w: &Win, at: Point<i32, Logical>) {
        self.space.map_element(w.clone(), at, false);
        if let Some(x) = w.x11_surface() {
            let _ = x.configure(Rectangle::new(at, x.geometry().size));
        }
    }

    /// Each frame: a dialog goes where its parent went; one the user moved
    /// keeps its new place relative to the parent.
    pub fn dialogs_follow(&mut self) {
        self.dialogs.follow.retain(|w, f| w.alive() && f.parent.alive());
        let follows: Vec<(Win, Follow)> = self.dialogs.follow.iter().map(|(w, f)| (w.clone(), f.clone())).collect();
        for (w, mut f) in follows {
            let (Some(pg), Some(g)) = (self.space.element_geometry(&f.parent), self.space.element_geometry(&w)) else { continue };
            if pg.loc != f.parent_at {
                let to = g.loc + (pg.loc - f.parent_at);
                self.move_window(&w, to);
                f.at = to;
            } else if g.loc != f.at {
                f.at = g.loc;
            }
            f.parent_at = pg.loc;
            self.dialogs.follow.insert(w, f);
        }
    }

    /// A press on a parent a modal dialog blocks: the dialog comes forward and flashes.
    pub fn flash_dialog(&mut self, dialog: &Win) {
        self.focus_window(dialog, smithay::utils::SERIAL_COUNTER.next_serial());
        self.dialogs.flash = Some((dialog.clone(), Instant::now()));
        self.redraw_all();
    }

    /// Whether a window's border is flashing now.
    pub fn flashing(&self, w: &Win) -> bool {
        self.dialogs.flash.as_ref().is_some_and(|(f, since)| f == w && since.elapsed() < FLASH)
    }
}
