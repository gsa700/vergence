//! What is running, as the shelf shows it: one item for each application
//! that has a window open, and where the items stand on the strip.

use crate::{Rect, STRIP_W, desktop::Entry};

/// The height of one place on the strip (an icon with its margins), and
/// the icon's side. The first place, at the top, is the apps button's.
pub const SLOT: i32 = 48;
pub const ICON: i32 = vergence_look::SHELF_ICON as i32;

/// The application a window belongs to, by the class the window carries
/// (a Wayland app_id or an X11 WM_CLASS): the entry whose id is that class,
/// else whose StartupWMClass is, else whose program is named so; case is
/// not looked at. None if nothing matches: the window still gets an item,
/// under its class.
pub fn owner<'a>(entries: &'a [Entry], class: &str) -> Option<&'a Entry> {
    if class.is_empty() {
        return None;
    }
    const COMM: usize = 15;
    let same = |a: &str| a.eq_ignore_ascii_case(class);
    let cut = |a: &str| a.len() > COMM && a.is_char_boundary(COMM) && a[..COMM].eq_ignore_ascii_case(class);
    let program = |e: &Entry| e.exec.first().map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).unwrap_or_default();
    entries
        .iter()
        .find(|e| same(&e.id))
        // "org.mozilla.firefox" for a window of class "firefox".
        .or_else(|| entries.iter().find(|e| e.id.rsplit('.').next().is_some_and(same)))
        .or_else(|| entries.iter().find(|e| !e.startup_wm_class.is_empty() && same(&e.startup_wm_class)))
        .or_else(|| entries.iter().find(|e| same(&program(e))))
        // A window that names no application is known by its process's name, which
        // the system keeps only fifteen characters of ("vergence-settin").
        .or_else(|| entries.iter().find(|e| class.len() == COMM && (cut(&e.id) || cut(&program(e)))))
}

/// One application on the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The entry's id, or the windows' class where no entry owns them.
    pub key: String,
    /// The entry's name, or the class.
    pub name: String,
    /// The entry's icon name ("" where there is no entry: a letter is drawn).
    pub icon: String,
    /// Its windows, as places in the list the caller gave.
    pub windows: Vec<usize>,
    /// One of its windows has the keyboard.
    pub focused: bool,
}

/// The items for these windows (each its class and whether it has the
/// keyboard), given in the order they were opened: one item for each
/// application, in the order of each one's first window.
pub fn items(entries: &[Entry], windows: &[(String, bool)]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for (index, (class, focused)) in windows.iter().enumerate() {
        let (key, name, icon) = match owner(entries, class) {
            Some(e) => (e.id.clone(), e.name.clone(), e.icon.clone()),
            None => (class.to_lowercase(), class.clone(), String::new()),
        };
        match out.iter_mut().find(|i| i.key == key) {
            Some(item) => {
                item.windows.push(index);
                item.focused |= focused;
            }
            None => out.push(Item { key, name, icon, windows: vec![index], focused: *focused }),
        }
    }
    out
}

/// The place of item `index` on the strip: below the apps button's, one
/// under the other. None for one that would not fit whole (a strip too
/// short for all its items shows the first of them; scrolling it comes
/// with the popup's).
pub fn slot(strip: Rect, index: usize) -> Option<Rect> {
    let y = strip.y + SLOT * (index as i32 + 1);
    (y + SLOT <= strip.bottom()).then(|| Rect::new(strip.x, y, STRIP_W, SLOT))
}

/// The icon's square in a place: in its middle.
pub fn icon_in(slot: Rect) -> Rect {
    Rect::new(slot.x + (slot.w - ICON) / 2, slot.y + (slot.h - ICON) / 2, ICON, ICON)
}

/// Which of `count` items is at a point, if any.
pub fn item_at(strip: Rect, count: usize, x: i32, y: i32) -> Option<usize> {
    (0..count).find(|i| slot(strip, *i).is_some_and(|s| x >= s.x && x < s.right() && y >= s.y && y < s.bottom()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, exec: &str, class: &str) -> Entry {
        Entry { id: id.into(), name: name.into(), exec: vec![exec.into()], icon: id.into(), startup_wm_class: class.into(), ..Entry::default() }
    }

    fn entries() -> Vec<Entry> {
        vec![
            entry("firefox", "Firefox", "/usr/lib/firefox/firefox", "firefox"),
            entry("vergence-term", "Terminal", "vergence-term", "vergence-term"),
            entry("org.gnome.Nautilus", "Files", "nautilus", ""),
            entry("albumwall", "AlbumWall", "/home/d/.local/share/albumwall/AlbumWall", ""),
            entry("gimp", "GNU Image Manipulation Program", "gimp-3.0", "Gimp-3.0"),
        ]
    }

    #[test]
    fn a_window_finds_its_application() {
        let e = entries();
        let id = |class: &str| owner(&e, class).map(|e| e.id.as_str());
        assert_eq!(id("firefox"), Some("firefox"));
        assert_eq!(id("Firefox"), Some("firefox"), "case is not looked at");
        assert_eq!(id("vergence-term"), Some("vergence-term"));
        assert_eq!(id("org.gnome.Nautilus"), Some("org.gnome.Nautilus"));
        assert_eq!(id("nautilus"), Some("org.gnome.Nautilus"), "the id's last part");
        assert_eq!(id("AlbumWall"), Some("albumwall"));
        assert_eq!(id("gimp-3.0"), Some("gimp"), "by StartupWMClass");
        assert_eq!(id("xterm"), None);
        assert_eq!(id(""), None);
        // A process name cut to fifteen characters still finds its application.
        let mut e = entries();
        e.push(entry("vergence-settings", "Settings", "vergence-settings", ""));
        assert_eq!(owner(&e, "vergence-settin").map(|e| e.id.as_str()), Some("vergence-settings"));
        assert_eq!(owner(&e, "vergence-setti").map(|e| e.id.as_str()), None, "only at exactly fifteen");
    }

    #[test]
    fn one_item_for_each_application_in_the_order_they_started() {
        let e = entries();
        let w = |class: &str, focused: bool| (class.to_string(), focused);
        let got = items(&e, &[w("vergence-term", false), w("firefox", false), w("XTerm", false), w("vergence-term", true), w("xterm", false)]);
        let summary: Vec<(&str, &str, usize, bool)> = got.iter().map(|i| (i.key.as_str(), i.name.as_str(), i.windows.len(), i.focused)).collect();
        assert_eq!(summary, [("vergence-term", "Terminal", 2, true), ("firefox", "Firefox", 1, false), ("xterm", "XTerm", 2, false)]);
        assert_eq!(got[0].windows, [0, 3]);
        assert_eq!((got[0].icon.as_str(), got[2].icon.as_str()), ("vergence-term", ""), "no entry, no icon name: a letter");
        assert!(items(&e, &[]).is_empty());
    }

    #[test]
    fn items_stand_under_the_apps_button() {
        let strip = Rect::new(1232, 0, 48, 720);
        assert_eq!(slot(strip, 0), Some(Rect::new(1232, 48, 48, 48)));
        assert_eq!(slot(strip, 2), Some(Rect::new(1232, 144, 48, 48)));
        assert_eq!(icon_in(slot(strip, 0).unwrap()), Rect::new(1240, 56, 32, 32));
        // 720 points hold the button and fourteen items; a fifteenth does not fit.
        assert!(slot(strip, 13).is_some() && slot(strip, 14).is_none());
        assert_eq!(item_at(strip, 3, 1250, 60), Some(0));
        assert_eq!(item_at(strip, 3, 1250, 150), Some(2));
        assert_eq!(item_at(strip, 3, 1250, 200), None, "below the last item");
        assert_eq!(item_at(strip, 3, 1250, 20), None, "the apps button's place");
        assert_eq!(item_at(strip, 3, 1200, 60), None, "left of the strip");
    }
}
