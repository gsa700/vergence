//! Named desktop actions (dock milestone 1, step 8; whitepaper §6.9): one
//! list of things the desktop can be asked to do by name, which the
//! launcher's search runs today and voice and agents will use later, with
//! their permission questions (§6.5, §7). The desktop's own are here; the
//! server adds one for each control on a live tile's card.

/// One action: its id (what `vctl action` and the server know it by), the
/// name people see, and more words it is found by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub name: String,
    pub words: Vec<String>,
}

impl Action {
    pub fn new(id: &str, name: &str, words: &str) -> Action {
        Action { id: id.into(), name: name.into(), words: words.split_whitespace().map(str::to_string).collect() }
    }
}

/// The settings app's pages an action opens: (page as `--page` takes it, name, more words).
pub const PAGES: [(&str, &str, &str); 14] = [
    ("sound", "Sound", "volume audio output speaker"),
    ("network", "Network", "ethernet cable internet address"),
    ("wi-fi", "Wi-Fi", "wifi wireless"),
    ("appearance", "Appearance", "dark light style accent frost"),
    ("wallpaper", "Wallpaper", "background picture"),
    ("windows", "Windows", "snap title bar buttons"),
    ("dock", "Dock", "shelf pinned hide applets"),
    ("notifications", "Notifications", "cards do not disturb"),
    ("keyboard", "Keyboard", "keys repeat layout spelling"),
    ("mouse", "Mouse", "pointer speed scroll"),
    ("screens", "Screens", "display monitor resolution scale"),
    ("energy", "Energy", "power sleep screens off idle"),
    ("date & time", "Date & Time", "clock time zone 24 hour"),
    ("about", "About", "version license"),
];

/// The desktop's own actions.
pub fn builtin() -> Vec<Action> {
    let mut out = vec![
        Action::new("sound.mute", "Mute sound", "silence quiet volume off"),
        Action::new("sound.unmute", "Unmute sound", "sound on volume"),
        Action::new("sound.up", "Volume up", "louder sound raise"),
        Action::new("sound.down", "Volume down", "quieter sound lower"),
        Action::new("screens.off", "Turn the screens off", "monitors displays blank dark sleep"),
        Action::new("desktop.show", "Show the desktop", "minimize all windows hide"),
        Action::new("dock.drawer", "Open the drawer", "applets tiles status dock"),
        Action::new("session.exit", "Exit Vergence…", "log out logout quit session end"),
    ];
    for (page, name, words) in PAGES {
        out.push(Action::new(&format!("settings.{page}"), &format!("{name} settings"), &format!("open preferences {words}")));
    }
    out
}

/// Whether an action is found by what was typed: every word typed begins
/// a word of its name or of its other words (case not looked at).
pub fn matches(a: &Action, typed: &str) -> bool {
    let words: Vec<String> = a.name.split(|c: char| !c.is_alphanumeric()).chain(a.words.iter().map(String::as_str)).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    let typed: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    !typed.is_empty() && typed.iter().all(|t| words.iter().any(|w| w.starts_with(t.as_str())))
}

/// The actions found by what was typed, as places in `actions`: those whose
/// name begins with it first, then the rest in their order.
pub fn search(actions: &[Action], typed: &str) -> Vec<usize> {
    let low = typed.trim().to_lowercase();
    let mut found: Vec<usize> = (0..actions.len()).filter(|i| matches(&actions[*i], typed)).collect();
    found.sort_by_key(|i| !actions[*i].name.to_lowercase().starts_with(&low));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(typed: &str) -> Vec<String> {
        let all = builtin();
        search(&all, typed).into_iter().map(|i| all[i].name.clone()).collect()
    }

    #[test]
    fn actions_are_found_by_their_words() {
        assert_eq!(names("mute"), ["Mute sound"], "\"unmute\" does not begin with \"mute\"");
        assert_eq!(names("unm"), ["Unmute sound"]);
        assert!(names("screens off").contains(&"Turn the screens off".to_string()));
        assert!(names("log out").contains(&"Exit Vergence…".to_string()));
        assert_eq!(names("sound set"), ["Sound settings"], "every word must be found");
        assert!(names("").is_empty(), "nothing typed, nothing found");
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn every_action_has_a_distinct_id() {
        let all = builtin();
        let mut ids: Vec<&str> = all.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
    }
}
