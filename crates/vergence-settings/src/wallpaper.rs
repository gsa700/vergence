//! The Wallpaper page (Page::Wallpaper) and its picture browser: every
//! screen's picture, fit and color (desktop.toml [wallpaper], as wall.rs
//! edits it).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

const FITS: [&str; 4] = ["Fill", "Fit", "Center", "Stretch"];

/// The picture browser: folder mode (add the folder shown) or pictures mode
/// (pick pictures in any folder, then add them).
pub(crate) struct Browser {
    folders: bool,
    dir: PathBuf,
    entries: Vec<(PathBuf, bool)>,
    picked: Vec<PathBuf>,
}

impl App {
    /// A picture, fit or color chosen: for the open screen (which then has its
    /// own wallpaper) or for the primary (the default).
    fn save_wallpaper(&mut self, image: Option<Option<&Path>>, mode: Option<&str>, color: Option<&str>) {
        let scope = self.scope();
        let shown = wall::shown(&self.desktop, scope.as_ref().map(|(i, _)| i.as_str()));
        let image = image.unwrap_or(shown.image.as_deref()).map(Path::to_path_buf);
        let mode = mode.unwrap_or(&shown.mode).to_string();
        let color = color.unwrap_or(&shown.color).to_string();
        self.save(|d| match &scope {
            Some((id, label)) => wall::set(d, Some(id), label, image.as_deref(), &mode, &color),
            None => wall::set(d, None, "", image.as_deref(), &mode, &color),
        });
    }

    /// Every screen's picture in one place (settings milestone 1): with
    /// several screens, a row of them to choose whose wallpaper is open, then
    /// its picture, fit and color, and the pictures to choose from.
    pub(crate) fn wallpaper(&self) -> Element<Message> {
        let multi = self.outputs.len() > 1;
        let scope = self.scope();
        let shown = wall::shown(&self.desktop, scope.as_ref().map(|(i, _)| i.as_str()));
        let is_primary = scope.is_none();
        let primary_label =
            self.primary().and_then(|p| self.outputs.iter().find(|o| o.id == p)).map(screen_label).unwrap_or_default();
        let description = if !multi {
            "The picture behind your windows, drawn at the screen's own pixels.".to_string()
        } else if is_primary {
            "The main screen's wallpaper is the default, shown on every other screen that has none of its own."
                .to_string()
        } else {
            format!("Shows the main screen's wallpaper ({primary_label}) unless it has its own.")
        };
        let (accent, card) = (self.accent(), self.card_color());
        let mut items: Vec<Element<Message>> = Vec::new();
        if multi {
            // The screens, each with the picture it shows.
            let open = match self.page {
                Page::Wallpaper(i) => i,
                _ => 0,
            };
            let primary = self.primary();
            let mut screens_out = Vec::new();
            for (i, o) in self.outputs.iter().enumerate() {
                let own = (primary.as_deref() != Some(o.id.as_str())).then_some(o.id.as_str());
                let its = wall::shown(&self.desktop, own);
                let pic: Element<Message> = match its.image.as_ref().and_then(|p| self.grid.get(p)) {
                    Some(img) => {
                        picture(img.clone()).width(Length::Fixed(thumbs::GRID_W)).height(Length::Fixed(thumbs::GRID_H))
                    }
                    None => panel(space())
                        .color(parse_hex(&its.color).unwrap_or(card))
                        .radius(0.0)
                        .width(Length::Fixed(thumbs::GRID_W))
                        .height(Length::Fixed(thumbs::GRID_H)),
                };
                let mut name = screen_label(o);
                if primary.as_deref() == Some(o.id.as_str()) {
                    name.push_str(" (main)");
                }
                let caption = label(short_name(&name, 25)).size(11.0).width(Length::Fixed(thumbs::GRID_W));
                screens_out.push(
                    button(column(vec![pic, caption]).spacing(4.0))
                        .padding(3.0)
                        .look(ButtonLook::Custom {
                            fill: None,
                            edge: (i == open).then_some((accent, 3.0)),
                            radius: 6.0,
                        })
                        .on_press(Message::Go(Page::Wallpaper(i)))
                        .name(name)
                        .key(1000 + i as u64),
                );
            }
            items.push(self.group(Some("Screens"), vec![row(screens_out).spacing(8.0).wrap().width(Length::FILL)]));
        }
        let mut wp: Vec<Element<Message>> = Vec::new();
        if multi && !is_primary {
            let detail = if shown.own {
                "Off: this screen has its own.".to_string()
            } else {
                format!("Shows {primary_label}'s wallpaper.")
            };
            wp.push(setting(
                "Same as the main screen",
                &detail,
                switch(!shown.own, Message::SameAsPrimary).name("Same as the main screen"),
            ));
        }
        if multi {
            wp.push(setting(
                "Main screen",
                if is_primary {
                    "The dock stands on it, and its wallpaper is the default for the others."
                } else {
                    "Make it the main screen: the dock moves to it, and its wallpaper becomes the default."
                },
                if is_primary {
                    label("Main screen").dim()
                } else {
                    button(label("Make main screen").size(12.0))
                        .padding(Padding::hv(4.0, 12.0))
                        .on_press(Message::MakePrimary)
                },
            ));
        }
        let fit = wall::MODES.iter().position(|m| *m == shown.mode).unwrap_or(0);
        let fits: Vec<String> = FITS.iter().map(|s| s.to_string()).collect();
        wp.push(setting("Fit", "", choice(fits, Some(fit), Message::Fit).width(Length::Fixed(140.0)).name("Fit")));
        wp.push(setting(
            "Background color",
            "Around a picture, or alone without one. Enter applies it.",
            field(&self.bg_text, Message::Background)
                .placeholder("#202020")
                .on_submit(Message::BackgroundSubmit)
                .width(Length::Fixed(120.0))
                .name("Background color"),
        ));
        items.push(self.group(Some("Wallpaper"), wp));
        // The pictures.
        let mut tiles_out = Vec::new();
        for (i, p) in self.files.iter().enumerate() {
            let selected = shown.image.as_deref() == Some(p.as_path());
            let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let pic: Element<Message> = match self.grid.get(p) {
                Some(img) => {
                    picture(img.clone()).width(Length::Fixed(thumbs::GRID_W)).height(Length::Fixed(thumbs::GRID_H))
                }
                None => panel(space())
                    .color(card)
                    .radius(0.0)
                    .width(Length::Fixed(thumbs::GRID_W))
                    .height(Length::Fixed(thumbs::GRID_H)),
            };
            // Long names are shortened to the tile's width instead of running into the next.
            let caption = label(short_name(&name, 25)).size(11.0).width(Length::Fixed(thumbs::GRID_W));
            tiles_out.push(
                button(column(vec![pic, caption]).spacing(4.0))
                    .padding(3.0)
                    .look(ButtonLook::Custom { fill: None, edge: selected.then_some((accent, 3.0)), radius: 6.0 })
                    .on_press(Message::PickPicture(i))
                    .name(name)
                    .key(i as u64),
            );
        }
        items.push(row(tiles_out).spacing(8.0).wrap().width(Length::FILL));
        // Your pictures.
        let (folders, files) = wall::sources(&self.desktop);
        let mut yours: Vec<Element<Message>> = Vec::new();
        for (i, (p, kind)) in
            folders.iter().map(|f| (f, "Folder")).chain(files.iter().map(|f| (f, "Picture"))).enumerate()
        {
            yours.push(setting(
                &p.display().to_string(),
                kind,
                button(label("Remove").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(Message::RemoveSource(i)),
            ));
        }
        yours.push(setting(
            if folders.is_empty() && files.is_empty() { "Add your own pictures" } else { "Add more" },
            "Pictures added to a folder later appear too.",
            row(vec![
                button(label("Add a folder\u{2026}").size(12.0))
                    .padding(Padding::hv(4.0, 10.0))
                    .on_press(Message::Browse(true)),
                button(label("Add pictures\u{2026}").size(12.0))
                    .padding(Padding::hv(4.0, 10.0))
                    .on_press(Message::Browse(false)),
            ])
            .spacing(6.0),
        ));
        items.push(self.group(Some("Your pictures"), yours));
        items.push(self.status_line());
        self.page_layout("Wallpaper", &description, items)
    }

    pub(crate) fn browser_page(&self) -> Element<Message> {
        let Some(b) = self.browser.as_ref() else { return space() };
        let mut entries = Vec::new();
        for (i, (p, folder)) in b.entries.iter().enumerate() {
            let picked = !b.folders && b.picked.contains(p);
            let mark = if *folder {
                "\u{25B8}"
            } else if picked {
                "\u{2713}"
            } else {
                ""
            };
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            entries.push(
                button(
                    row(vec![label(mark).width(Length::Fixed(18.0)), label(name.clone()), space().width(Length::FILL)])
                        .spacing(6.0)
                        .width(Length::FILL),
                )
                .padding(Padding::hv(5.0, 10.0))
                .look(if picked { ButtonLook::Chosen } else { ButtonLook::Plain })
                .on_press(Message::BrowseOpen(i))
                .width(Length::FILL)
                .name(name)
                .key(i as u64),
            );
        }
        let add_label = if b.folders {
            "Add this folder".to_string()
        } else if b.picked.len() == 1 {
            "Add 1 picture".into()
        } else {
            format!("Add {} pictures", b.picked.len())
        };
        let mut add = button(label(add_label)).look(ButtonLook::Accent).padding(Padding::hv(5.0, 14.0));
        if b.folders || !b.picked.is_empty() {
            add = add.on_press(Message::BrowseAdd);
        }
        panel(
            column(vec![
                self.intro(
                    Some(if b.folders { "Add a folder" } else { "Add pictures" }),
                    if b.folders {
                        "Open the folder you want, then add it. Every picture in it and its folders is offered."
                    } else {
                        "Click pictures to pick them, in as many folders as you like, then add them."
                    },
                ),
                row(vec![
                    button(label("\u{2191} Up").size(12.0)).padding(Padding::hv(4.0, 10.0)).on_press(Message::BrowseUp),
                    label(b.dir.display().to_string()).size(12.0).dim(),
                ])
                .spacing(10.0)
                .align(Align::Center),
                panel(scroll(column(entries).spacing(1.0).width(Length::FILL)).name("Folder"))
                    .color(self.card_color())
                    .radius(10.0)
                    .padding(4.0)
                    .width(Length::FILL)
                    .height(Length::FILL),
                row(vec![
                    space().width(Length::FILL),
                    button(label("Cancel")).padding(Padding::hv(5.0, 14.0)).on_press(Message::BrowseCancel),
                    add,
                ])
                .spacing(8.0)
                .width(Length::FILL),
            ])
            .spacing(12.0)
            .width(Length::FILL)
            .height(Length::FILL),
        )
        // Below the frosted bar, as every page starts (page_layout): this view
        // doesn't scroll, so its heading would stay under the bar (techbench,
        // 2026-11-17).
        .padding(Padding { top: 22.0 + self.top.get(), bottom: 22.0, left: 22.0, right: 22.0 })
        .width(Length::FILL)
        .height(Length::FILL)
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn wallpaper_message(&mut self, m: Message) {
        match m {
            Message::PickPicture(i) => {
                if let Some(p) = self.files.get(i).cloned() {
                    self.save_wallpaper(Some(Some(&p)), None, None);
                }
            }
            Message::Fit(i) => self.save_wallpaper(None, Some(wall::MODES[i.min(3)]), None),
            Message::Background(t) => self.bg_text = t,
            Message::BackgroundSubmit => {
                let c = self.bg_text.trim().to_string();
                if parse_hex(&c).is_some() {
                    self.save_wallpaper(None, None, Some(&c));
                } else {
                    self.status = "Background color: write it as #rrggbb".into();
                }
            }
            Message::SameAsPrimary(same) => {
                if let Some((id, label)) = self.scope() {
                    let shown = wall::shown(&self.desktop, Some(&id));
                    self.save(|d| {
                        if same {
                            wall::clear_own(d, &id);
                        } else {
                            wall::set(d, Some(&id), &label, shown.image.as_deref(), &shown.mode, &shown.color);
                        }
                    });
                }
            }
            Message::MakePrimary => {
                if let Some(new) = self.wall_output().or_else(|| self.current_output()).map(|o| o.id.clone()) {
                    self.make_main(&new);
                }
            }
            Message::RemoveSource(i) => {
                let (mut folders, mut files) = wall::sources(&self.desktop);
                if i < folders.len() {
                    folders.remove(i);
                } else if i - folders.len() < files.len() {
                    files.remove(i - folders.len());
                }
                self.save(|d| wall::set_sources(d, &folders, &files));
                self.load_pictures();
            }
            Message::Browse(folders) => {
                let pics = home().join("Pictures");
                let dir = if pics.is_dir() { pics } else { home() };
                self.browser = Some(Browser { folders, entries: list(&dir), dir, picked: Vec::new() });
            }
            Message::BrowseOpen(i) => {
                if let Some(b) = self.browser.as_mut()
                    && let Some((p, folder)) = b.entries.get(i).cloned()
                {
                    if folder {
                        b.entries = list(&p);
                        b.dir = p;
                    } else if !b.folders {
                        match b.picked.iter().position(|x| *x == p) {
                            Some(k) => {
                                b.picked.remove(k);
                            }
                            None => b.picked.push(p),
                        }
                    }
                }
            }
            Message::BrowseUp => {
                if let Some(b) = self.browser.as_mut()
                    && let Some(parent) = b.dir.parent().map(Path::to_path_buf)
                {
                    b.entries = list(&parent);
                    b.dir = parent;
                }
            }
            Message::BrowseAdd => {
                if let Some(b) = self.browser.take() {
                    let (mut folders, mut files) = wall::sources(&self.desktop);
                    if b.folders {
                        if !folders.contains(&b.dir) {
                            folders.push(b.dir);
                        }
                    } else {
                        for p in b.picked {
                            if !files.contains(&p) {
                                files.push(p);
                            }
                        }
                    }
                    self.save(|d| wall::set_sources(d, &folders, &files));
                    self.load_pictures();
                }
            }
            Message::BrowseCancel => self.browser = None,
            _ => {}
        }
    }
}

/// A folder's sub-folders, then its pictures.
fn list(dir: &Path) -> Vec<(PathBuf, bool)> {
    let (dirs, pics) = wall::list_dir(dir);
    dirs.into_iter().map(|d| (d, true)).chain(pics.into_iter().map(|p| (p, false))).collect()
}
