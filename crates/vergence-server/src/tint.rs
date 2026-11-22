// Windows tinted with the wallpaper behind them (docs/window-styles-milestone-1.md,
// step 6; as macOS's "Tint window background with wallpaper color", which Dave
// asked about on 2026-11-14).
//
// The server knows each screen's wallpaper (wallpaper.rs keeps its colors
// averaged over a 32 x 18 grid) and where every window is. A native window is
// told the average color of the wallpaper under it (`WinEvent::Tint`) when it
// changes enough to see, looked at each frame but at most five times a second
// for one window; the toolkit mixes a few percent of it into the window's
// grounds. On in dark style only, where it shows (Dave: (a)); off with Reduce
// transparency and High contrast, and by the Appearance switch (desktop.toml
// `[appearance] wallpaper-tint = false`).

use std::time::{Duration, Instant};

use smithay::utils::{IsAlive, Logical, Point};

use crate::{
    Server,
    wallpaper::{GRID_H, GRID_W},
    win::Win,
};

/// How often one window's tint is worked out again at most.
const EVERY: Duration = Duration::from_millis(200);

/// A tint sent: the color (None: no tint), where the window was, the grids' number.
struct Sent {
    rgb: Option<u32>,
    at: Point<i32, Logical>,
    grids: u64,
    when: Instant,
}

#[derive(Default)]
pub struct Tints {
    sent: std::collections::HashMap<Win, Sent>,
}

/// Two colors near enough that a change between them would not show.
fn near(a: Option<u32>, b: Option<u32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let d = |s: u32| (((a >> s) & 0xff) as i32 - ((b >> s) & 0xff) as i32).abs();
            d(16) + d(8) + d(0) <= 9
        }
        (None, None) => true,
        _ => false,
    }
}

fn tint_wanted_from_file() -> bool {
    let Some(home) = std::env::var_os("HOME") else { return true };
    let text =
        std::fs::read_to_string(std::path::Path::new(&home).join(".config/vergence/desktop.toml")).unwrap_or_default();
    text.parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("appearance").and_then(|a| a.get("wallpaper-tint")).and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

impl Server {
    /// desktop.toml changed (or the session started): the switch again.
    pub fn tint_reload(&mut self) {
        self.tint_on = tint_wanted_from_file();
    }

    /// Whether windows are tinted now: the switch, dark style, neither Reduce
    /// transparency nor High contrast.
    fn tinting(&self) -> bool {
        let high_contrast =
            self.settings.current.get(&(crate::settings::APPEARANCE.to_string(), "contrast".to_string()))
                == Some(&vergence_protocol::bus::SettingValue::U32(1));
        self.tint_on && !self.light_style() && !self.reduce_transparency() && !high_contrast
    }

    /// The wallpaper's average color under a window: the grid cells its
    /// rectangle covers on the screen most of it is on.
    fn wallpaper_under(&self, w: &Win) -> Option<u32> {
        let geo = self.space.element_geometry(w)?;
        let (out, og) = self
            .space
            .outputs()
            .filter_map(|o| Some((o.clone(), self.space.output_geometry(o)?)))
            .max_by_key(|(_, g)| g.intersection(geo).map_or(0, |i| i.size.w as i64 * i.size.h as i64))?;
        let grid = self.wallpaper.as_ref()?.grids.get(&out.name())?;
        let cell = |v: i32, from: i32, size: i32, n: usize| {
            (((v - from).clamp(0, size - 1) as i64 * n as i64) / size.max(1) as i64) as usize
        };
        let (x0, x1) =
            (cell(geo.loc.x, og.loc.x, og.size.w, GRID_W), cell(geo.loc.x + geo.size.w, og.loc.x, og.size.w, GRID_W));
        let (y0, y1) =
            (cell(geo.loc.y, og.loc.y, og.size.h, GRID_H), cell(geo.loc.y + geo.size.h, og.loc.y, og.size.h, GRID_H));
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        for y in y0..=y1.min(GRID_H - 1) {
            for x in x0..=x1.min(GRID_W - 1) {
                let c = grid[y * GRID_W + x];
                (r, g, b, n) = (r + c[0] as u32, g + c[1] as u32, b + c[2] as u32, n + 1);
            }
        }
        (n > 0).then(|| ((r / n) << 16) | ((g / n) << 8) | (b / n))
    }

    /// Each frame: tell native windows their tint when it changed.
    pub fn tint_tick(&mut self) {
        let on = self.tinting();
        let grids = self.wallpaper.as_ref().map_or(0, |w| w.grid_gen);
        self.tints.sent.retain(|w, _| w.alive());
        let windows: Vec<Win> = self.space.elements().filter(|w| w.native().is_some()).cloned().collect();
        for w in windows {
            let Some(at) = self.space.element_location(&w) else { continue };
            let last = self.tints.sent.get(&w).map(|s| (s.at, s.grids, s.rgb, s.when));
            let moved = last.is_none_or(|(a, g, r, _)| a != at || g != grids || (r.is_some() != on));
            if !moved || last.is_some_and(|(.., when)| when.elapsed() < EVERY) {
                continue;
            }
            let rgb = if on { self.wallpaper_under(&w) } else { None };
            let changed = last.is_none_or(|(_, _, r, _)| !near(r, rgb));
            if changed && let Some(n) = w.native() {
                self.bus_send(
                    n.client,
                    vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::Tint {
                        window: n.id,
                        rgb,
                    }),
                );
            }
            let rgb = if changed { rgb } else { last.and_then(|(_, _, r, _)| r) };
            self.tints.sent.insert(w, Sent { rgb, at, grids, when: Instant::now() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_change_of_color_is_not_sent() {
        assert!(near(Some(0x203040), Some(0x213141)));
        assert!(!near(Some(0x203040), Some(0x2a3a4a)));
        assert!(near(None, None));
        assert!(!near(None, Some(0)));
    }
}
