//! Named desktop actions (dock milestone 1, step 8): the desktop's own
//! (vergence-shell actions.rs) and one for each switch and button on a live
//! tile's card, run by name from the launcher's search or `vctl action`.
//! Voice and agents will reach the same list later, with their permission
//! questions (whitepaper §6.5, §7); nothing else runs them yet.

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use vergence_protocol::tile::ControlKind;
use vergence_shell::actions::{self, Action};

use crate::Server;

impl Server {
    /// Every action there is now: the desktop's, then the tiles' controls
    /// ("UPS (rack): Self-test", "Sound: Mute").
    pub fn actions_all(&self) -> Vec<Action> {
        let mut all = actions::builtin();
        // Programs' tiles only: the desktop's own controls are actions above already.
        for p in self.tiles.list.iter().filter(|p| p.owner != crate::tiles::Owner::Desktop) {
            let title = if p.tile.card.title.is_empty() { p.tile.label.clone() } else { p.tile.card.title.clone() };
            for k in &p.tile.card.controls {
                if matches!(k.kind, ControlKind::Button | ControlKind::Switch(_)) {
                    let id = format!("tile.{}.{}", p.key(), k.id);
                    let words = format!("{} {} {}", p.tile.label, p.tile.group, p.tile.id);
                    all.push(Action::new(&id, &format!("{title}: {}", k.label), &words));
                }
            }
        }
        all
    }

    /// Run an action by its id, or by its name (case not looked at).
    pub fn action_run(&mut self, which: &str) -> Result<String, String> {
        let all = self.actions_all();
        let a = all
            .iter()
            .find(|a| a.id == which)
            .or_else(|| all.iter().find(|a| a.name.eq_ignore_ascii_case(which)))
            .cloned()
            .ok_or_else(|| format!("no action {which:?}"))?;
        tracing::info!("actions: {} ({})", a.name, a.id);
        let id = a.id.as_str();
        match id {
            "sound.mute" | "sound.unmute" => {
                self.desktop_tile_action("sound", "mute", if id == "sound.mute" { 1.0 } else { 0.0 })
            }
            "sound.up" => self.volume_key(5),
            "sound.down" => self.volume_key(-5),
            // After a moment: the key that ran it, let go, is input and would wake them.
            "screens.off" => {
                let _ = self.loop_handle.insert_source(
                    Timer::from_duration(std::time::Duration::from_millis(700)),
                    |_, _, st| {
                        st.energy.screens_off = true;
                        crate::drm::screens_off(st);
                        st.input_screens_off();
                        TimeoutAction::Drop
                    },
                );
            }
            "desktop.show" => self.show_desktop(),
            "lock.screen" => {
                self.lock_request("the named action");
            }
            "dock.drawer" => {
                if self.tiles.drawer.is_none() {
                    self.drawer_toggle();
                }
            }
            "session.exit" => self.session_dialog_open(),
            _ if id.starts_with("settings.") => {
                let page = &id["settings.".len()..];
                if let Err(e) = crate::spawn(std::process::Command::new("vergence-settings").args(["--page", page])) {
                    return Err(format!("the settings app could not be started: {e}"));
                }
            }
            _ if id.starts_with("tile.") => self.tile_control_by_name(&id["tile.".len()..])?,
            _ => return Err(format!("{id} does nothing yet")),
        }
        self.redraw_all();
        Ok(a.name)
    }

    /// "`<key>.<control>`": a tile's button pressed or switch turned over, as
    /// on its card.
    fn tile_control_by_name(&mut self, rest: &str) -> Result<(), String> {
        for (index, p) in self.tiles.list.iter().enumerate() {
            let key = p.key();
            if let Some(control) = rest.strip_prefix(&format!("{key}."))
                && let Some(i) = p.tile.card.controls.iter().position(|k| k.id == control)
            {
                let value = match p.tile.card.controls[i].kind {
                    ControlKind::Switch(true) => 0.0,
                    _ => 1.0,
                };
                self.card_act(index, i, value);
                return Ok(());
            }
        }
        Err(format!("no tile control {rest:?}"))
    }

    /// For `vctl actions`: every action's id and name.
    pub fn actions_report(&self) -> String {
        self.actions_all().iter().map(|a| format!("{}  {}", a.id, a.name)).collect::<Vec<_>>().join("\n")
    }
}
