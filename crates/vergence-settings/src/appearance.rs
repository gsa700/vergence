//! The Appearance page (Page::Appearance): style, accent, the frost, the
//! title bar's shade, the tab in front, the wallpaper tint, under the bar and
//! the size of controls (desktop.toml [appearance]).
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

const SCHEMES: [&str; 3] = ["default", "dark", "light"];

const SCHEME_NAMES: [&str; 3] = ["App default", "Dark", "Light"];
/// The Frost choice's names and their values in desktop.toml (Standard when unset).
const FROST_NAMES: [&str; 3] = ["Clear", "Standard", "Heavy"];
const FROST_VALUES: [&str; 3] = ["clear", "standard", "heavy"];
/// Title bar shade (`[appearance] bar-shade`, vergence-look bar_look_shaded):
/// the names shown and their steps.
const SHADE_CHOICES: [(&str, i64); 5] =
    [("Lighter", -1), ("Standard", 0), ("A little darker", 1), ("Darker", 2), ("Darkest", 3)];
/// The tab in front: the page's own color, or frosted (page when unset).
const FRONT_TAB_NAMES: [&str; 2] = ["Page color", "Frosted"];
const FRONT_TAB_VALUES: [&str; 2] = ["page", "frosted"];
/// The size of controls (Standard when unset); a screen's adds Automatic first.
const CONTROLS_NAMES: [&str; 3] = ["Compact", "Standard", "Large"];
const CONTROLS_VALUES: [&str; 3] = ["compact", "standard", "large"];

const SWATCHES: [&str; 8] = ["#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#e05a00", "#eab308", "#22c55e", "#6b7280"];

impl App {
    pub(crate) fn save_appearance(&mut self, f: impl FnOnce(&mut toml::Table)) {
        self.save_table("appearance", f);
    }

    fn save_accent(&mut self) {
        let a = self.accent_text.trim().to_string();
        if !a.is_empty() && parse_hex(&a).is_none() {
            self.status = "Accent color: write it as #rrggbb, or leave it empty".into();
            return;
        }
        self.save_appearance(|t| {
            if a.is_empty() {
                t.remove("accent");
            } else {
                t.insert("accent".into(), a.into());
            }
        });
    }

    pub(crate) fn appearance(&self) -> Element<Message> {
        let scheme = SCHEMES.iter().position(|s| *s == appearance_text(&self.desktop, "color-scheme")).unwrap_or(0);
        let accent = appearance_text(&self.desktop, "accent");
        let none_selected = accent.is_empty();
        let text_c = self.text_color();
        let line_c = self.line_color();
        let mut swatches = vec![
            button(label("\u{2715}").size(10.0).dim())
                .padding(0.0)
                .width(Length::Fixed(22.0))
                .height(Length::Fixed(22.0))
                .look(ButtonLook::Custom {
                    fill: None,
                    edge: Some(if none_selected { (text_c, 2.0) } else { (line_c, 1.0) }),
                    radius: 11.0,
                })
                .on_press(Message::Accent(None))
                .name("No accent"),
        ];
        for hex in SWATCHES {
            let col = parse_hex(hex).unwrap_or(Color::rgb(0, 0, 0));
            let selected = accent.eq_ignore_ascii_case(hex);
            swatches.push(
                button(space())
                    .padding(0.0)
                    .width(Length::Fixed(22.0))
                    .height(Length::Fixed(22.0))
                    .look(ButtonLook::Custom { fill: Some(col), edge: selected.then_some((text_c, 2.0)), radius: 11.0 })
                    .on_press(Message::Accent(Some(hex)))
                    .name(format!("Accent {hex}")),
            );
        }
        let contrast = appearance_text(&self.desktop, "contrast") == "high";
        let motion = self
            .desktop
            .get("appearance")
            .and_then(|a| a.get("reduced-motion"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let names: Vec<String> = SCHEME_NAMES.iter().map(|s| s.to_string()).collect();
        let transparency = self
            .desktop
            .get("appearance")
            .and_then(|a| a.get("reduce-transparency"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let frost = FROST_VALUES.iter().position(|v| *v == appearance_text(&self.desktop, "frost")).unwrap_or(1);
        let front =
            FRONT_TAB_VALUES.iter().position(|v| *v == appearance_text(&self.desktop, "front-tab")).unwrap_or(0);
        let frost_names: Vec<String> = FROST_NAMES.iter().map(|s| s.to_string()).collect();
        let front_names: Vec<String> = FRONT_TAB_NAMES.iter().map(|s| s.to_string()).collect();
        let controls =
            CONTROLS_VALUES.iter().position(|v| *v == appearance_text(&self.desktop, "controls")).unwrap_or(1);
        let controls_names: Vec<String> = CONTROLS_NAMES.iter().map(|s| s.to_string()).collect();
        self.page_layout(
            "Appearance",
            "Light or dark, the accent color, contrast and motion. Every app that follows the desktop changes at once.",
            vec![
                self.group(None, vec![
                    setting("Style", "App default lets each app look as it chooses (its own settings, Ctrl+, in it); Dark or Light is for every app. Settings windows stay light under App default, to be read easily.", choice(names, Some(scheme), Message::Scheme).width(Length::Fixed(160.0)).name("Style")),
                    setting("Accent color", if accent.is_empty() { "None: each app uses its own." } else { "" }, row(swatches).spacing(6.0).align(Align::Center)),
                    setting(
                        "Custom accent",
                        "Any color as #rrggbb; Enter applies it.",
                        field(&self.accent_text, Message::AccentText).placeholder("#rrggbb").on_submit(Message::AccentSubmit).width(Length::Fixed(120.0)).name("Custom accent"),
                    ),
                ]),
                self.group(Some("Desktop look"), vec![
                    setting(
                        "Title bar shade",
                        "Darker sets the tabs off from the bar. Lighter is for light bars only. Titles stay readable at every step.",
                        {
                            let shade = self.desktop.get("appearance").and_then(|a| a.get("bar-shade")).and_then(|v| v.as_integer()).unwrap_or(0);
                            let names: Vec<String> = SHADE_CHOICES.iter().map(|(n, _)| n.to_string()).collect();
                            choice(names, SHADE_CHOICES.iter().position(|(_, s)| *s == shade), Message::BarShade).width(Length::Fixed(160.0)).name("Title bar shade")
                        },
                    ),
                    setting("Frost", "How much of a window shows through its bar. Titles stay readable whatever is under them.", choice(frost_names, Some(frost), Message::Frost).width(Length::Fixed(160.0)).name("Frost")),
                    setting(
                        "Under the bar",
                        "What scrolls up under a window's bar: kept faint so the bar's titles are crisp over anything, or shown more, the titles still readable.",
                        {
                            let more = self.desktop.get("appearance").and_then(|a| a.get("under-bar")).and_then(|v| v.as_str()) == Some("more");
                            choice(vec!["Titles first".to_string(), "Show more".to_string()], Some(more as usize), Message::UnderBar).width(Length::Fixed(160.0)).name("Under the bar")
                        },
                    ),
                    setting("Tab in front", "In the page's own color, joined to it, or frosted like the rest of the bar.", choice(front_names, Some(front), Message::FrontTab).width(Length::Fixed(160.0)).name("Tab in front")),
                    setting("Size of controls", "Window bars, tabs and buttons. A screen with a touch panel uses Large unless its own setting says otherwise (Screens).", choice(controls_names, Some(controls), Message::Controls).width(Length::Fixed(160.0)).name("Size of controls")),
                    setting(
                        "Tint windows with the wallpaper's color",
                        "In dark style, a window takes on the color (not the brightness) of the wallpaper behind it: 0 (off) to 60%; 25% unless set. Off with Reduce transparency and High contrast.",
                        row(vec![
                            slider(0.0, 60.0, self.tint_pct, Message::TintSlide).step(5.0).on_release(Message::TintDone).width(Length::Fixed(160.0)).name("Tint windows with the wallpaper's color"),
                            label(if self.tint_pct <= 0.0 { "Off".to_string() } else { format!("{:.0}%", self.tint_pct) }).width(Length::Fixed(44.0)).align(Align::End),
                        ])
                        .spacing(8.0)
                        .align(Align::Center),
                    ),
                ]),
                self.group(None, vec![setting(
                    "Accessibility",
                    &format!("High contrast {}, reduce motion {}, reduce transparency {}: on their own page now.", if contrast { "on" } else { "off" }, if motion { "on" } else { "off" }, if transparency { "on" } else { "off" }),
                    button(label("Accessibility")).on_press(Message::Go(Page::Accessibility)).name("Open Accessibility"),
                )]),
                self.status_line(),
            ],
        )
    }

    /// This page's messages, as main.rs's handle routes them here.
    pub(crate) fn appearance_message(&mut self, m: Message) {
        match m {
            Message::Scheme(i) => self.save_appearance(|t| {
                t.insert("color-scheme".into(), SCHEMES[i.min(2)].into());
            }),
            Message::Accent(hex) => {
                self.accent_text = hex.unwrap_or("").to_string();
                self.save_accent();
            }
            Message::AccentText(t) => self.accent_text = t,
            Message::AccentSubmit => self.save_accent(),
            Message::BarShade(i) => self.save_appearance(|t| {
                t.insert("bar-shade".into(), SHADE_CHOICES[i.min(SHADE_CHOICES.len() - 1)].1.into());
            }),
            Message::Frost(i) => self.save_appearance(|t| {
                t.insert("frost".into(), FROST_VALUES[i.min(2)].into());
            }),
            Message::FrontTab(i) => self.save_appearance(|t| {
                t.insert("front-tab".into(), FRONT_TAB_VALUES[i.min(1)].into());
            }),
            Message::UnderBar(k) => self.save_appearance(|t| {
                if k == 1 {
                    t.insert("under-bar".into(), "more".into());
                } else {
                    t.remove("under-bar");
                }
            }),
            Message::TintSlide(v) => self.tint_pct = v,
            Message::TintDone => {
                // 0 is off (the server's switch); otherwise the strength in fives
                // (tint-strength, the color-only tint; the old tint-amount goes).
                let p = (self.tint_pct / 5.0).round() * 5.0;
                self.save_appearance(|t| {
                    t.insert("wallpaper-tint".into(), (p > 0.0).into());
                    t.remove("tint-amount");
                    if p > 0.0 {
                        t.insert("tint-strength".into(), toml::Value::Integer(p as i64));
                    } else {
                        t.remove("tint-strength");
                    }
                });
            }
            Message::Controls(i) => self.save_appearance(|t| {
                t.insert("controls".into(), CONTROLS_VALUES[i.min(2)].into());
            }),
            _ => {}
        }
    }
}
