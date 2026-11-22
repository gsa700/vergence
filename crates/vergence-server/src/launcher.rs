// The launcher (menus milestone 1, 2026-11-16; before it, the applications
// popup of shelf milestones 1 and 2, a grid in the middle of the screen).
// Dave: the grid felt "disconnected"; the launcher is a start menu, a text
// menu pinned to the dock's button as menus are, in the menus' one look.
// Opened by the button at the head of the dock or by the Super key tapped
// alone. It is the app menu's panel (appmenu.rs) with these lines, and a
// search field over them that finds an application in any submenu:
//
//   Vergence's own applications (File Viewer, Text Editor, Terminal...)
//   ---
//   Internet ›, Office ›, Graphics ›, ... (each category that has one), Other ›
//   ---
//   Lock Screen
//   Log Out…   (the session dialog: Log Out, Sleep, Restart, Shut Down)
//
// Every line is carried out here (menu.rs `menu_run`), no program told.
// Which category an application is in, and Vergence's own, are
// vergence-shell's (launcher.rs there).

use smithay::utils::{Logical, Point, Rectangle};
use vergence_protocol::window::{MAX_MENU_LINES, MENU_SERVER_IDS, MenuLine};
use vergence_shell::{Edge, Place};

use crate::{Server, menu::Entry, shelf::Act};

#[derive(Default)]
pub struct Launcher {}

impl Server {
    pub fn launcher_is_open(&self) -> bool {
        self.app_menu_is_launcher()
    }

    pub fn launcher_close(&mut self) {
        if self.app_menu_is_launcher() {
            tracing::info!("apps: closed");
            self.app_menu_close();
        }
    }

    /// The launcher's lines, and what each does.
    fn launcher_lines(&self) -> (Vec<MenuLine>, Vec<(u32, Entry)>) {
        let apps = &self.shelf.apps;
        let mut own: Vec<(u32, Entry)> = Vec::new();
        let item = |own: &mut Vec<(u32, Entry)>, label: String, e: Entry| {
            let id = MENU_SERVER_IDS + own.len() as u32;
            own.push((id, e));
            MenuLine::item(id, &label, "")
        };
        let name = |i: usize| -> String {
            let n = &apps[i].name;
            if n.trim().is_empty() {
                apps[i].id.trim_end_matches(".desktop").to_string()
            } else {
                n.chars().filter(|c| !c.is_control()).take(60).collect()
            }
        };
        let mut lines = Vec::new();
        for i in vergence_shell::launcher::own(apps) {
            lines.push(item(&mut own, name(i), Entry::Shelf(Act::Start(apps[i].id.clone()), name(i))));
        }
        lines.push(MenuLine::Separator);
        // Each category that has an application, in the table's order; those in none last.
        let all = vergence_shell::launcher::search(apps, "");
        for (t, _) in vergence_shell::launcher::tabs(apps, &all) {
            let mut sub = Vec::new();
            for i in vergence_shell::launcher::search_in(apps, "", Some(t)).into_iter().take(MAX_MENU_LINES) {
                sub.push(item(&mut own, name(i), Entry::Shelf(Act::Start(apps[i].id.clone()), name(i))));
            }
            if !sub.is_empty() {
                lines.push(MenuLine::Sub { label: vergence_shell::launcher::TABS[t].into(), lines: sub });
            }
        }
        let other: Vec<usize> = all
            .iter()
            .copied()
            .filter(|i| vergence_shell::launcher::tab_of(&apps[*i]).is_none())
            .take(MAX_MENU_LINES)
            .collect();
        if !other.is_empty() {
            let sub = other
                .into_iter()
                .map(|i| item(&mut own, name(i), Entry::Shelf(Act::Start(apps[i].id.clone()), name(i))))
                .collect();
            lines.push(MenuLine::Sub { label: "Other".into(), lines: sub });
        }
        lines.push(MenuLine::Separator);
        lines.push(item(&mut own, "Lock Screen".into(), Entry::Lock));
        lines.push(item(&mut own, "Log Out…".into(), Entry::LogOut));
        (lines, own)
    }

    /// Open the launcher (or close it, if open), pinned to the dock's
    /// button: above it on a dock at the bottom (growing upward), below it
    /// on one at the top, beside it on one at a side; in the middle of the
    /// pointer's screen if there is no dock.
    pub fn launcher_toggle(&mut self) {
        // No launcher at the login screen (review C3).
        if crate::greeter() {
            return;
        }
        if self.launcher_is_open() {
            return self.launcher_close();
        }
        let (lines, own) = self.launcher_lines();
        let h = Server::launcher_height(&lines);
        let w = vergence_shell::appmenu::WIDTH;
        let gap = vergence_look::SHELF_POPUP_GAP as i32;
        let items = self.shelf_items().len();
        let dock = self.shelf_dock(items);
        let screen = dock
            .and_then(|d| self.space.outputs().filter_map(|o| self.space.output_geometry(o)).find(|g| g.overlaps(d)))
            .or_else(|| self.output_at_pointer())
            .or_else(|| self.shelf_screen());
        let Some(screen) = screen else { return };
        let area = self.work_area(screen);
        let placement = self.shelf.config.placement();
        let (at, bottom): (Point<i32, Logical>, Option<i32>) = match dock {
            Some(d) => {
                // The button: the dock's head.
                let b = Rectangle::<i32, Logical>::new(d.loc, (placement.size(), placement.size()).into());
                let mid = (screen.loc.x + screen.size.w / 2, screen.loc.y + screen.size.h / 2);
                let side = match placement.place {
                    Place::Edge(e) => e,
                    // A free dock: away from the nearer edge.
                    Place::Free { upright: true, .. } => {
                        if b.loc.x < mid.0 {
                            Edge::Left
                        } else {
                            Edge::Right
                        }
                    }
                    Place::Free { .. } => {
                        if b.loc.y < mid.1 {
                            Edge::Top
                        } else {
                            Edge::Bottom
                        }
                    }
                };
                let top_at = |y: i32| y.min(area.loc.y + area.size.h - h).max(area.loc.y);
                match side {
                    Edge::Bottom => ((b.loc.x, 0).into(), Some(d.loc.y - gap)),
                    Edge::Top => ((b.loc.x, d.loc.y + d.size.h + gap).into(), None),
                    Edge::Left => ((d.loc.x + d.size.w + gap, top_at(b.loc.y)).into(), None),
                    Edge::Right => ((d.loc.x - gap - w, top_at(b.loc.y)).into(), None),
                }
            }
            None => ((area.loc.x + (area.size.w - w) / 2, area.loc.y + (area.size.h - h) / 2).into(), None),
        };
        tracing::info!("apps: opened");
        self.app_menu_launcher(at, bottom, area, lines, own);
    }

    /// For `vctl apps`: whether it is open.
    pub fn launcher_report(&self) -> String {
        if self.launcher_is_open() { "open".into() } else { "closed".into() }
    }
}
