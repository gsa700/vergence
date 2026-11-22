//! The Dock page (Page::Dock): its edge, screen, size, gap, opacity, hiding,
//! the desktop's icons and the applets (desktop.toml [shelf]).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

/// The dock's edges: as shown, and as desktop.toml writes them.
/// The dock's edges, the default (bottom, where the launcher opens as a start menu) first
/// (see docs/menus-milestone-1.md).
const DOCK_EDGES: [(&str, &str); 4] =
    [("Bottom edge", "bottom"), ("Left edge", "left"), ("Top edge", "top"), ("Right edge", "right")];
/// When the dock hides: as shown, and as desktop.toml writes it.
const DOCK_HIDE: [(&str, &str); 3] = [("Never", "never"), ("When a window covers it", "windows"), ("Always", "always")];

impl App {
    fn shelf(&self) -> Option<&toml::Table> {
        self.desktop.get("shelf").and_then(|v| v.as_table())
    }

    /// The dock's sliders follow the file when it is read.
    pub(crate) fn dock_from_file(&mut self) {
        let shelf = self.shelf();
        let gap = shelf
            .and_then(|t| t.get("gap"))
            .and_then(|v| v.as_integer())
            .unwrap_or(vergence_shell::GAP as i64)
            .clamp(0, vergence_shell::MAX_GAP as i64) as f32;
        let opacity = shelf
            .and_then(|t| t.get("opacity"))
            .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
            .unwrap_or(1.0)
            .clamp(0.3, 1.0) as f32;
        let size = shelf
            .and_then(|t| t.get("size"))
            .and_then(|v| v.as_integer())
            .unwrap_or(vergence_shell::STRIP_W as i64)
            .clamp(vergence_shell::MIN_SIZE as i64, vergence_shell::MAX_SIZE as i64) as f32;
        (self.dock_gap, self.dock_opacity, self.dock_size) = (gap, opacity, size);
    }

    /// When the dock hides, as a place in DOCK_HIDE: `hide` is a word, or
    /// true ("windows") and false ("never").
    fn dock_hide(&self) -> usize {
        let word = match self.shelf().and_then(|t| t.get("hide")) {
            Some(toml::Value::Boolean(true)) => "windows".to_string(),
            Some(toml::Value::String(w)) => w.trim().to_ascii_lowercase(),
            _ => "never".to_string(),
        };
        DOCK_HIDE.iter().position(|h| h.1 == word).unwrap_or(0)
    }

    pub(crate) fn dock(&self) -> Element<Message> {
        let shelf = self.shelf();
        let text = |k: &str| shelf.and_then(|t| t.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let flag = |k: &str, default: bool| shelf.and_then(|t| t.get(k)).and_then(|v| v.as_bool()).unwrap_or(default);
        let free = text("place") == "free";
        let mut edges: Vec<String> = DOCK_EDGES.iter().map(|e| e.0.to_string()).collect();
        let at = if free {
            edges.push("Where you put it".into());
            Some(edges.len() - 1)
        } else {
            let e = text("edge");
            Some(DOCK_EDGES.iter().position(|d| d.1 == e).unwrap_or(0))
        };
        let mut place = vec![setting(
            "Where",
            if free {
                "Floating where it was dragged. Choose an edge, or drag it to one, to dock it there."
            } else {
                "Drag the dock away from its edge to float it anywhere."
            },
            choice(edges, at, Message::DockEdge).width(Length::Fixed(180.0)).name("Where"),
        )];
        if self.outputs.len() > 1 {
            let mut screens = vec!["The main screen".to_string()];
            screens.extend(self.outputs.iter().map(screen_label));
            let id = text("screen");
            let at = self.outputs.iter().position(|o| o.id == id).map_or(0, |i| i + 1);
            place.push(setting(
                "Screen",
                "",
                choice(screens, Some(at), Message::DockScreen).width(Length::Fixed(180.0)).name("Screen"),
            ));
        }
        place.push(setting(
            "Gap",
            "The room between the dock, its edge and the windows.",
            row(vec![
                slider(0.0, vergence_shell::MAX_GAP as f32, self.dock_gap, Message::DockGap)
                    .step(1.0)
                    .marks(vec![vergence_shell::GAP as f32])
                    .on_release(Message::DockSave)
                    .width(Length::Fixed(200.0))
                    .name("Gap"),
                label(format!("{:.0} pt", self.dock_gap)).width(Length::Fixed(48.0)).align(Align::End),
            ])
            .spacing(8.0)
            .align(Align::Center),
        ));
        place.push(setting(
            "Keep windows clear",
            "At an edge, maximized and new windows stop short of the dock.",
            switch(flag("keep-clear", true), Message::DockKeepClear).name("Keep windows clear"),
        ));
        let look = vec![
            setting(
                "Size",
                "How big the dock and its icons are. The usual size is 48.",
                row(vec![
                    slider(
                        vergence_shell::MIN_SIZE as f32,
                        vergence_shell::MAX_SIZE as f32,
                        self.dock_size,
                        Message::DockSize,
                    )
                    .step(1.0)
                    .marks(vec![vergence_shell::STRIP_W as f32])
                    .on_release(Message::DockSave)
                    .width(Length::Fixed(200.0))
                    .name("Size"),
                    label(format!("{:.0} pt", self.dock_size)).width(Length::Fixed(48.0)).align(Align::End),
                ])
                .spacing(8.0)
                .align(Align::Center),
            ),
            setting(
                "Hide",
                "Out of the way, a sliver left at its edge; the pointer there brings it back. Windows then have the whole screen.",
                choice(DOCK_HIDE.iter().map(|h| h.0.to_string()).collect(), Some(self.dock_hide()), Message::DockHide)
                    .width(Length::Fixed(220.0))
                    .name("Hide"),
            ),
            setting(
                "Opacity",
                "Less, and the desktop shows through the dock. Its icons stay solid.",
                row(vec![
                    slider(0.3, 1.0, self.dock_opacity, Message::DockOpacity)
                        .step(0.05)
                        .on_release(Message::DockSave)
                        .width(Length::Fixed(200.0))
                        .name("Opacity"),
                    percent(self.dock_opacity),
                ])
                .spacing(8.0)
                .align(Align::Center),
            ),
        ];
        let off: Vec<String> = shelf
            .and_then(|t| t.get("applets-off"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut applets: Vec<Element<Message>> = vec![setting(
            "Out of the drawer when in trouble",
            "An applet with a problem or a warning stands on the dock until it clears.",
            switch(flag("trouble-out", true), Message::TroubleOut).name("Out of the drawer when in trouble"),
        )];
        if self.applets.is_empty() {
            applets.push(setting("No applets yet", "The clock, sound, network and power, and those programs put up, are listed here once the dock has shown them.", space()));
        }
        for (i, (id, name, from)) in self.applets.iter().enumerate() {
            let on = !off.contains(id);
            let what = if from == "the desktop" { "From the desktop.".to_string() } else { format!("From {from}.") };
            let id = id.clone();
            let shown = if name.is_empty() { id.clone() } else { name.clone() };
            applets.push(
                setting(
                    &shown,
                    &what,
                    switch(on, move |on| Message::Applet(id.clone(), on)).name(&format!("Show {shown}")),
                )
                .key(i as u64 + 1),
            );
        }
        self.page_layout(
            "Dock",
            "Where the dock sits and how it looks, and its applets. Drag an icon to move it, onto the dock to pin it, off the dock to remove it. It follows at once.",
            vec![self.group(Some("Place"), place), self.group(Some("Look"), look), self.group(Some("Applets"), applets), self.desktop_group(), self.status_line()],
        )
    }

    /// The desktop's icons: on or off (from the next login), and what they are.
    fn desktop_group(&self) -> Element<Message> {
        let on = self.desktop.get("desktop").and_then(|d| d.get("icons")).and_then(|v| v.as_bool()).unwrap_or(true);
        self.group(
            Some("Desktop"),
            vec![setting(
                "Icons on the desktop",
                "The files in your Desktop folder, over the wallpaper. Super+D shows the desktop. From the next login.",
                switch(on, Message::DesktopIcons).name("Icons on the desktop"),
            )],
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn dock_message(&mut self, m: Message) {
        match m {
            Message::DesktopIcons(on) => self.save(|d| {
                let t = d.entry("desktop").or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if let Some(t) = t.as_table_mut() {
                    t.insert("icons".into(), on.into());
                }
            }),
            Message::DockEdge(i) => {
                // Choosing an edge docks it there; "where it was put" is only shown, not chosen.
                if let Some(edge) = DOCK_EDGES.get(i) {
                    let edge = edge.1;
                    self.save_table("shelf", |t| {
                        for k in ["x", "y", "upright"] {
                            t.remove(k);
                        }
                        t.insert("place".into(), "edge".into());
                        t.insert("edge".into(), edge.into());
                    });
                }
            }
            Message::DockScreen(i) => {
                let id = if i == 0 { None } else { self.outputs.get(i - 1).map(|o| o.id.clone()) };
                self.save_table("shelf", |t| match id {
                    Some(id) => {
                        t.insert("screen".into(), id.into());
                    }
                    None => {
                        t.remove("screen");
                    }
                });
            }
            Message::DockGap(v) => self.dock_gap = v.round(),
            Message::DockOpacity(v) => self.dock_opacity = (v * 20.0).round() / 20.0,
            // In steps of 4 points, sticking to the usual size near it.
            Message::DockSize(v) => {
                let usual = vergence_shell::STRIP_W as f32;
                self.dock_size = if (v - usual).abs() <= 3.0 { usual } else { (v / 4.0).round() * 4.0 };
            }
            Message::DockSave => {
                let (gap, opacity) = (self.dock_gap as i64, (self.dock_opacity as f64 * 100.0).round() / 100.0);
                let size = self.dock_size.round() as i64;
                self.save_table("shelf", |t| {
                    t.insert("gap".into(), gap.into());
                    t.insert("opacity".into(), opacity.into());
                    // The usual size is no setting: the key goes.
                    if size == vergence_shell::STRIP_W as i64 {
                        t.remove("size");
                    } else {
                        t.insert("size".into(), size.into());
                    }
                });
            }
            Message::DockKeepClear(on) => self.save_table("shelf", |t| {
                t.insert("keep-clear".into(), on.into());
            }),
            Message::DockHide(i) => {
                let word = DOCK_HIDE.get(i).map_or("never", |h| h.1);
                self.save_table("shelf", |t| {
                    t.insert("hide".into(), word.into());
                });
            }
            Message::Applet(id, on) => {
                let mut off: Vec<String> = self
                    .shelf()
                    .and_then(|t| t.get("applets-off"))
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                off.retain(|x| *x != id);
                if !on {
                    off.push(id);
                }
                off.sort();
                self.save_table("shelf", |t| {
                    if off.is_empty() {
                        t.remove("applets-off");
                    } else {
                        t.insert(
                            "applets-off".into(),
                            toml::Value::Array(off.into_iter().map(toml::Value::String).collect()),
                        );
                    }
                });
            }
            Message::TroubleOut(on) => self.save_table("shelf", |t| {
                t.insert("trouble-out".into(), on.into());
            }),
            _ => {}
        }
    }
}

/// The applets the desktop has seen (the server's
/// ~/.local/state/vergence/applets.toml): tile id, name, who supplies it;
/// the desktop's first, then by name.
pub(crate) fn read_applets() -> Vec<(String, String, String)> {
    let Some(state) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
    else {
        return Vec::new();
    };
    let text = std::fs::read_to_string(state.join("vergence/applets.toml")).unwrap_or_default();
    parse_applets(&text)
}

fn parse_applets(text: &str) -> Vec<(String, String, String)> {
    let Some(a) = text.parse::<toml::Table>().ok().and_then(|t| t.get("applets").and_then(|a| a.as_table()).cloned())
    else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, String)> = a
        .iter()
        .filter_map(|(id, v)| {
            let v = v.as_table()?;
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            Some((id.clone(), s("name"), s("from")))
        })
        .collect();
    out.sort_by(|a, b| {
        (a.2 != "the desktop").cmp(&(b.2 != "the desktop")).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    out
}

#[cfg(test)]
mod applets_tests {
    #[test]
    fn the_desktops_applets_come_first_then_by_name() {
        let text = "[applets.ups]\nname = \"UPS (rack)\"\nfrom = \"vtile\"\n[applets.clock]\nname = \"Monday\"\nfrom = \"the desktop\"\n[applets.nas]\nname = \"NAS\"\nfrom = \"vtile\"\n";
        let ids: Vec<String> = super::parse_applets(text).into_iter().map(|(id, ..)| id).collect();
        assert_eq!(ids, ["clock", "nas", "ups"]);
        assert!(super::parse_applets("not toml [").is_empty());
    }
}
