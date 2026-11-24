//! The sidebar: the pages in their groups, the search, and finding a page
//! by its name.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

/// Whether a page answers a search (`q` lower case): Some(true) by its name,
/// Some(false) by the words of its settings (each word's start), None not.
fn search_match(q: &str, name: &str, words: &str) -> Option<bool> {
    if q.is_empty() {
        None
    } else if name.to_lowercase().contains(q) {
        Some(true)
    } else if words.split(' ').any(|w| w.starts_with(q)) {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod search_tests {
    use super::search_match;

    #[test]
    fn pages_are_found_by_name_first_then_by_their_settings() {
        assert_eq!(search_match("mouse", "Mouse", "pointer speed"), Some(true));
        assert_eq!(search_match("pad", "Touchpad", "pointer speed"), Some(true), "anywhere in the name");
        assert_eq!(
            search_match("fro", "Appearance", "style light dark frost"),
            Some(false),
            "a setting's word, by its start"
        );
        assert_eq!(
            search_match("ost", "Appearance", "style light dark frost"),
            None,
            "not the middle of a setting's word"
        );
        assert_eq!(search_match("", "Appearance", "frost"), None);
    }
}

impl App {
    /// The pages in the sidebar's groups, by type, as GNOME and macOS do
    /// (see docs/settings-milestone-1.md), each with its group's color for its icon and
    /// the words its settings go by, for the search.
    /// A sidebar page by its name, case and punctuation aside ("wifi" is Wi-Fi).
    pub(crate) fn page_named(&self, name: &str) -> Option<Page> {
        let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let want = key(name);
        self.page_groups().into_iter().flat_map(|(_, rows)| rows).find(|(n, ..)| key(n) == want).map(|(_, p, ..)| p)
    }

    pub(crate) fn page_groups(&self) -> Vec<(&'static str, Vec<(String, Page, Icon, &'static str)>)> {
        let multi = self.outputs.len() > 1;
        let screens_page = if multi { Page::Overview } else { Page::Screen(0) };
        let mut screens = vec![(
            "Screens".to_string(),
            screens_page,
            Icon::Screen,
            "screen display monitor scale resolution arrangement",
        )];
        if multi {
            for (i, o) in self.outputs.iter().enumerate() {
                screens.push((screen_label(o), Page::Screen(i), Icon::Screen, "screen display monitor scale"));
            }
        }
        let mut groups = vec![
            ("Connections", self.has_wifi()
                .then(|| ("Wi-Fi".into(), Page::WiFi, Icon::WiFi, "wifi wi-fi wireless network wlan password join hotspot internet"))
                .into_iter()
                .chain([("Network".into(), Page::Network, Icon::Network, "network ethernet cable wired iphone phone usb tethering hotspot address ip")])
                .collect()),
            ("Look", vec![
                ("Appearance".into(), Page::Appearance, Icon::Appearance, "style light dark accent color frost tab tint controls size title bar shade darker lighter"),
                ("Theme and fonts".into(), Page::Theme, Icon::Fonts, "theme font fonts monospace interface icons cursor"),
                ("Wallpaper".into(), Page::Wallpaper(self.primary_index()), Icon::Wallpaper, "wallpaper picture pictures background image photo fit color primary main screen"),
            ]),
            ("Desktop", vec![
                ("Windows".into(), Page::Windows, Icon::Windows, "window title bar buttons controls minimize maximize close snap snapping alt tab keys x11"),
                ("Dock".into(), Page::Dock, Icon::Dock, "dock shelf pin hide opacity gap size bigger smaller edge desktop icons super d"),
                ("Notifications".into(), Page::Notifications, Icon::Notifications, "notifications cards alerts do not disturb quiet corner banner"),
                ("Screenshots".into(), Page::Screenshots, Icon::Screenshot, "screenshot screenshots screen shot capture picture print screen folder thumbnail clipboard super shift 3 4 5 portal programs allowed"),
                ("Apps".into(), Page::Apps, Icon::Apps, "apps applications title bar light dark firefox per app own settings"),
                ("Default apps".into(), Page::DefaultApps, Icon::Apps, "default apps applications open with browser mail text pictures pdf music video folders file types"),
            ]),
            ("Screens", screens),
            ("Input", vec![
                ("Keyboard".into(), Page::Keyboard, Icon::Keyboard, "keyboard repeat delay layout keys spelling spellcheck dictionary"),
                ("Mouse".into(), Page::Mouse, Icon::Pointer, "mouse pointer speed acceleration scroll wheel natural button left handed middle click double"),
            ]
            .into_iter()
            .chain(self.has_had(InputKind::Touchpad).then(|| ("Touchpad".into(), Page::Touchpad, Icon::Touchpad, "touchpad trackpad pointer speed scroll natural tap click drag edge fingers typing palm")))
            .chain(self.has_had(InputKind::Touchscreen).then(|| ("Touchscreen".into(), Page::Touchscreen, Icon::Touchscreen, "touchscreen touch screen panel finger tap press")))
            .collect()),
            ("Sound", vec![("Sound".into(), Page::Sound, Icon::Sound, "sound volume output speaker headphones mute audio")]),
            ("Accessibility", vec![("Accessibility".into(), Page::Accessibility, Icon::Accessibility, "accessibility contrast motion transparency size zoom magnifier lens focus ring")]),
            ("Privacy", vec![
                ("Agents".into(), Page::Agents, Icon::Lock, "agents agent assistant ai claude automation permission allow take back grants privacy mcp"),
                ("Remote".into(), Page::Remote, Icon::Network, "remote vergence remote ssh viewer kvm desktop sharing access another computer"),
            ]),
            ("System", vec![
                ("Energy".into(), Page::Energy, Icon::Energy, "energy power sleep screens off idle"),
                ("Date & Time".into(), Page::DateTime, Icon::Clock, "date time clock zone timezone 24 hour am pm calendar"),
                ("Lock Screen".into(), Page::LockScreen, Icon::Lock, "lock screen password idle sleep super l security"),
                ("Login".into(), Page::Login, Icon::Lock, "login log in automatic autologin login screen boot password vdm"),
                ("Session".into(), Page::Session, Icon::Apps, "session restore reopen restart start apps windows documents place page remember forget log out"),
            ]
            .into_iter()
            .chain(self.has_battery().then(|| ("Battery".into(), Page::Battery, Icon::Battery, "battery charge power watts time left health gpu graphics nvidia")))
            .chain([
                ("About".into(), Page::About, Icon::About, "about version license notices"),
            ])
            .collect()),
        ];
        // No screen (none reported yet): nothing for the Screens pages to show.
        if self.outputs.is_empty() {
            groups.retain(|(h, _)| *h != "Screens");
        }
        groups
    }

    /// The pages the search finds: by name, then by the words of their settings.
    pub(crate) fn found_pages(&self) -> Vec<Page> {
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let all: Vec<(String, Page, &str)> = self
            .page_groups()
            .into_iter()
            .flat_map(|(_, pages)| pages.into_iter().map(|(n, p, _, words)| (n, p, words)))
            .collect();
        let mut found: Vec<Page> =
            all.iter().filter(|(n, _, w)| search_match(&q, n, w) == Some(true)).map(|(_, p, _)| *p).collect();
        for (n, p, words) in &all {
            if !found.contains(p) && search_match(&q, n, words) == Some(false) {
                found.push(*p);
            }
        }
        found
    }

    pub(crate) fn sidebar(&self) -> Element<Message> {
        let selected_page = |p: Page| match p {
            Page::About => matches!(self.page, Page::About | Page::LicenseText | Page::Notices),
            Page::Wallpaper(_) => matches!(self.page, Page::Wallpaper(_)),
            p => self.page == p,
        };
        let item = |text: String, page: Page, icon: Icon, child: bool| -> Element<Message> {
            let selected = selected_page(page);
            let l = label(text).size(if child { 12.0 } else { 13.0 }).weight(if selected { 600 } else { 400 });
            button(
                row(vec![icon_mark(icon), l, space().width(Length::FILL)])
                    .spacing(8.0)
                    .align(Align::Center)
                    .width(Length::FILL),
            )
            .look(if selected { ButtonLook::Chosen } else { ButtonLook::Plain })
            .padding(Padding { top: 4.0, bottom: 4.0, left: if child { 22.0 } else { 8.0 }, right: 10.0 })
            .on_press(Message::Go(page))
            .width(Length::FILL)
        };
        let mut items: Vec<Element<Message>> = vec![
            field(&self.search, Message::Search)
                .placeholder("Search")
                .on_submit(Message::SearchGo)
                .width(Length::FILL)
                .name("Search settings"),
            space().height(Length::Fixed(8.0)),
        ];
        let found = self.found_pages();
        let searching = !self.search.trim().is_empty();
        for (heading, pages) in self.page_groups() {
            let pages: Vec<_> = pages.into_iter().filter(|(_, p, ..)| !searching || found.contains(p)).collect();
            if pages.is_empty() {
                continue;
            }
            items.push(label(heading).size(11.0).dim().weight(600));
            for (i, (name, page, icon, _)) in pages.into_iter().enumerate() {
                let child = heading == "Screens" && i > 0;
                items.push(item(name, page, icon, child));
            }
            items.push(space().height(Length::Fixed(6.0)));
        }
        if searching && found.is_empty() {
            items.push(label("Nothing by that name.").size(12.0).dim());
        }
        // Its color runs up under the frosted bar; its items start below it
        // and scroll under it when the window is shorter than the list.
        let list = panel(column(items).spacing(2.0).width(Length::FILL))
            .padding(Padding { top: 12.0 + self.top.get(), bottom: 12.0, left: 10.0, right: 10.0 })
            .radius(0.0)
            .width(Length::FILL);
        panel(scroll(list)).radius(0.0).width(Length::Fixed(SIDEBAR_W)).height(Length::FILL).name("Pages")
    }
}
