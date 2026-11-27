//! The Screens pages (Page::Overview, Page::Screen): the arrangement, each
//! screen's scale, size of controls and main screen (desktop.toml
//! `[displays]`, as disp.rs edits it; resolutions are modes.rs).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

const SCREEN_CONTROLS_NAMES: [&str; 4] = ["Automatic", "Compact", "Standard", "Large"];

/// A screen being dragged on the overview: which, where the press was, how
/// far it has gone (in the drawing's units).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArrangeDrag {
    index: usize,
    start: (f32, f32),
    offset: (f32, f32),
}

impl App {
    /// The pointer or a finger on the overview's arrangement: a press on a
    /// screen picks it up (a second within 400 ms opens its page), a move
    /// drags it, letting go puts it down where it snaps.
    fn arrange_touch(&mut self, t: Touch) {
        let size = self.arrange_size.get();
        match t {
            Touch::Press { x, y } => {
                let Some(i) = tiles(&self.outputs, size).iter().position(|r| inside(r, x, y)) else { return };
                if let Some((j, at)) = self.arrange_click
                    && j == i
                    && at.elapsed() < Duration::from_millis(400)
                {
                    self.arrange_click = None;
                    self.arrange_drag = None;
                    self.open(Page::Screen(i));
                    return;
                }
                self.arrange_click = Some((i, Instant::now()));
                self.arrange_drag = Some(ArrangeDrag { index: i, start: (x, y), offset: (0.0, 0.0) });
            }
            Touch::Move { x, y, pressed: true } => {
                if let Some(d) = self.arrange_drag.as_mut() {
                    d.offset = (x - d.start.0, y - d.start.1);
                }
            }
            Touch::Release { .. } | Touch::Leave => {
                let Some(d) = self.arrange_drag.take() else { return };
                if d.offset.0.abs() < 2.0 && d.offset.1.abs() < 2.0 || d.index >= self.outputs.len() {
                    return;
                }
                // Back to desktop pixels, as fractions of the arrangement's size.
                let (x0, y0, bw, bh) = bbox(&self.outputs);
                let (_, k) = area(&self.outputs, size);
                let o = &self.outputs[d.index];
                let fx = ((o.x - x0) as f32 + d.offset.0 / k) / bw as f32;
                let fy = ((o.y - y0) as f32 + d.offset.1 / k) / bh as f32;
                let mut rects: Vec<disp::Rect> = self
                    .outputs
                    .iter()
                    .map(|o| disp::Rect { id: o.id.clone(), x: o.x, y: o.y, w: o.width, h: o.height })
                    .collect();
                let (x, y) = (x0 + (fx * bw as f32).round() as i32, y0 + (fy * bh as f32).round() as i32);
                let (nx, ny) = disp::snap(&rects, d.index, x, y, (bw.max(bh) / 40).max(20));
                rects[d.index].x = nx;
                rects[d.index].y = ny;
                let setup = self.setup.clone();
                self.save(|dt| disp::set_arrangement(dt, &setup, &rects));
            }
            Touch::Move { .. } => {}
        }
    }

    /// The screens as arranged, each with its wallpaper and name.
    /// `interactive`: the overview's (drag; double-click for a screen's
    /// page); else a screen page's map, the screen being edited framed in the
    /// accent and a press on another going to its page.
    fn arrangement(&self, highlight: Option<usize>, interactive: bool, height: f32) -> Element<Message> {
        let screens: Vec<(OutputInfo, Option<Arc<Image>>)> = self
            .outputs
            .iter()
            .map(|o| {
                (o.clone(), wall::shown(&self.desktop, Some(&o.id)).image.and_then(|p| self.thumbs.get(&p).cloned()))
            })
            .collect();
        let drag = if interactive { self.arrange_drag } else { None };
        let size_out = self.arrange_size.clone();
        let (accent, line) = (self.accent(), self.line_color());
        let d = drawing(move |scene: &mut Scene, fonts: &mut Fonts, r: Rect, t: &Theme| {
            size_out.set((r.w, r.h));
            let outputs: Vec<OutputInfo> = screens.iter().map(|s| s.0.clone()).collect();
            let face = fonts.face(&t.font, 400, false).or_else(|| fonts.face("sans-serif", 400, false));
            for (i, mut tile) in tiles(&outputs, (r.w, r.h)).into_iter().enumerate() {
                if let Some(d) = drag.filter(|d| d.index == i) {
                    tile.x += d.offset.0;
                    tile.y += d.offset.1;
                }
                let tile = Rect::new(r.x + tile.x, r.y + tile.y, tile.w, tile.h);
                let (o, thumb) = &screens[i];
                scene.fill(tile, Color::rgb(0, 0, 0));
                if let Some(img) = thumb {
                    let id = scene.image(img.clone());
                    scene.draw_image(id, tile, 1.0);
                }
                let band = 34.0f32.min(tile.h / 2.0);
                scene.fill(Rect::new(tile.x, tile.y + tile.h - band, tile.w, band), Color::rgba(0, 0, 0, 153));
                if let Some(f) = face {
                    let m = fonts.metrics(f, 11.0);
                    let lines = [screen_label(o), format!("{:.0}%", o.scale * 100.0)];
                    for (k, s) in lines.iter().enumerate() {
                        let w = fonts.layout(f, 11.0, s).1;
                        let base = tile.y + tile.h - band + 3.0 + m.ascent + k as f32 * 14.0;
                        scene.push_clip(tile, 0.0);
                        text(
                            scene,
                            fonts,
                            f,
                            11.0,
                            WHITE,
                            Point::new((tile.x + (tile.w - w) / 2.0).round(), base.round()),
                            s,
                        );
                        scene.pop_clip();
                    }
                }
                let framed = highlight == Some(i) || drag.is_some_and(|d| d.index == i);
                let (c, w) = if framed { (accent, 3.0) } else { (line, 1.0) };
                scene.stroke(tile, 0.0, w, c);
            }
        })
        .height(Length::Fixed(height))
        .name(if interactive { "Screen arrangement" } else { "Screens" });
        if interactive {
            d.on_touch(|t| (!matches!(t, Touch::Move { pressed: false, .. })).then_some(Message::Arrange(t)))
        } else {
            d.on_touch(|t| matches!(t, Touch::Press { .. }).then_some(Message::Map(t)))
        }
    }

    pub(crate) fn overview(&self) -> Element<Message> {
        // Every screen's scale here, each on its own, since monitors differ
        // and each needs its own scale, and the main screen; the size of
        // everything is Accessibility's (see docs/settings-milestone-2.md).
        let primary = self.primary();
        let mut scales: Vec<Element<Message>> = Vec::new();
        for (i, o) in self.outputs.iter().enumerate() {
            let v = match self.scale_drag {
                Some((j, v)) if j == i => v,
                _ => o.base_scale as f32,
            };
            let marks = scale_marks(o);
            let note = if (v as f64 - o.default_scale).abs() < 1e-6 {
                format!("{:.0}%, the sharpest for it.", o.default_scale * 100.0)
            } else {
                format!("Its default is {:.0}%.", o.default_scale * 100.0)
            };
            scales.push(setting(&screen_label(o), &note, scale_slider_of(i, v, o, &marks)));
        }
        let mut screens_group = scales;
        if self.outputs.len() > 1 {
            let names: Vec<String> = self.outputs.iter().map(screen_label).collect();
            let at = primary.as_ref().and_then(|p| self.outputs.iter().position(|o| &o.id == p));
            screens_group.push(setting(
                "Main screen",
                "The dock stands on it, and its wallpaper is the default for the others.",
                choice(names, at, Message::MainScreen).width(Length::Fixed(200.0)).name("Main screen"),
            ));
        }
        let mut items = vec![
            self.group(None, vec![panel(self.arrangement(None, true, 240.0)).padding(8.0).width(Length::FILL)]),
            label("Double-click a screen for its own settings.").size(11.0).dim(),
            self.group(Some("Scale"), screens_group),
        ];
        if (self.size - 1.0).abs() > 0.004 {
            items.push(self.group(
                None,
                vec![setting(
                        "Size in Accessibility",
                        &format!(
                            "Everything is {}% bigger on every screen, on top of these scales.",
                            ((self.size - 1.0) * 100.0).round()
                        ),
                        button(label("Accessibility"))
                            .on_press(Message::Go(Page::Accessibility))
                            .name("Open Accessibility"),
                    )],
            ));
        }
        self.page_layout(
            "Screens",
            "Drag the screens to where they sit on your desk. Each one's scale is worked out from its size and how far away it is, or chosen here.",
            {
                // Out of the way in Advanced, showing each screen: the scales
                // above are the setting most people need; these two fine-tune
                // them (see docs/settings-milestone-2.md).
                items.push(self.advanced("screens", vec![self.same_size_row()]));
                items.push(self.status_line());
                items
            },
        )
    }

    /// "Windows the same size on every screen", with what each screen has
    /// now and why, so a soft screen says so (see docs/settings-milestone-2.md).
    fn same_size_row(&self) -> Element<Message> {
        let on = disp::same_size(&self.desktop);
        let screens: Vec<String> = self
            .outputs
            .iter()
            .map(|o| {
                let why = if disp::screen(&self.desktop, &o.id).1.is_some() {
                    "chosen"
                } else if o.base_scale.fract() == 0.0 {
                    "sharpest"
                } else if on {
                    "the same size as the others"
                } else {
                    "its own scale"
                };
                format!("{} {:.0}% ({why})", screen_label(o), o.base_scale * 100.0)
            })
            .collect();
        let note = format!(
            "On: each screen at its calculated scale, so a window keeps its size moving between screens; a whole scale wins where it is within 8%. Off: the nearest whole scale where it is close, the sharpest. Now: {}.",
            if screens.is_empty() { "no screens".to_string() } else { screens.join(", ") }
        );
        setting(
            "Windows the same size on every screen",
            &note,
            switch(on, Message::SameSize).name("Windows the same size on every screen"),
        )
    }

    pub(crate) fn screen(&self) -> Element<Message> {
        let multi = self.outputs.len() > 1;
        let o = self.current_output();
        let title = o.map(screen_label).unwrap_or_else(|| "Screens".into());
        // What the slider's marks are (Dave, 2026-11-22: no mention that the
        // ticks are the evenly dividing scales, or which is recommended).
        let description = if multi {
            "Its scale and where it sits, drawn at the screen's own pixels. The marks on the slider are the sharp scales: tall at whole numbers and the recommended one, short where the screen's pixels divide evenly."
        } else {
            "Its scale, drawn at the screen's own pixels. The marks on the slider are the sharp scales: tall at whole numbers and the recommended one, short where the screen's pixels divide evenly."
        };
        let mut items: Vec<Element<Message>> = Vec::new();
        if multi && let Page::Screen(i) = self.page {
            items.push(
                self.group(None, vec![panel(self.arrangement(Some(i), false, 110.0)).padding(6.0).width(Length::FILL)]),
            );
        }
        if let Some(o) = o {
            let marks = scale_marks(o);
            let note = if (self.scale as f64 - o.default_scale).abs() < 1e-6 {
                format!("{:.0}%, recommended: the sharpest for this screen.", o.default_scale * 100.0)
            } else if self.scale.fract() == 0.0 {
                format!("Whole number: sharp in every app. Recommended: {:.0}%.", o.default_scale * 100.0)
            } else if scale_detents(o).1.iter().any(|c| (c - self.scale as f64).abs() < 1e-4) {
                format!(
                    "Divides the screen's pixels evenly: sharp in apps that draw at any scale. Recommended: {:.0}%.",
                    o.default_scale * 100.0
                )
            } else {
                format!("In between: soft in some older apps. Recommended: {:.0}%.", o.default_scale * 100.0)
            };
            let chosen = disp::screen(&self.desktop, &o.id).1.is_some();
            let mut shown_row = vec![label(format!("{:.0}%", o.scale * 100.0)).dim()];
            if chosen {
                shown_row.push(
                    button(label("Default").size(12.0)).padding(Padding::hv(3.0, 10.0)).on_press(Message::ScaleDefault),
                );
            }
            let screen_controls: Vec<String> = SCREEN_CONTROLS_NAMES.iter().map(|s| s.to_string()).collect();
            let mode_rows = self.mode_rows(o);
            let mut rows = vec![setting("Scale", &note, scale_slider(self.scale, o, &marks))];
            // Its brightness, where it has a control (brightness milestone 1).
            if let Some((connector, level)) = self.brightness.iter().find(|(c, _)| *c == o.name).cloned() {
                rows.push(setting(
                    "Brightness",
                    &format!("{level}%"),
                    slider(0.0, 100.0, level as f32, move |v| Message::Brightness(connector.clone(), v))
                        .step(1.0)
                        .width(Length::Fixed(220.0))
                        // A laptop's panel by what it is, as the Display tile says.
                        .name(format!(
                            "{} brightness",
                            if vergence_dm::brightness::is_internal(&o.name) {
                                "Built-in display".to_string()
                            } else {
                                screen_label(o)
                            }
                        )),
                ));
            }
            rows.extend([
                setting(
                    "Shown at",
                    &format!(
                        "With Accessibility's Size: it looks like {} \u{d7} {} ({} \u{d7} {} pixels).",
                        o.width,
                        o.height,
                        (o.width as f64 * o.scale).round() as i64,
                        (o.height as f64 * o.scale).round() as i64
                    ),
                    row(shown_row).spacing(8.0).align(Align::Center),
                ),
                setting(
                    "Size of controls",
                    "Automatic: Large with a touch panel, else the desktop's (Appearance).",
                    choice(screen_controls, Some(disp::controls(&self.desktop, &o.id)), Message::ScreenControls)
                        .width(Length::Fixed(160.0))
                        .name("Size of controls"),
                ),
            ]);
            if multi {
                let is_main = self.primary().as_deref() == Some(o.id.as_str());
                rows.push(setting(
                    "Main screen",
                    if is_main {
                        "The dock stands on it, and its wallpaper is the default for the others."
                    } else {
                        "Make it the main screen: the dock moves to it, and its wallpaper becomes the default."
                    },
                    if is_main {
                        label("Main screen").dim()
                    } else {
                        button(label("Make main screen").size(12.0))
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Message::MakePrimary)
                    },
                ));
            }
            rows.extend(mode_rows);
            // A screen on one GPU drawn by another (GPUs milestone 1) says so up front.
            if !o.shown_by.is_empty() && o.shown_by != o.drawn_by {
                rows.push(setting(
                    "Graphics",
                    &format!("Shown by {}, drawn by {} and copied there.", o.shown_by, o.drawn_by),
                    space(),
                ));
            }
            items.push(self.group(Some("Display"), rows));
        }
        // Its picture is chosen on the Wallpaper page (settings milestone 1).
        if let Page::Screen(i) = self.page {
            let shown = wall::shown(&self.desktop, self.scope().as_ref().map(|(id, _)| id.as_str()));
            let what = shown
                .image
                .as_deref()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("Color {}", shown.color));
            items.push(self.group(
                Some("Wallpaper"),
                vec![setting(
                        &short_name(&what, 40),
                        "Chosen on the Wallpaper page.",
                        button(label("Wallpaper").size(12.0))
                            .padding(Padding::hv(4.0, 12.0))
                            .on_press(Message::Go(Page::Wallpaper(i)))
                            .name("Open Wallpaper"),
                    )],
            ));
        }
        // How the scale was found and what the screen is, for the curious.
        if let Some(o) = o {
            // With one screen the fine-tuning lives here (with several, on the
            // Screens page's own Advanced): it still decides for screens met later.
            let mut rows = if multi { Vec::new() } else { vec![self.same_size_row()] };
            rows.push(setting("How the scale was found", &o.note, space()));
            if !o.shown_by.is_empty() {
                let g = if o.shown_by == o.drawn_by {
                    o.shown_by.clone()
                } else {
                    format!("Shown by {}, drawn by {}", o.shown_by, o.drawn_by)
                };
                rows.push(setting("Graphics", &g, space()));
            }
            // A laptop's panel often has no make and model: its connector stands in.
            if !o.description.is_empty() && o.description != o.name {
                rows.push(setting("Monitor", &o.description, space()));
            }
            rows.push(setting("Connector", &o.name, space()));
            rows.push(setting("Size", &format!("{} x {} points at {}, {}", o.width, o.height, o.x, o.y), space()));
            rows.push(setting(
                "Identity",
                &format!("{} (from its EDID: its settings follow it to any port)", o.id),
                space(),
            ));
            items.push(self.advanced("screen", rows));
        }
        items.push(self.status_line());
        self.page_layout(&title, description, items)
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn screens_message(&mut self, m: Message) {
        match m {
            Message::Brightness(connector, v) => {
                let level = v.round().clamp(0.0, 100.0) as u32;
                let Some(entry) = self.brightness.iter_mut().find(|(c, _)| *c == connector) else { return };
                entry.1 = level;
                let jobs = self.brightness_jobs.get_or_insert_with(|| {
                    let (tx, rx) = std::sync::mpsc::channel::<(String, u32)>();
                    let _ = std::thread::Builder::new().name("brightness".into()).spawn(move || brightness_work(rx));
                    tx
                });
                let _ = jobs.send((connector, level));
            }
            Message::ScreenControls(i) => {
                if let Some(o) = self.current_output() {
                    let (id, label) = (o.id.clone(), screen_label(o));
                    self.save(|d| disp::set_controls(d, &id, &label, i));
                }
            }
            Message::SameSize(on) => self.save(|d| disp::set_same_size(d, on)),
            Message::Scale(v) => {
                self.scale = match self.current_output() {
                    Some(o) => {
                        let (strong, clean) = scale_detents(o);
                        disp::snap_scale(v as f64, &strong, &clean) as f32
                    }
                    None => v,
                };
            }
            Message::ScaleDone => {
                if let Some(o) = self.current_output() {
                    let v = (self.scale as f64 * 120.0).round() / 120.0;
                    let choice = ((v - o.default_scale).abs() > 1e-6).then_some(v);
                    let (id, label) = (o.id.clone(), screen_label(o));
                    self.save(|d| disp::set_scale(d, &id, &label, choice));
                }
            }
            Message::ScaleOf(i, v) => {
                if let Some(o) = self.outputs.get(i) {
                    let (strong, clean) = scale_detents(o);
                    let v = disp::snap_scale(v as f64, &strong, &clean) as f32;
                    self.scale_drag = Some((i, v));
                }
            }
            Message::ScaleOfDone(i) => {
                if let (Some((j, v)), Some(o)) = (self.scale_drag.take(), self.outputs.get(i))
                    && j == i
                {
                    let v = (v as f64 * 120.0).round() / 120.0;
                    let choice = ((v - o.default_scale).abs() > 1e-6).then_some(v);
                    let (id, label) = (o.id.clone(), screen_label(o));
                    self.save(|d| disp::set_scale(d, &id, &label, choice));
                }
            }
            Message::MainScreen(i) => {
                if let Some(new) = self.outputs.get(i).map(|o| o.id.clone()) {
                    self.make_main(&new);
                }
            }
            Message::ScaleDefault => {
                if let Some(o) = self.current_output() {
                    let (id, label, d) = (o.id.clone(), screen_label(o), o.default_scale);
                    self.scale = d as f32;
                    self.save(|d| disp::set_scale(d, &id, &label, None));
                }
            }
            Message::Arrange(t) => self.arrange_touch(t),
            Message::Map(Touch::Press { x, y }) => {
                let size = self.arrange_size.get();
                if let Some(i) = tiles(&self.outputs, size).iter().position(|r| inside(r, x, y)) {
                    self.open(Page::Screen(i));
                }
            }
            Message::Map(_) => {}
            _ => {}
        }
    }
}

/// The slider's top: 300%, since no one runs a monitor past that, or a
/// screen's own default where that is higher.
fn scale_max(o: &OutputInfo) -> f64 {
    o.default_scale.ceil().max(3.0)
}

/// A screen's scale marks: the strong ones (whole numbers, its default) and
/// the clean ones, where its running mode is a whole number of points.
fn scale_detents(o: &OutputInfo) -> (Vec<f64>, Vec<f64>) {
    let max = scale_max(o);
    let clean = match o.modes.get(o.mode as usize) {
        Some(m) => disp::clean_scales(m.width, m.height, 0.75, max),
        None => Vec::new(),
    };
    (disp::detents(o.default_scale, max), clean)
}

/// Every mark a screen's scale slider shows.
fn scale_marks(o: &OutputInfo) -> Vec<f64> {
    let (strong, clean) = scale_detents(o);
    disp::all_marks(&strong, &clean)
}

/// A screen's scale slider, with marks at the whole numbers and the default.
fn scale_slider(v: f32, o: &OutputInfo, marks: &[f64]) -> Element<Message> {
    let max = scale_max(o) as f32;
    let marks: Vec<f32> = marks.iter().map(|m| *m as f32).collect();
    row(vec![
        slider(0.75, max, v, Message::Scale)
            .step(0.01)
            .marks(marks)
            .on_release(Message::ScaleDone)
            .width(Length::Fixed(260.0))
            .name("Scale"),
        percent(v),
    ])
    .spacing(8.0)
    .align(Align::Center)
}

/// Screen `i`'s scale slider on the Screens overview.
fn scale_slider_of(i: usize, v: f32, o: &OutputInfo, marks: &[f64]) -> Element<Message> {
    let max = scale_max(o) as f32;
    let marks: Vec<f32> = marks.iter().map(|m| *m as f32).collect();
    row(vec![
        slider(0.75, max, v, move |x| Message::ScaleOf(i, x))
            .step(0.01)
            .marks(marks)
            .on_release(Message::ScaleOfDone(i))
            .width(Length::Fixed(220.0))
            .name("Scale"),
        percent(v),
    ])
    .spacing(8.0)
    .align(Align::Center)
}

fn bbox(outputs: &[OutputInfo]) -> (i32, i32, i32, i32) {
    let x0 = outputs.iter().map(|o| o.x).min().unwrap_or(0);
    let y0 = outputs.iter().map(|o| o.y).min().unwrap_or(0);
    let x1 = outputs.iter().map(|o| o.x + o.width).max().unwrap_or(1);
    let y1 = outputs.iter().map(|o| o.y + o.height).max().unwrap_or(1);
    (x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

/// Room around the arrangement in its drawing, and the gap between screens.
const MARGIN: f32 = 20.0;
const GAP: f32 = 3.0;

/// The arrangement's place in a drawing `size` big: its corner, and drawing
/// units to the desktop pixel.
fn area(outputs: &[OutputInfo], size: (f32, f32)) -> ((f32, f32), f32) {
    let (_, _, bw, bh) = bbox(outputs);
    let k = ((size.0 - 2.0 * MARGIN) / bw as f32).min((size.1 - 2.0 * MARGIN) / bh as f32).max(0.0001);
    (((size.0 - bw as f32 * k) / 2.0, (size.1 - bh as f32 * k) / 2.0), k)
}

/// Each screen's rectangle in a drawing `size` big, in its own coordinates.
fn tiles(outputs: &[OutputInfo], size: (f32, f32)) -> Vec<Rect> {
    let (x0, y0, _, _) = bbox(outputs);
    let (origin, k) = area(outputs, size);
    outputs
        .iter()
        .map(|s| {
            Rect::new(
                origin.0 + (s.x - x0) as f32 * k + GAP,
                origin.1 + (s.y - y0) as f32 * k + GAP,
                s.width as f32 * k - 2.0 * GAP,
                s.height as f32 * k - 2.0 * GAP,
            )
        })
        .collect()
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

/// Each screen with a brightness control and its level now (readable by
/// anyone: the panel's own file).
pub(crate) fn brightness_now() -> Vec<(String, u32)> {
    vergence_dm::brightness::controls()
        .iter()
        .filter_map(|c| vergence_dm::brightness::level(c).map(|l| (c.connector.clone(), l)))
        .collect()
}

/// Sets what the sliders asked, through vdm: of a drag only the latest for
/// each screen (vdm answers one at a time).
fn brightness_work(jobs: std::sync::mpsc::Receiver<(String, u32)>) {
    while let Ok(first) = jobs.recv() {
        let mut wanted = std::collections::HashMap::new();
        for (c, l) in std::iter::once(first).chain(std::iter::from_fn(|| jobs.try_recv().ok())) {
            wanted.insert(c, l);
        }
        for (c, l) in wanted {
            if let Err(e) = vergence_dm::brightness::ask(&c, l) {
                eprintln!("settings: brightness of {c}: {e}");
            }
        }
    }
}
