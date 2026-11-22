// The desktop (files milestone 1, step 9; docs/files-milestone-1.md §6:
// included as Windows does it). The icons of ~/Desktop
// are drawn by File Viewer in a window of the Desktop kind (bus v17,
// `vergence-files --desktop`, app id vergence-desktop), which the server
// sizes to the main screen's work area and keeps under every other window:
// never raised, no bar or border, out of the dock, Alt+Tab, window memory
// and the session's list; a click on it gives it the keyboard as any
// window. A right press on it File Viewer answers: it asks the server for
// its menu (bus v18 WinRequest::ContextMenu), the icon's commands or, on the
// bare desktop, New folder, Paste... with the desktop menu's own entries.
//
// The session starts it (session.rs, at the session's start) unless
// desktop.toml `[desktop] icons = false` (Settings > Desktop), and starts it
// again whenever it exits (so tools/install.sh can stop the old program and
// the new one comes up, and a crash comes back too; see
// docs/drives-milestone-1.md), up to five times a minute, never while the
// session exits.
//
// Super+D (the seat's ShowDesktop): every window minimized out of the way;
// again, the same ones back.
//
// The dock's "Add to desktop": a copy of the application's desktop entry in
// ~/Desktop, executable, so it is a trusted launcher there.

use smithay::utils::{Logical, Rectangle};

use crate::{Server, win::Win};

/// Set when the session begins to exit (exit.rs): the desktop is not started again.
pub static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether desktop.toml asks for the desktop's icons (on unless `icons = false`).
pub fn icons_wanted() -> bool {
    let text = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let table = text.parse::<toml::Table>().unwrap_or_default();
    table.get("desktop").and_then(|d| d.get("icons")).and_then(|v| v.as_bool()).unwrap_or(true)
}

/// The user's Desktop folder (the XDG user-dirs file, else ~/Desktop).
pub fn desktop_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let config =
        std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let said = text
        .lines()
        .find(|l| l.trim_start().starts_with("XDG_DESKTOP_DIR="))
        .and_then(|l| l.split_once('='))
        .map(|(_, v)| v.trim().trim_matches('"').to_string());
    Some(match said {
        Some(v) if v.starts_with("$HOME/") => home.join(&v[6..]),
        Some(v) if v.starts_with('/') && v != home.to_string_lossy() => std::path::PathBuf::from(v),
        _ => home.join("Desktop"),
    })
}

impl Server {
    /// Whether a window is the desktop's.
    pub fn is_desktop(&self, w: &Win) -> bool {
        self.window_style(w).kind == crate::style::Kind::Desktop
    }

    /// Where the desktop window goes: the main screen's work area (the dock's
    /// band left out), else the first screen.
    fn desktop_area(&self) -> Option<Rectangle<i32, Logical>> {
        let screen =
            self.shelf_screen().or_else(|| self.space.outputs().next().and_then(|o| self.space.output_geometry(o)))?;
        Some(self.work_area(screen))
    }

    /// A desktop window put in its place, at its size, under every window.
    pub fn desktop_place(&mut self, w: &Win) {
        let (Some(area), Win::Native(n)) = (self.desktop_area(), w) else { return };
        let n = n.clone();
        self.put(w.clone(), area.loc, false);
        self.native_configure(&n, area.size, false);
        self.desktop_to_bottom();
        tracing::info!("desktop: its window at {}x{} from {},{}", area.size.w, area.size.h, area.loc.x, area.loc.y);
    }

    /// Every desktop window placed again (the screens or the dock changed).
    pub fn desktops_replace(&mut self) {
        let ws: Vec<Win> = self.space.elements().filter(|w| self.is_desktop(w)).cloned().collect();
        for w in ws {
            self.desktop_place(&w);
        }
    }

    /// The desktop windows below all others: every other window raised again,
    /// in its order.
    pub fn desktop_to_bottom(&mut self) {
        let others: Vec<Win> = self.space.elements().filter(|w| !self.is_desktop(w)).cloned().collect();
        for w in others {
            self.space.raise_element(&w, false);
        }
    }

    /// The desktop's icons, at the session's start, if wanted.
    pub fn desktop_start(&mut self) {
        if crate::greeter() {
            return;
        }
        if !icons_wanted() {
            return tracing::info!("desktop: icons switched off ([desktop] icons = false)");
        }
        // Kept running on a thread of its own: started, waited for, started again.
        std::thread::spawn(|| {
            use std::sync::atomic::Ordering;
            use std::time::{Duration, Instant};
            let mut starts: Vec<Instant> = Vec::new();
            loop {
                if STOPPING.load(Ordering::Relaxed) || !icons_wanted() {
                    break;
                }
                starts.retain(|t| t.elapsed() < Duration::from_secs(60));
                if starts.len() >= 5 {
                    tracing::warn!("desktop: exited five times in a minute; not started again");
                    break;
                }
                starts.push(Instant::now());
                match crate::spawn_child(std::process::Command::new("vergence-files").arg("--desktop")) {
                    Ok(mut child) => {
                        // tools/install.sh looks for these words before it stops the desktop.
                        tracing::info!("desktop: started (pid {}); started again if it exits", child.id());
                        let status = child.wait();
                        if STOPPING.load(Ordering::Relaxed) {
                            break;
                        }
                        tracing::info!("desktop: exited ({status:?}); starting it again");
                    }
                    Err(e) => {
                        tracing::warn!("desktop: vergence-files --desktop could not be started: {e}");
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    /// A window someone works in, for Super+D and the desktop moving aside:
    /// not the desktop, a splash, a palette, a dialog of another window or
    /// an X11 popup, nor one full screen; whoever draws its title bar
    /// (Firefox and GTK 4 apps draw their own: they were left out while this
    /// asked for the server's bar).
    pub fn ordinary_window(&self, w: &Win) -> bool {
        use crate::style::Kind;
        if self.is_desktop(w) || self.belongs_to_parent(w) || self.fullscreen.contains_key(w) {
            return false;
        }
        if w.x11_surface().is_some_and(|x| x.is_override_redirect()) {
            return false;
        }
        !matches!(self.window_style(w).kind, Kind::Splash | Kind::Desktop | Kind::Palette)
    }

    /// Super+D: the windows out of the way (minimized), or the same back.
    pub fn show_desktop(&mut self) {
        let hidden: Vec<Win> = std::mem::take(&mut self.desktop_hidden)
            .into_iter()
            .filter(|w| smithay::utils::IsAlive::alive(w))
            .collect();
        if !hidden.is_empty() {
            for w in hidden.iter().rev() {
                self.unminimize(w);
            }
            return;
        }
        let shown: Vec<Win> = self.space.elements().filter(|w| self.ordinary_window(w)).cloned().collect();
        for w in &shown {
            self.minimize(w);
        }
        self.desktop_hidden = shown;
        // The desktop has the keyboard (its icons can be typed at).
        let desk = self.space.elements().find(|w| self.is_desktop(w)).cloned();
        if let Some(d) = desk {
            self.focus_window(&d, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    /// The dock's "Add to desktop": the entry copied into ~/Desktop, executable.
    pub fn add_to_desktop(&mut self, id: &str) {
        let Some(dir) = desktop_dir() else { return };
        let file = format!("{id}.desktop");
        let source = vergence_shell::apps::data_dirs()
            .into_iter()
            .map(|d| d.join("applications").join(&file))
            .find(|p| p.is_file());
        let Some(source) = source else { return tracing::info!("desktop: no entry file for {id}") };
        let _ = std::fs::create_dir_all(&dir);
        let mut to = dir.join(&file);
        let mut n = 2;
        while to.exists() {
            to = dir.join(format!("{id} {n}.desktop"));
            n += 1;
        }
        let copied = std::fs::copy(&source, &to).and_then(|_| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755))
        });
        match copied {
            Ok(()) => tracing::info!("desktop: {} added", to.display()),
            Err(e) => tracing::warn!("desktop: {id} could not be added: {e}"),
        }
    }
}
