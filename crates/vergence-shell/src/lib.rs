//! The shell's arithmetic and, later, its knowledge of applications
//! (whitepaper §6.9; docs/shelf-milestone-1.md).
//!
//! Step 2 is the applications a system has ([`apps`], from desktop
//! entries read by [`desktop`]) and their icons ([`icons`]).
//!
//! Step 1 is here: the dock, along an edge of its screen or floating on it
//! ([`Placement`]), the band it stands in at an edge, the **work area** the band leaves (what maximizing, placing and fitting windows,
//! the desktop menu and notification cards keep inside), and moving a
//! remembered window clear of the strip. Rectangles are in points, in the
//! desktop's own space (a screen's rectangle is where the arrangement put
//! it).

pub mod actions;
pub mod appmenu;
pub mod apps;
pub mod autostart;
pub mod desktop;
pub mod icons;
pub mod launcher;
pub mod open;
pub mod overview;
pub mod running;
pub mod session;
pub mod snap;
pub mod tiles;

/// A rectangle in points: its top-left corner and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// True if the two share any area.
    pub const fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    /// True if `other` lies wholly inside.
    pub const fn holds(&self, other: &Rect) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
    }
}

/// The shelf's width, in points (the same physical width on every screen),
/// and the room it keeps from the screen's edge and from the windows unless
/// the settings say another.
pub const STRIP_W: i32 = vergence_look::SHELF_W as i32;
pub const GAP: i32 = vergence_look::SHELF_GAP as i32;
/// The widest room the settings may ask for, and how near an edge the dock
/// must be dropped to settle along it.
pub const MAX_GAP: i32 = 24;
pub const SNAP: i32 = 40;
/// The sizes the person may give the dock (its thickness, and each place
/// along it), in points: 48 unless the settings say another (Dave,
/// 2026-11-17: "the dock's size, chosen by the person").
pub const MIN_SIZE: i32 = 32;
pub const MAX_SIZE: i32 = 96;

/// An edge of a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Where the dock is on its screen (Dave, 2026-11-10: it can be dragged; "let's
/// have it all, adjustable in settings").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Along an edge, in the middle of it: upright at the sides, level at
    /// the top and bottom. Windows keep clear of it if the settings say so.
    Edge(Edge),
    /// Where it was put, over the windows: its top-left corner from the
    /// screen's, and whether it stands upright or lies level.
    Free { x: i32, y: i32, upright: bool },
}

/// The dock's place and the settings that go with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub place: Place,
    /// The room between the dock and its edge, and between it and the windows.
    pub gap: i32,
    /// At an edge: windows keep out of the dock's whole column (or row).
    pub keep_clear: bool,
    /// Its thickness and each place's length, in points (STRIP_W unless
    /// the settings say another; MIN_SIZE to MAX_SIZE).
    pub size: i32,
}

impl Default for Placement {
    fn default() -> Placement {
        // The bottom edge by default (Dave, 2026-11-16, with the launcher as a start menu).
        Placement { place: Place::Edge(Edge::Bottom), gap: GAP, keep_clear: true, size: STRIP_W }
    }
}

impl Placement {
    /// True if the dock stands upright (its items one under the other).
    pub fn upright(&self) -> bool {
        match self.place {
            Place::Edge(Edge::Left | Edge::Right) => true,
            Place::Edge(Edge::Top | Edge::Bottom) => false,
            Place::Free { upright, .. } => upright,
        }
    }

    fn gap(&self) -> i32 {
        self.gap.clamp(0, MAX_GAP)
    }

    /// The dock's thickness and each place's length, within the limits.
    pub fn size(&self) -> i32 {
        self.size.clamp(MIN_SIZE, MAX_SIZE)
    }
}

/// The band of its screen an edge's dock stands in: the whole length of the
/// edge, as deep as the shelf and the room on both its sides. None for a dock
/// that floats, and for a screen too small to give up a band (narrower, or
/// shorter, than three shelves).
pub fn column(screen: Rect, at: &Placement) -> Option<Rect> {
    let Place::Edge(edge) = at.place else { return None };
    let side = at.size();
    let deep = side + 2 * at.gap();
    let room = if at.upright() { screen.w } else { screen.h };
    if room < 3 * side || screen.w <= 0 || screen.h <= 0 {
        return None;
    }
    Some(match edge {
        Edge::Right => Rect::new(screen.right() - deep, screen.y, deep, screen.h),
        Edge::Left => Rect::new(screen.x, screen.y, deep, screen.h),
        Edge::Top => Rect::new(screen.x, screen.y, screen.w, deep),
        Edge::Bottom => Rect::new(screen.x, screen.bottom() - deep, screen.w, deep),
    })
}

/// The dock itself (Dave, 2026-11-10: "like a dock"): a panel as long as
/// the apps button's place and `items` places after it. At an edge it is in
/// the middle of the edge, a gap in from it; floating, it is where it was
/// put, moved only as far as keeps it whole on its screen. On a screen too
/// small for all its items it is as long as the screen allows, and holds
/// the items that fit.
pub fn dock(screen: Rect, at: &Placement, items: usize) -> Option<Rect> {
    let (upright, gap, side) = (at.upright(), at.gap(), at.size());
    let along = if upright { screen.h } else { screen.w };
    let places = (items as i32 + 1).min(((along - 2 * gap) / side).max(1));
    let long = places * side;
    let (w, h) = if upright { (side, long) } else { (long, side) };
    match at.place {
        Place::Edge(edge) => {
            let band = column(screen, at)?;
            Some(match edge {
                Edge::Right | Edge::Left => Rect::new(band.x + gap, band.y + (band.h - h) / 2, w, h),
                Edge::Top | Edge::Bottom => Rect::new(band.x + (band.w - w) / 2, band.y + gap, w, h),
            })
        }
        Place::Free { x, y, .. } => {
            if screen.w < w || screen.h < h {
                return None;
            }
            Some(Rect::new(
                (screen.x + x).clamp(screen.x, screen.right() - w),
                (screen.y + y).clamp(screen.y, screen.bottom() - h),
                w,
                h,
            ))
        }
    }
}

/// What a screen with the shelf leaves for windows: all of it but the band
/// the dock stands in, when it is at an edge and windows are to keep clear.
pub fn work_area(screen: Rect, at: &Placement) -> Rect {
    let (Some(band), true, Place::Edge(edge)) = (column(screen, at), at.keep_clear, at.place) else { return screen };
    match edge {
        Edge::Right => Rect::new(screen.x, screen.y, screen.w - band.w, screen.h),
        Edge::Left => Rect::new(screen.x + band.w, screen.y, screen.w - band.w, screen.h),
        Edge::Top => Rect::new(screen.x, screen.y + band.h, screen.w, screen.h - band.h),
        Edge::Bottom => Rect::new(screen.x, screen.y, screen.w, screen.h - band.h),
    }
}

/// A window's remembered place on a screen whose dock now keeps windows
/// clear: moved out of the dock's band, as long as it then still lies on the
/// work area. A window too large for that, or one the band does not touch,
/// is given back as it was (the caller's own fitting deals with the first).
pub fn clear_of_dock(window: Rect, screen: Rect, at: &Placement) -> Rect {
    let (Some(band), true, Place::Edge(edge)) = (column(screen, at), at.keep_clear, at.place) else { return window };
    if !window.overlaps(&band) {
        return window;
    }
    let moved = match edge {
        Edge::Right => Rect::new(band.x - window.w, window.y, window.w, window.h),
        Edge::Left => Rect::new(band.right(), window.y, window.w, window.h),
        Edge::Top => Rect::new(window.x, band.bottom(), window.w, window.h),
        Edge::Bottom => Rect::new(window.x, band.y - window.h, window.w, window.h),
    };
    if work_area(screen, at).holds(&moved) { moved } else { window }
}

/// Where a dock that was dragged and let go settles: `dropped` is its
/// rectangle as it was let go, `screens` the screens there are. It belongs to
/// the screen its middle is on (the nearest, if on none). If one of its sides
/// is within [`SNAP`] of that screen's edge (or past it) it settles along the
/// nearest such edge; else it floats where it was dropped, keeping the way it
/// stood.
/// Given back: the screen's place in `screens`, and the place on it.
pub fn settle(dropped: Rect, upright: bool, screens: &[Rect]) -> Option<(usize, Place)> {
    let (cx, cy) = (dropped.x + dropped.w / 2, dropped.y + dropped.h / 2);
    let away = |s: &Rect| {
        let dx = (s.x - cx).max(cx - (s.right() - 1)).max(0) as i64;
        let dy = (s.y - cy).max(cy - (s.bottom() - 1)).max(0) as i64;
        dx * dx + dy * dy
    };
    let (index, screen) = screens.iter().enumerate().min_by_key(|(_, s)| away(s))?;
    let edges = [
        (dropped.x - screen.x, Edge::Left),
        (screen.right() - dropped.right(), Edge::Right),
        (dropped.y - screen.y, Edge::Top),
        (screen.bottom() - dropped.bottom(), Edge::Bottom),
    ];
    let (near, edge) = edges.into_iter().map(|(d, e)| (d.max(0), e)).min_by_key(|(d, _)| *d)?;
    if near <= SNAP {
        return Some((index, Place::Edge(edge)));
    }
    Some((index, Place::Free { x: dropped.x - screen.x, y: dropped.y - screen.y, upright }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hubble's two screens as the server lays them out: the 14" 4K at scale 3
    // (1280 x 720 points) and the ASUS at 1 (1920 x 1080) to its right.
    const SMALL: Rect = Rect::new(0, 180, 1280, 720);
    const LARGE: Rect = Rect::new(1280, 0, 1920, 1080);

    // The arithmetic is checked with a gap of 6, whatever the default is.
    const T_GAP: i32 = 6;
    const RIGHT: Placement = Placement { place: Place::Edge(Edge::Right), gap: T_GAP, keep_clear: true, size: STRIP_W };

    fn at(edge: Edge) -> Placement {
        Placement { place: Place::Edge(edge), ..RIGHT }
    }

    #[test]
    fn the_band_runs_along_its_edge_and_is_the_same_depth_everywhere() {
        assert_eq!(Placement::default(), Placement { gap: GAP, ..at(Edge::Bottom) }, "the bottom edge by default");
        assert_eq!(column(SMALL, &RIGHT), Some(Rect::new(1280 - 60, 180, 60, 720)));
        assert_eq!(column(LARGE, &RIGHT), Some(Rect::new(1280 + 1920 - 60, 0, 60, 1080)));
        assert_eq!(column(LARGE, &at(Edge::Left)), Some(Rect::new(1280, 0, 60, 1080)));
        assert_eq!(column(LARGE, &at(Edge::Top)), Some(Rect::new(1280, 0, 1920, 60)));
        assert_eq!(column(LARGE, &at(Edge::Bottom)), Some(Rect::new(1280, 1020, 1920, 60)));
        assert_eq!(column(Rect::new(0, 0, 100, 500), &RIGHT), None, "too narrow a screen has no shelf at its side");
        assert_eq!(column(Rect::new(0, 0, 500, 100), &at(Edge::Top)), None, "nor too short a one along its top");
        // The settings' gap, kept within reason.
        assert_eq!(column(LARGE, &Placement { gap: 20, ..RIGHT }).unwrap().w, 88);
        assert_eq!(column(LARGE, &Placement { gap: -5, ..RIGHT }).unwrap().w, 48);
        assert_eq!(column(LARGE, &Placement { gap: 5000, ..RIGHT }).unwrap().w, 48 + 2 * MAX_GAP);
        // A dock that floats stands in no band.
        assert_eq!(column(LARGE, &Placement { place: Place::Free { x: 10, y: 10, upright: true }, ..RIGHT }), None);
    }

    #[test]
    fn the_dock_is_as_long_as_what_it_holds() {
        // The apps button's place and three items: four places, in the middle of the edge.
        assert_eq!(dock(LARGE, &RIGHT, 3), Some(Rect::new(1280 + 1920 - 54, (1080 - 192) / 2, 48, 192)));
        assert_eq!(dock(LARGE, &at(Edge::Left), 3), Some(Rect::new(1280 + 6, (1080 - 192) / 2, 48, 192)));
        // Level along the top and the bottom.
        assert_eq!(dock(LARGE, &at(Edge::Top), 3), Some(Rect::new(1280 + (1920 - 192) / 2, 6, 192, 48)));
        assert_eq!(dock(LARGE, &at(Edge::Bottom), 3), Some(Rect::new(1280 + (1920 - 192) / 2, 1080 - 54, 192, 48)));
        // Nothing running: the apps button's place alone.
        assert_eq!(dock(SMALL, &RIGHT, 0), Some(Rect::new(1280 - 54, 180 + (720 - 48) / 2, 48, 48)));
        // More than the screen can hold: as long as it allows, a gap from both ends.
        let full = dock(SMALL, &RIGHT, 40).unwrap();
        assert_eq!(full.h, 14 * 48);
        assert!(full.y >= SMALL.y + T_GAP && full.bottom() <= SMALL.bottom() - T_GAP);
        // It stands inside its band, a gap from the screen's edge and from the windows.
        let (d, band) = (dock(LARGE, &RIGHT, 5).unwrap(), column(LARGE, &RIGHT).unwrap());
        assert!(band.holds(&d) && d.x - band.x == T_GAP && band.right() - d.right() == T_GAP);
        assert_eq!(dock(Rect::new(0, 0, 100, 500), &RIGHT, 3), None);
        // The places of its items are the ones `running` gives for it.
        assert_eq!(running::slot(d, 0, true), Some(Rect::new(d.x, d.y + 48, 48, 48)));
        assert!(running::slot(d, 4, true).is_some() && running::slot(d, 5, true).is_none());
        let level = dock(LARGE, &at(Edge::Bottom), 3).unwrap();
        assert_eq!(running::slot(level, 0, false), Some(Rect::new(level.x + 48, level.y, 48, 48)));
        assert!(running::slot(level, 2, false).is_some() && running::slot(level, 3, false).is_none());
    }

    #[test]
    fn the_dock_takes_the_size_it_is_given() {
        // 64 points: thicker, each place as long, the band as deep, icons two thirds.
        let big = Placement { size: 64, ..RIGHT };
        let d = dock(LARGE, &big, 3).unwrap();
        assert_eq!((d.w, d.h), (64, 4 * 64));
        assert_eq!(column(LARGE, &big).unwrap().w, 64 + 2 * T_GAP);
        let s = running::slot(d, 0, true).unwrap();
        assert_eq!(s, Rect::new(d.x, d.y + 64, 64, 64));
        assert_eq!(running::icon_in(s).w, 42);
        assert_eq!(running::item_at(d, 3, true, d.x + 10, d.y + 64 * 2 + 5), Some(1));
        assert_eq!(running::gap_at(d, 3, true, d.x + 10, d.y + 64 * 3 + 5), 2);
        // The usual size gives the usual icon; the limits hold.
        assert_eq!(running::icon_in(Rect::new(0, 0, 48, 48)).w, running::ICON);
        assert_eq!(Placement { size: 4, ..RIGHT }.size(), MIN_SIZE);
        assert_eq!(Placement { size: 500, ..RIGHT }.size(), MAX_SIZE);
        // Level, along the bottom.
        let level = dock(LARGE, &Placement { size: 64, ..at(Edge::Bottom) }, 2).unwrap();
        assert_eq!((level.w, level.h), (3 * 64, 64));
    }

    #[test]
    fn a_floating_dock_is_where_it_was_put_and_whole_on_its_screen() {
        let free = |x: i32, y: i32, upright: bool| Placement { place: Place::Free { x, y, upright }, ..RIGHT };
        assert_eq!(dock(LARGE, &free(300, 200, true), 2), Some(Rect::new(1280 + 300, 200, 48, 144)));
        assert_eq!(dock(LARGE, &free(300, 200, false), 2), Some(Rect::new(1280 + 300, 200, 144, 48)));
        // Put past the screen's far corner, or before its near one: brought back whole.
        assert_eq!(dock(LARGE, &free(5000, 5000, true), 2), Some(Rect::new(1280 + 1920 - 48, 1080 - 144, 48, 144)));
        assert_eq!(dock(LARGE, &free(-50, -50, true), 2), Some(Rect::new(1280, 0, 48, 144)));
        // It reserves nothing: windows have the whole screen.
        assert_eq!(work_area(LARGE, &free(300, 200, true)), LARGE);
        let window = Rect::new(1280 + 250, 150, 400, 300);
        assert_eq!(clear_of_dock(window, LARGE, &free(300, 200, true)), window);
    }

    #[test]
    fn the_work_area_is_the_screen_less_the_band() {
        assert_eq!(work_area(SMALL, &RIGHT), Rect::new(0, 180, 1220, 720));
        assert_eq!(work_area(LARGE, &RIGHT), Rect::new(1280, 0, 1860, 1080));
        assert_eq!(work_area(LARGE, &at(Edge::Left)), Rect::new(1340, 0, 1860, 1080));
        assert_eq!(work_area(LARGE, &at(Edge::Top)), Rect::new(1280, 60, 1920, 1020));
        assert_eq!(work_area(LARGE, &at(Edge::Bottom)), Rect::new(1280, 0, 1920, 1020));
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let (w, band) = (work_area(LARGE, &at(edge)), column(LARGE, &at(edge)).unwrap());
            assert!(!w.overlaps(&band) && LARGE.holds(&w) && LARGE.holds(&band), "{edge:?}");
        }
        assert_eq!(work_area(Rect::new(0, 0, 100, 500), &RIGHT), Rect::new(0, 0, 100, 500));
        // Windows need not keep clear: then they have the whole screen.
        assert_eq!(work_area(LARGE, &Placement { keep_clear: false, ..RIGHT }), LARGE);
    }

    #[test]
    fn a_remembered_window_is_moved_clear_of_the_band() {
        // Its right 40 points would be in the band: moved left by that much.
        let w = Rect::new(1280 + 1400, 100, 500, 400);
        let moved = clear_of_dock(w, LARGE, &RIGHT);
        assert_eq!(moved, Rect::new(1280 + 1860 - 500, 100, 500, 400));
        assert!(work_area(LARGE, &RIGHT).holds(&moved));
        // The other edges move it the other ways.
        assert_eq!(
            clear_of_dock(Rect::new(1290, 100, 500, 400), LARGE, &at(Edge::Left)),
            Rect::new(1340, 100, 500, 400)
        );
        assert_eq!(clear_of_dock(Rect::new(1500, 10, 500, 400), LARGE, &at(Edge::Top)), Rect::new(1500, 60, 500, 400));
        assert_eq!(
            clear_of_dock(Rect::new(1500, 700, 500, 400), LARGE, &at(Edge::Bottom)),
            Rect::new(1500, 620, 500, 400)
        );
        // One the band does not touch stays.
        let clear = Rect::new(1300, 50, 600, 400);
        assert_eq!(clear_of_dock(clear, LARGE, &RIGHT), clear);
        // One wider than the work area is left for the caller to fit.
        let wide = Rect::new(1290, 0, 1900, 500);
        assert_eq!(clear_of_dock(wide, LARGE, &RIGHT), wide);
        // A window on the other screen is not the shelf's business.
        let other = Rect::new(600, 300, 600, 400);
        assert_eq!(clear_of_dock(other, LARGE, &RIGHT), other);
        // Nor is any window, when windows need not keep clear.
        assert_eq!(clear_of_dock(w, LARGE, &Placement { keep_clear: false, ..RIGHT }), w);
    }

    #[test]
    fn a_dropped_dock_settles_along_a_near_edge_or_floats() {
        let screens = [SMALL, LARGE];
        let upright = |x: i32, y: i32| Rect::new(x, y, 48, 144);
        // Near the large screen's left edge (which is also where the small one ends).
        assert_eq!(settle(upright(1290, 400), true, &screens), Some((1, Place::Edge(Edge::Left))));
        assert_eq!(settle(upright(3120, 400), true, &screens), Some((1, Place::Edge(Edge::Right))));
        assert_eq!(settle(upright(2200, 10), true, &screens), Some((1, Place::Edge(Edge::Top))));
        assert_eq!(settle(upright(2200, 900), true, &screens), Some((1, Place::Edge(Edge::Bottom))));
        // The small screen's own edges.
        assert_eq!(settle(upright(20, 500), true, &screens), Some((0, Place::Edge(Edge::Left))));
        assert_eq!(settle(upright(600, 190), true, &screens), Some((0, Place::Edge(Edge::Top))));
        // Away from every edge: it floats where it was dropped, as it stood.
        assert_eq!(
            settle(upright(2000, 400), true, &screens),
            Some((1, Place::Free { x: 720, y: 400, upright: true }))
        );
        assert_eq!(
            settle(Rect::new(500, 500, 144, 48), false, &screens),
            Some((0, Place::Free { x: 500, y: 320, upright: false }))
        );
        // Dropped off every screen (above the small one): the nearest screen's nearest edge.
        assert_eq!(settle(upright(600, 20), true, &screens), Some((0, Place::Edge(Edge::Top))));
        assert_eq!(settle(upright(0, 0), true, &[]), None);
        // What it settles to is a place the dock can be drawn at.
        for (x, y) in [(1290, 400), (2000, 400), (600, 20), (3100, 1000)] {
            let (screen, place) = settle(upright(x, y), true, &screens).unwrap();
            let d = dock(screens[screen], &Placement { place, ..RIGHT }, 2).unwrap();
            assert!(screens[screen].holds(&d), "{x},{y}: {d:?}");
        }
    }
}
