// A window's kind and its size limits, whatever program it belongs to
// (docs/window-styles-milestone-1.md, step 1).
//
// Native windows say them (bus protocol v4, `Style`; native.rs keeps it).
// Wayland windows: a parent makes a dialog, modal by the xdg-dialog hint
// (Smithay's front end, handlers/xdg_shell.rs), and xdg-shell's smallest and
// largest size. X11 windows: `_NET_WM_WINDOW_TYPE` (dialog, utility as a
// palette, splash), `WM_TRANSIENT_FOR` (the parent), `_NET_WM_STATE_MODAL`
// and `WM_NORMAL_HINTS`. One answer for all three, so the rest of the
// server (the bar, sizes, the dock, Alt+Tab) never asks which road a window
// came by.

use smithay::{
    utils::{Logical, Size},
    wayland::{
        compositor,
        shell::xdg::{SurfaceCachedState, XdgToplevelSurfaceData, dialog::ToplevelDialogHint},
    },
    xwayland::xwm::WmWindowType,
};

use crate::{Server, win::Win};

/// A window's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Normal,
    Dialog,
    Palette,
    Sidebar,
    /// A splash screen: no bar.
    Splash,
    /// The desktop's icons (desktop.rs).
    Desktop,
}

/// What the server goes by for a window.
#[derive(Debug, Clone, Default)]
pub struct WinStyle {
    pub kind: Kind,
    pub modal: bool,
    /// Smallest and largest size of its content, in points; None: no limit.
    pub min: Option<Size<i32, Logical>>,
    pub max: Option<Size<i32, Logical>>,
    pub parent: Option<Win>,
}

impl WinStyle {
    /// Smallest and largest the same on both axes: it cannot be resized.
    pub fn fixed(&self) -> bool {
        matches!((self.min, self.max), (Some(a), Some(b)) if a == b)
    }
}

/// The largest size a program may ask for, and the smallest largest size it
/// may set (a largest size of a few points would hide a window: §7.3).
const MAX_LIMIT: i32 = 16384;
const LEAST_MAX: i32 = 64;

/// A size limit from a program: 0 on an axis is none; kept within sane bounds.
fn limits(min: (i32, i32), max: (i32, i32)) -> (Option<Size<i32, Logical>>, Option<Size<i32, Logical>>) {
    let clamp = |v: i32| v.clamp(0, MAX_LIMIT);
    let (minw, minh) = (clamp(min.0), clamp(min.1));
    let min = (minw > 0 || minh > 0).then(|| Size::from((minw, minh)));
    // An axis with no largest size takes the greatest; one that has one keeps it,
    // never below the smallest of its axis nor below LEAST_MAX.
    let axis = |m: i32, low: i32| if m <= 0 { MAX_LIMIT } else { clamp(m).max(low).max(LEAST_MAX) };
    let max = (max.0 > 0 || max.1 > 0).then(|| Size::from((axis(max.0, minw), axis(max.1, minh))));
    (min, max)
}

/// The kind an X11 window's type and transient-for make it.
pub fn x11_kind(types: Option<WmWindowType>, transient: bool) -> Kind {
    match types {
        Some(WmWindowType::Dialog) => Kind::Dialog,
        Some(WmWindowType::Utility | WmWindowType::Toolbar) if transient => Kind::Palette,
        Some(WmWindowType::Splash) => Kind::Splash,
        // A normal window that is transient for another is a dialog in all but name (ICCCM).
        Some(WmWindowType::Normal) | None if transient => Kind::Dialog,
        _ => Kind::Normal,
    }
}

impl Server {
    /// A window's kind, modal, limits and parent.
    pub fn window_style(&self, w: &Win) -> WinStyle {
        if let Some(n) = w.native() {
            let (parent, style) = n.style();
            // A portal dialog's parent is another program's window (portal.rs).
            let parent = (parent != 0)
                .then(|| self.natives.get(&(n.client, parent)).cloned())
                .flatten()
                .or_else(|| self.portal.adopted.get(w).cloned());
            let Some(s) = style else { return WinStyle { parent, ..Default::default() } };
            use vergence_protocol::window::WinKind;
            let kind = match s.kind {
                WinKind::Normal => Kind::Normal,
                WinKind::Dialog => Kind::Dialog,
                WinKind::Palette => Kind::Palette,
                WinKind::Sidebar => Kind::Sidebar,
                WinKind::Desktop => Kind::Desktop,
            };
            let (min, max) = limits(
                (s.min.0.round() as i32, s.min.1.round() as i32),
                (s.max.0.round() as i32, s.max.1.round() as i32),
            );
            return WinStyle { kind, modal: s.modal, min, max, parent };
        }
        if let Some(x) = w.x11_surface() {
            let parent_id = x.is_transient_for();
            let parent = parent_id.and_then(|id| {
                self.space.elements().find(|e| e.x11_surface().is_some_and(|p| p.window_id() == id)).cloned()
            });
            let kind = x11_kind(x.window_type(), parent.is_some());
            let size = |s: Option<Size<i32, Logical>>| s.map_or((0, 0), |s| (s.w, s.h));
            let (min, max) = limits(size(x.min_size()), size(x.max_size()));
            return WinStyle { kind, modal: x.is_modal() && kind == Kind::Dialog, min, max, parent };
        }
        if let Some(t) = w.toplevel() {
            let surface = t.wl_surface();
            let parent = t.parent().and_then(|p| {
                self.space.elements().find(|e| e.toplevel().is_some_and(|pt| *pt.wl_surface() == p)).cloned()
            });
            let (cached, hint) = compositor::with_states(surface, |s| {
                let cached = *s.cached_state.get::<SurfaceCachedState>().current();
                let hint = s
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .map(|d| d.lock().unwrap().dialog_hint)
                    .unwrap_or_default();
                (cached, hint)
            });
            let (min, max) = limits((cached.min_size.w, cached.min_size.h), (cached.max_size.w, cached.max_size.h));
            let kind = if parent.is_some() { Kind::Dialog } else { Kind::Normal };
            return WinStyle {
                kind,
                modal: kind == Kind::Dialog && hint == ToplevelDialogHint::Modal,
                min,
                max,
                parent,
            };
        }
        WinStyle::default()
    }

    /// Whether a window can be maximized: no largest size, or one that holds
    /// the work area of the screen it is mostly on (window styles milestone 1,
    /// step 2). A fixed-size window never can.
    pub fn maximizable(&self, w: &Win) -> bool {
        let s = self.window_style(w);
        if s.fixed() {
            return false;
        }
        let Some(max) = s.max else { return true };
        let Some(geo) = self.space.element_geometry(w) else { return true };
        let screen = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .max_by_key(|g| g.intersection(geo).map_or(0, |i| i.size.w as i64 * i.size.h as i64));
        let Some(screen) = screen else { return true };
        let area = self.work_area(screen);
        let top = self.title_h_in(w, screen);
        max.w >= area.size.w && max.h >= area.size.h - top
    }

    /// Whether a window can be resized by hand: not one of a fixed size.
    pub fn resizable(&self, w: &Win) -> bool {
        !self.window_style(w).fixed()
    }

    /// A size kept within a window's limits.
    pub fn within_limits(&self, w: &Win, size: Size<i32, Logical>) -> Size<i32, Logical> {
        let s = self.window_style(w);
        let (min, max) = (s.min.unwrap_or_default(), s.max.unwrap_or((i32::MAX, i32::MAX).into()));
        Size::from((size.w.min(max.w).max(min.w.max(1)), size.h.min(max.h).max(min.h.max(1))))
    }

    /// For `vctl windows`: the kind, modal, limits and parent in a few words.
    pub fn describe_style(&self, w: &Win) -> String {
        let s = self.window_style(w);
        let size = |s: Option<Size<i32, Logical>>| s.map_or("-".to_string(), |s| format!("{}x{}", s.w, s.h));
        let parent = s.parent.as_ref().map(|p| format!(" parent {:?}", self.window_title(p))).unwrap_or_default();
        format!(
            "kind={:?}{} min={} max={}{}",
            s.kind,
            if s.modal { " modal" } else { "" },
            size(s.min),
            size(s.max),
            parent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_types_make_kinds() {
        assert_eq!(x11_kind(Some(WmWindowType::Dialog), false), Kind::Dialog);
        assert_eq!(x11_kind(Some(WmWindowType::Utility), true), Kind::Palette);
        assert_eq!(x11_kind(Some(WmWindowType::Utility), false), Kind::Normal, "a palette of nothing is a window");
        assert_eq!(x11_kind(Some(WmWindowType::Splash), false), Kind::Splash);
        assert_eq!(x11_kind(None, true), Kind::Dialog, "transient-for alone");
        assert_eq!(x11_kind(Some(WmWindowType::Normal), false), Kind::Normal);
        assert_eq!(x11_kind(None, false), Kind::Normal);
    }

    #[test]
    fn limits_are_kept_sane() {
        assert_eq!(limits((0, 0), (0, 0)), (None, None));
        assert_eq!(limits((760, 480), (1100, 0)), (Some((760, 480).into()), Some((1100, MAX_LIMIT).into())));
        // A largest size of a few points cannot hide a window.
        assert_eq!(limits((0, 0), (2, 2)).1, Some((LEAST_MAX, LEAST_MAX).into()));
        // Nor below the smallest.
        assert_eq!(limits((500, 0), (300, 0)).1, Some((500, MAX_LIMIT).into()));
        // Absurd sizes are cut.
        assert_eq!(limits((1_000_000, 0), (0, 0)).0, Some((MAX_LIMIT, 0).into()));
        let fixed = WinStyle { min: Some((400, 300).into()), max: Some((400, 300).into()), ..Default::default() };
        assert!(fixed.fixed());
        assert!(!WinStyle::default().fixed());
    }
}
