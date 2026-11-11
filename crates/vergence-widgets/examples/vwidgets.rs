//! The widgets, in a window: `cargo run --release --example vwidgets`.
//! `VERGENCE_UI_TREE=1` prints what each widget means.

use std::sync::Arc;

use vergence_scene::{Color, Image, Point, Rect};
use vergence_widgets::{
    Align, ButtonLook, Element, Fit, Length, Padding, Program, Theme, Touch, Window, button, choice, column, drawing, field, label, panel, picture, row, rule, run, scroll, slider, space, switch,
};

#[derive(Debug, Clone)]
enum Msg {
    Count(i32),
    Side(bool),
    Dark(bool),
    Repeat(bool),
    Delay(f32),
    DelayDone,
    Accent(String),
    AccentApply,
    Secret(String),
    Fit(usize),
    Dot(Touch),
}

struct Demo {
    count: i32,
    left: bool,
    dark: bool,
    repeat: bool,
    delay: f32,
    released: u32,
    accent: String,
    applied: Option<Color>,
    secret: String,
    fit: usize,
    dot: (f32, f32),
    dragging: bool,
    picture: Arc<Image>,
}

const FITS: [&str; 3] = ["Contain", "Cover", "Stretch"];

fn desktop() -> String {
    let dir = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")));
    dir.and_then(|d| std::fs::read_to_string(d.join("vergence/desktop.toml")).ok()).unwrap_or_default()
}

/// A picture to show: a wash of colour, 160 x 90.
fn wash() -> Image {
    let (w, h) = (160u32, 90u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, 180, 255]);
        }
    }
    Image { width: w, height: h, rgba }
}

impl Program for Demo {
    type Message = Msg;

    fn title(&self) -> String {
        "Widgets".into()
    }

    fn app_id(&self) -> String {
        "vwidgets".into()
    }

    fn theme(&self) -> Theme {
        let base = Theme::from_desktop(&desktop(), self.dark, "[widgets]");
        let mut t = Theme::new(self.dark, self.applied.unwrap_or(base.accent), base.high_contrast);
        t.opacity = base.opacity;
        t.reduced_motion = base.reduced_motion;
        t
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        let side = |left: bool, name: &str| button(label(name)).look(if self.left == left { ButtonLook::Chosen } else { ButtonLook::Normal }).on_press(Msg::Side(left));
        let setting = |title: &str, note: &str, control: Element<Msg>| {
            row(vec![column(vec![label(title), label(note).dim().size(11.0).wrap()]).spacing(2.0).width(Length::FILL), control]).spacing(16.0).align(Align::Center).width(Length::FILL)
        };
        let card = |items: Vec<Element<Msg>>| {
            let mut with_rules = Vec::new();
            for (i, item) in items.into_iter().enumerate() {
                if i > 0 {
                    with_rules.push(rule());
                }
                with_rules.push(item);
            }
            panel(column(with_rules).spacing(10.0).width(Length::FILL)).color(t.card).padding(Padding::all(14.0)).width(Length::FILL)
        };
        let buttons = card(vec![
            setting("Count", &format!("Pressed so far: {}.", self.count), row(vec![button(label("\u{2212}")).name("Less").on_press(Msg::Count(-1)), button(label("+")).name("More").on_press(Msg::Count(1))]).spacing(6.0)),
            setting("Controls", "Left, as on a Mac; right, as on Windows.", row(vec![side(true, "Left"), side(false, "Right")]).spacing(6.0)),
            setting("Look", "Dark or light, for this window only.", button(label(if self.dark { "Light" } else { "Dark" })).look(ButtonLook::Accent).on_press(Msg::Dark(!self.dark))),
            setting("Off", "A button with nothing to do.", button(label("Nothing"))),
        ]);
        let mut repeat_rows = vec![setting("Key repeat", "Holding a key down types it again and again.", switch(self.repeat, Msg::Repeat).name("Key repeat"))];
        if self.repeat {
            repeat_rows.push(setting(
                "Delay",
                &format!("{:.0} ms; let go {} times.", self.delay, self.released),
                slider(150.0, 1000.0, self.delay, Msg::Delay).step(10.0).marks(vec![150.0, 500.0, 1000.0]).on_release(Msg::DelayDone).name("Delay"),
            ));
        }
        repeat_rows.push(setting("Fixed", "A switch that cannot be changed.", switch(true, Msg::Repeat).disabled().name("Fixed")));
        let controls = card(repeat_rows);
        let fields = card(vec![
            setting(
                "Custom accent",
                "Any colour as #rrggbb; Enter applies it to this window.",
                field(&self.accent, Msg::Accent).placeholder("#rrggbb").on_submit(Msg::AccentApply).width(Length::Fixed(140.0)).name("Custom accent"),
            ),
            setting(
                "Password",
                &format!("Never shown, copied or told; {} characters.", self.secret.chars().count()),
                field(&self.secret, Msg::Secret).secret().placeholder("secret").width(Length::Fixed(140.0)).name("Password"),
            ),
            setting("Fit", "How the picture below fills its place.", choice(FITS.iter().map(|s| s.to_string()).collect(), Some(self.fit), Msg::Fit).width(Length::Fixed(140.0)).name("Fit")),
        ]);
        let fit = [Fit::Contain, Fit::Cover, Fit::Stretch][self.fit];
        let (dx, dy, dragging) = (self.dot.0, self.dot.1, self.dragging);
        let pictures = card(vec![
            row(vec![
                picture(self.picture.clone()).fit(fit).radius(8.0).width(Length::Fixed(200.0)).height(Length::Fixed(80.0)).name("A wash of colour"),
                drawing(move |scene, r, t| {
                    scene.fill_rounded(r, 8.0, t.page);
                    scene.stroke(r, 8.0, 1.0, t.line);
                    let d = if dragging { 22.0 } else { 18.0 };
                    scene.fill_rounded(Rect::new(r.x + dx - d / 2.0, r.y + dy - d / 2.0, d, d), d / 2.0, t.accent);
                    scene.line(Point::new(r.x + 8.0, r.y + r.h - 8.0), Point::new(r.x + dx, r.y + dy), 1.0, t.dim);
                })
                .on_touch(|t| (!matches!(t, Touch::Move { pressed: false, .. })).then_some(Msg::Dot(t)))
                .height(Length::Fixed(80.0))
                .name("Drag the dot"),
            ])
            .spacing(12.0)
            .width(Length::FILL),
        ]);
        let page = column(vec![
            label("Widgets").size(20.0).weight(600),
            label("Tab moves between the controls; Enter or Space presses. A finger works as the pointer does, and drags the page.").dim().wrap().width(Length::FILL),
            space().height(Length::Fixed(6.0)),
            buttons,
            controls,
            fields,
            pictures,
        ])
        .spacing(10.0)
        .width(Length::FILL)
        .max_width(560.0);
        scroll(panel(page).padding(Padding::all(24.0)).align_x(Align::Center).width(Length::FILL))
    }

    fn update(&mut self, m: Msg, _: &mut Window) {
        match m {
            Msg::Count(d) => self.count += d,
            Msg::Side(left) => self.left = left,
            Msg::Dark(d) => self.dark = d,
            Msg::Repeat(on) => self.repeat = on,
            Msg::Delay(v) => self.delay = v,
            Msg::DelayDone => self.released += 1,
            Msg::Accent(s) => self.accent = s,
            Msg::AccentApply => self.applied = vergence_widgets::parse_hex(&self.accent),
            Msg::Secret(s) => self.secret = s,
            Msg::Fit(i) => self.fit = i,
            Msg::Dot(t) => match t {
                Touch::Press { x, y } => {
                    self.dot = (x, y);
                    self.dragging = true;
                }
                Touch::Move { x, y, pressed: true } => self.dot = (x.max(0.0), y.clamp(0.0, 80.0)),
                Touch::Release { .. } | Touch::Leave => self.dragging = false,
                Touch::Move { .. } => {}
            },
        }
    }
}

fn main() -> std::io::Result<()> {
    run(
        Demo {
            count: 0,
            left: true,
            dark: true,
            repeat: true,
            delay: 500.0,
            released: 0,
            accent: String::new(),
            applied: None,
            secret: String::new(),
            fit: 0,
            dot: (40.0, 30.0),
            dragging: false,
            picture: Arc::new(wash()),
        },
        (640, 560),
    )
}
