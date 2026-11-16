//! Live tiles on the dock (dock milestone 1): where they stand, and the
//! desktop's own clock tile. The tiles themselves are
//! vergence_protocol::tile; the server draws them.

use crate::{Rect, running};
use vergence_protocol::tile::{Card, Tile};

/// The place of tile `index` on a dock with `items` applications: after
/// them (applications and tiles keep their own parts; Dave, 2026-11-16).
pub fn slot(dock: Rect, items: usize, index: usize, upright: bool) -> Option<Rect> {
    running::slot(dock, items + index, upright)
}

/// Which of `count` tiles is at a point, if any.
pub fn tile_at(dock: Rect, items: usize, count: usize, upright: bool, x: i32, y: i32) -> Option<usize> {
    (0..count).find(|i| slot(dock, items, *i, upright).is_some_and(|s| x >= s.x && x < s.right() && y >= s.y && y < s.bottom()))
}

/// A moment in local time, as the clock shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 0 to 11.
    pub month: i32,
    pub mday: i32,
    /// 0 is Sunday.
    pub wday: i32,
    pub hour: i32,
    pub min: i32,
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The clock's tile for a local time and the same moment in UTC: the time
/// on the dock with the day under it; the card has the whole date, the
/// time large, and UTC. A click opens Date & Time.
pub fn clock(local: Local, utc: Local) -> Tile {
    let day = DAYS[local.wday.rem_euclid(7) as usize];
    let month = MONTHS[local.month.rem_euclid(12) as usize];
    let hm = |t: Local| format!("{:02}:{:02}", t.hour, t.min);
    Tile {
        id: "clock".into(),
        value: hm(local),
        label: format!("{} {}", &day[..3], local.mday),
        group: "System".into(),
        card: Card {
            title: format!("{day}, {month} {}, {}", local.mday, local.year),
            big: hm(local),
            rows: vec![("UTC".into(), hm(utc))],
            ..Card::default()
        },
        page: "date & time".into(),
        ..Tile::default()
    }
}

#[repr(C)]
struct Tm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn gmtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

/// `secs` since 1970 in local time and in UTC, by localtime_r(3) and
/// gmtime_r(3) (glibc's and FreeBSD's struct tm agree on these fields).
pub fn now_at(secs: i64) -> Option<(Local, Local)> {
    let blank = || Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    let (mut l, mut u) = (blank(), blank());
    // SAFETY: each reads the time and fills a struct we own.
    let ok = unsafe { !localtime_r(&secs, &mut l).is_null() && !gmtime_r(&secs, &mut u).is_null() };
    let take = |t: &Tm| Local { year: t.year + 1900, month: t.mon, mday: t.mday, wday: t.wday, hour: t.hour, min: t.min };
    ok.then(|| (take(&l), take(&u)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_stand_after_the_applications() {
        let strip = Rect::new(1232, 0, 48, 720);
        assert_eq!(slot(strip, 3, 0, true), Some(Rect::new(1232, 192, 48, 48)));
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 200), Some(0));
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 250), Some(1));
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 150), None, "an application's place");
        assert_eq!(tile_at(strip, 3, 2, true, 1250, 300), None, "past the last tile");
    }

    #[test]
    fn the_clock_says_the_time_and_the_day() {
        let local = Local { year: 2026, month: 10, mday: 16, wday: 1, hour: 8, min: 5 };
        let utc = Local { hour: 14, ..local };
        let t = clock(local, utc);
        assert_eq!((t.value.as_str(), t.label.as_str()), ("08:05", "Mon 16"));
        assert_eq!(t.card.title, "Monday, November 16, 2026");
        assert_eq!(t.card.rows, [("UTC".to_string(), "14:05".to_string())]);
        assert_eq!(t.page, "date & time");
    }

    #[test]
    fn utc_at_the_start_of_1970() {
        let (_, u) = now_at(0).unwrap();
        assert_eq!((u.year, u.month, u.mday, u.wday, u.hour, u.min), (1970, 0, 1, 4, 0, 0));
    }
}
