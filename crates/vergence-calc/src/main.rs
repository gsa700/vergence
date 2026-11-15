//! Vergence's calculator (tools milestone 1, docs/tools-milestone-1.md): a
//! keypad window (Dave: "a basic calculator with advanced scientific as an
//! option"), exact decimal arithmetic of its own (dec.rs), the expression
//! typed shown above its result, a live answer while typing, and this
//! session's calculations above it, each one taken back by a click. The
//! scientific keys open on the left (View › Scientific in its menu). Keys
//! from the keyboard and the number pad; Ctrl+C copies the answer. Its
//! settings (Ctrl+,): angles in degrees or radians, decimal places, the
//! thousands grouped, and its look.

use std::cell::Cell;
use std::rc::Rc;
use std::time::SystemTime;

use vergence_calc::dec::{CalcError, DIGITS, Dec};
use vergence_calc::expr::{Context, eval};
use vergence_ui::{AppCommand as C, BarStyle, Role};
use vergence_widgets::prefs::{Choice, Control, Part, Prefs, Row, Rows};
use vergence_widgets::{Align, ButtonLook, Element, Input, Length, Padding, Program, Theme, Waker, Window, button, column, label, panel, row, space};

const SECTION: &str = "[calculator]";
const APP_ID: &str = "vergence-calc";
/// The room the scientific keys take: four keys and a gap.
const SCI_W: f32 = 320.0;

/// What the display shows below the expression.
#[derive(Debug, Clone)]
enum Shown {
    /// Typing: the answer so far, if it is one.
    Typing,
    Answer(Dec),
    Error(CalcError),
}

#[derive(Debug, Clone)]
enum Msg {
    /// Text to add to the expression ("7", "+", "sin(").
    Put(&'static str),
    Equals,
    Clear,
    Back,
    Sign,
    Angle,
    /// Take back a calculation of the tape (its answer).
    Tape(usize),
}

struct Calc {
    expr: String,
    shown: Shown,
    ans: Option<Dec>,
    /// This session's calculations: what was typed and its answer, shown.
    tape: Vec<(String, String)>,
    scientific: bool,
    desktop: String,
    stamp: Option<SystemTime>,
    prefs_open: Rc<Cell<bool>>,
    waker: Option<Waker>,
    /// Its window's size now, in points.
    size: (f32, f32),
}

/// ~/.config/vergence/desktop.toml.
fn desktop_text() -> (String, Option<SystemTime>) {
    match vergence_widgets::prefs::desktop_path() {
        Some(p) => (std::fs::read_to_string(&p).unwrap_or_default(), std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        None => (String::new(), None),
    }
}

impl Calc {
    fn new() -> Calc {
        let (desktop, stamp) = desktop_text();
        let scientific = vergence_ui::table_value(&desktop, SECTION, "scientific").as_deref() == Some("true");
        Calc { expr: String::new(), shown: Shown::Typing, ans: None, tape: Vec::new(), scientific, desktop, stamp, prefs_open: Rc::new(Cell::new(false)), waker: None, size: (0.0, 0.0) }
    }

    fn degrees(&self) -> bool {
        vergence_ui::table_value(&self.desktop, SECTION, "angle").as_deref() != Some("radians")
    }

    fn context(&self) -> Context {
        Context { degrees: self.degrees(), ans: self.ans.clone() }
    }

    /// desktop.toml again, if it changed.
    fn reread(&mut self) -> bool {
        let (text, stamp) = desktop_text();
        if stamp == self.stamp {
            return false;
        }
        (self.desktop, self.stamp) = (text, stamp);
        true
    }

    /// A number as the settings say to show it: decimal places, grouping.
    fn format(&self, v: &Dec) -> String {
        let places = vergence_ui::table_value(&self.desktop, SECTION, "places").and_then(|p| p.parse::<usize>().ok());
        let s = match places {
            Some(n) if n <= 12 => v.show_fixed(n),
            _ => v.show(16),
        };
        if vergence_ui::table_value(&self.desktop, SECTION, "separators").as_deref() == Some("false") { s } else { group(&s) }
    }

    #[cfg(test)]
    fn answer(&self) -> Option<&Dec> {
        match &self.shown {
            Shown::Answer(v) => Some(v),
            _ => None,
        }
    }

    fn put(&mut self, text: &str) {
        if let Shown::Answer(v) = &self.shown {
            // An operator goes on from the answer; anything else starts afresh.
            let goes_on = matches!(text, "+" | "−" | "×" | "÷" | "^" | "%" | "!" | "^2");
            self.expr = if goes_on { v.show(DIGITS).replace('-', "−") } else { String::new() };
        } else if let Shown::Error(_) = self.shown {
            self.expr.clear();
        }
        self.shown = Shown::Typing;
        self.expr.push_str(text);
    }

    fn equals(&mut self) {
        if self.expr.trim().is_empty() {
            return;
        }
        match eval(&self.expr, &self.context()) {
            Ok(v) => {
                // Brackets left open are closed in the tape, as they were worked out.
                let open = self.expr.matches('(').count().saturating_sub(self.expr.matches(')').count());
                self.tape.push((format!("{}{}", self.expr, ")".repeat(open)), self.format(&v)));
                if self.tape.len() > 200 {
                    self.tape.remove(0);
                }
                self.ans = Some(v.clone());
                self.shown = Shown::Answer(v);
            }
            Err(e) => self.shown = Shown::Error(e),
        }
    }

    fn sign(&mut self) {
        if let Shown::Answer(v) = &self.shown {
            let n = v.neg();
            self.expr = n.show(DIGITS).replace('-', "−");
            self.shown = Shown::Answer(n);
            return;
        }
        if let Some(inner) = self.expr.strip_prefix("−(").and_then(|r| r.strip_suffix(')')) {
            self.expr = inner.to_string();
        } else if !self.expr.is_empty() {
            self.expr = format!("−({})", self.expr);
        } else {
            self.expr.push('−');
        }
        self.shown = Shown::Typing;
    }

    fn open_settings(&mut self, window: &mut Window) {
        if self.prefs_open.get() {
            return;
        }
        let waker = self.waker.clone();
        let prefs = Prefs::new("Calculator", APP_ID, SECTION, false, Box::new(CalcRows), self.prefs_open.clone()).on_write(move || {
            if let Some(w) = &waker {
                w.wake();
            }
        });
        window.open_window(vergence_widgets::window(prefs));
    }

    fn key(&mut self, sym: u32, text: Option<&str>, ctrl: bool, window: &mut Window) -> bool {
        const RETURN: u32 = 0xff0d;
        const KP_ENTER: u32 = 0xff8d;
        const BACKSPACE: u32 = 0xff08;
        const ESCAPE: u32 = 0xff1b;
        const DELETE: u32 = 0xffff;
        if ctrl {
            match sym {
                0x63 | 0x43 => {
                    self.copy(window);
                    return false;
                }
                0x76 | 0x56 => {
                    window.paste();
                    return false;
                }
                0x2c => {
                    self.open_settings(window);
                    return false;
                }
                _ => return false,
            }
        }
        match sym {
            RETURN | KP_ENTER => self.equals(),
            BACKSPACE => self.update_back(),
            ESCAPE | DELETE => self.clear(),
            _ => {
                let Some(t) = text else { return false };
                let put = match t {
                    "0" => "0",
                    "1" => "1",
                    "2" => "2",
                    "3" => "3",
                    "4" => "4",
                    "5" => "5",
                    "6" => "6",
                    "7" => "7",
                    "8" => "8",
                    "9" => "9",
                    "." | "," => ".",
                    "+" => "+",
                    "-" => "−",
                    "*" | "x" => "×",
                    "/" => "÷",
                    "^" => "^",
                    "(" => "(",
                    ")" => ")",
                    "%" => "%",
                    "!" => "!",
                    "=" => {
                        self.equals();
                        return true;
                    }
                    _ => return false,
                };
                self.put(put);
            }
        }
        true
    }

    fn update_back(&mut self) {
        if matches!(self.shown, Shown::Answer(_) | Shown::Error(_)) {
            self.shown = Shown::Typing;
            return;
        }
        // A function's name goes as one ("sin(").
        for f in ["asin(", "acos(", "atan(", "sin(", "cos(", "tan(", "ln(", "log(", "Ans"] {
            if self.expr.ends_with(f) {
                self.expr.truncate(self.expr.len() - f.len());
                return;
            }
        }
        self.expr.pop();
    }

    fn clear(&mut self) {
        self.expr.clear();
        self.shown = Shown::Typing;
    }

    fn copy(&mut self, window: &mut Window) {
        let text = match &self.shown {
            Shown::Answer(v) => v.show(DIGITS),
            _ => match eval(&self.expr, &self.context()) {
                Ok(v) => v.show(DIGITS),
                Err(_) => return,
            },
        };
        window.copy(&text);
    }

    fn key_button(&self, words: &'static str, m: Msg, look: ButtonLook) -> Element<Msg> {
        let name = match words {
            "−" => "minus",
            "×" => "times",
            "÷" => "divide",
            "⌫" => "backspace",
            "±" => "change sign",
            "√" => "square root",
            other => other,
        };
        button(label(words).size(18.0).align(Align::Center).width(Length::FILL)).look(look).padding(Padding::hv(4.0, 4.0)).on_press(m).width(Length::FILL).height(Length::FILL).name(name)
    }
}

/// "1234567.5" as "1,234,567.5"; scientific form and short numbers as they are.
fn group(s: &str) -> String {
    if s.contains('e') {
        return s.to_string();
    }
    let (sign, rest) = s.strip_prefix('-').map_or(("", s), |r| ("-", r));
    let (int, frac) = rest.split_once('.').map_or((rest, None), |(i, f)| (i, Some(f)));
    if int.len() <= 4 {
        return s.to_string();
    }
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

/// The calculator's command ids.
mod cmd {
    pub const NEW_WINDOW: u32 = 1;
    pub const COPY: u32 = 2;
    pub const PASTE: u32 = 3;
    pub const SCIENTIFIC: u32 = 4;
    pub const DEGREES: u32 = 5;
    pub const SETTINGS: u32 = 6;
    pub const ABOUT: u32 = 7;
    pub const CLEAR: u32 = 8;
}

impl Program for Calc {
    type Message = Msg;

    fn title(&self) -> String {
        "Calculator".into()
    }

    fn app_id(&self) -> String {
        APP_ID.into()
    }

    /// Light under App default, as the settings are (its built-in look).
    fn theme(&self) -> Theme {
        Theme::from_desktop(&self.desktop, false, SECTION)
    }

    fn bar_style(&self) -> Option<BarStyle> {
        Some(vergence_ui::bar_style_setting(&self.desktop, SECTION, BarStyle::Light))
    }

    fn view(&self, t: &Theme) -> Element<Msg> {
        // The display: the tape's last lines, the expression, the answer.
        let mut lines: Vec<Element<Msg>> = Vec::new();
        let shown_tape = self.tape.len().saturating_sub(3);
        for (i, (e, r)) in self.tape.iter().enumerate().skip(shown_tape) {
            lines.push(
                button(label(format!("{e} = {r}")).size(12.0).dim().align(Align::End).width(Length::FILL))
                    .look(ButtonLook::Plain)
                    .padding(Padding::hv(0.0, 2.0))
                    .on_press(Msg::Tape(i))
                    .width(Length::FILL)
                    .name(format!("Earlier: {e} = {r}")),
            );
        }
        let expr = if self.expr.is_empty() { " ".to_string() } else { self.expr.clone() };
        lines.push(label(expr).size(16.0).dim().align(Align::End).width(Length::FILL));
        let (big, dim) = match &self.shown {
            Shown::Answer(v) => (self.format(v), false),
            Shown::Error(e) => (e.to_string(), true),
            Shown::Typing => match eval(&self.expr, &self.context()) {
                Ok(v) if !self.expr.is_empty() => (self.format(&v), true),
                _ => (if self.expr.is_empty() { "0".into() } else { " ".into() }, true),
            },
        };
        let big = label(big).size(if matches!(self.shown, Shown::Error(_)) { 20.0 } else { 34.0 }).weight(500).align(Align::End).width(Length::FILL);
        lines.push(if dim { big.dim() } else { big });
        let display = panel(column(lines).spacing(2.0).width(Length::FILL)).color(t.card).radius(10.0).padding(Padding::hv(10.0, 14.0)).width(Length::FILL);

        let n = ButtonLook::Normal;
        let k = |w: &'static str| self.key_button(w, Msg::Put(w), n);
        let basic: Vec<Vec<Element<Msg>>> = vec![
            vec![self.key_button("C", Msg::Clear, n), self.key_button("⌫", Msg::Back, n), k("%"), k("÷")],
            vec![k("7"), k("8"), k("9"), k("×")],
            vec![k("4"), k("5"), k("6"), k("−")],
            vec![k("1"), k("2"), k("3"), k("+")],
            vec![self.key_button("±", Msg::Sign, n), k("0"), k("."), self.key_button("=", Msg::Equals, ButtonLook::Accent)],
        ];
        let angle = if self.degrees() { "Deg" } else { "Rad" };
        let mut rows: Vec<Element<Msg>> = Vec::new();
        for (i, keys) in basic.into_iter().enumerate() {
            let mut all: Vec<Element<Msg>> = Vec::new();
            // The scientific keys, on the left, a little apart.
            if self.scientific {
                let row_sci: Vec<Element<Msg>> = match i {
                    0 => vec![self.key_button("sin", Msg::Put("sin("), n), self.key_button("cos", Msg::Put("cos("), n), self.key_button("tan", Msg::Put("tan("), n), self.key_button(angle, Msg::Angle, n)],
                    1 => vec![self.key_button("asin", Msg::Put("asin("), n), self.key_button("acos", Msg::Put("acos("), n), self.key_button("atan", Msg::Put("atan("), n), self.key_button("π", Msg::Put("π"), n)],
                    2 => vec![self.key_button("ln", Msg::Put("ln("), n), self.key_button("log", Msg::Put("log("), n), self.key_button("√", Msg::Put("√"), n), self.key_button("e", Msg::Put("e"), n)],
                    3 => vec![self.key_button("x²", Msg::Put("^2"), n), self.key_button("xʸ", Msg::Put("^"), n), self.key_button("(", Msg::Put("("), n), self.key_button(")", Msg::Put(")"), n)],
                    _ => vec![self.key_button("n!", Msg::Put("!"), n), self.key_button("1/x", Msg::Put("^−1"), n), self.key_button("Ans", Msg::Put("Ans"), n), space().width(Length::FILL).height(Length::FILL)],
                };
                all.extend(row_sci);
                all.push(space().width(Length::Fixed(6.0)));
            }
            all.extend(keys);
            rows.push(row(all).spacing(6.0).width(Length::FILL).height(Length::FILL));
        }
        panel(column(vec![display, column(rows).spacing(6.0).width(Length::FILL).height(Length::FILL)]).spacing(10.0).width(Length::FILL).height(Length::FILL))
            .padding(Padding { top: 12.0 + t.top, bottom: 12.0, left: 12.0, right: 12.0 })
            .radius(0.0)
            .width(Length::FILL)
            .height(Length::FILL)
            .into()
    }

    fn update(&mut self, m: Msg, window: &mut Window) {
        match m {
            Msg::Put(t) => self.put(t),
            Msg::Equals => self.equals(),
            Msg::Clear => self.clear(),
            Msg::Back => self.update_back(),
            Msg::Sign => self.sign(),
            Msg::Angle => self.toggle_angle(),
            Msg::Tape(i) => {
                if let Some((_, r)) = self.tape.get(i) {
                    let plain: String = r.chars().filter(|c| *c != ',').collect();
                    if let Shown::Answer(_) | Shown::Error(_) = self.shown {
                        self.expr.clear();
                        self.shown = Shown::Typing;
                    }
                    self.expr.push_str(&plain.replace('-', "−"));
                }
            }
        }
        window.redraw();
    }

    /// Its keys before the widgets: Enter is =, never the button last clicked.
    fn key_first(&mut self, input: &Input, window: &mut Window) -> Option<bool> {
        match input {
            Input::Key { sym, text, pressed: true, modifiers, .. } if !modifiers.alt && !modifiers.logo => {
                self.key(*sym, text.as_deref(), modifiers.ctrl, window).then_some(true)
            }
            _ => None,
        }
    }

    fn input(&mut self, input: &Input, _window: &mut Window) -> bool {
        match input {
            Input::Paste(text) => {
                let clean: String = text.chars().filter(|c| c.is_ascii_digit() || ".+-*/()%^".contains(*c)).collect();
                if !clean.is_empty() {
                    self.put("");
                    self.expr.push_str(&clean.replace('-', "−").replace('*', "×").replace('/', "÷"));
                }
                true
            }
            _ => false,
        }
    }

    fn waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn resized(&mut self, width: f32, height: f32) {
        self.size = (width, height);
    }

    fn woken(&mut self, _window: &mut Window) -> bool {
        self.reread()
    }

    fn commands(&self) -> Option<(bool, Vec<C>)> {
        Some((
            false,
            vec![
                C::role(cmd::NEW_WINDOW, Role::NewWindow).featured(),
                C::role(cmd::COPY, Role::Copy).featured(),
                C::role(cmd::PASTE, Role::Paste),
                C::own(cmd::SCIENTIFIC, "View", "Scientific", "").checked(self.scientific).featured(),
                C::own(cmd::DEGREES, "View", "Angles in degrees", "").checked(self.degrees()),
                C::own(cmd::CLEAR, "Edit", "Clear", "Escape"),
                C::role(cmd::SETTINGS, Role::Settings),
                C::role(cmd::ABOUT, Role::About),
            ],
        ))
    }

    fn command(&mut self, id: u32, window: &mut Window) {
        match id {
            cmd::NEW_WINDOW => {
                if let Ok(me) = std::env::current_exe() {
                    let _ = std::process::Command::new(me).spawn();
                }
            }
            cmd::COPY => self.copy(window),
            cmd::PASTE => window.paste(),
            cmd::SCIENTIFIC => self.toggle_scientific(window),
            cmd::DEGREES => self.toggle_angle(),
            cmd::CLEAR => self.clear(),
            cmd::SETTINGS => self.open_settings(window),
            cmd::ABOUT => {
                let about = vergence_widgets::about::About::new(APP_ID, "Calculator", env!("CARGO_PKG_VERSION"), "A calculator with exact decimal arithmetic to 34 digits, so 0.1 + 0.2 is 0.3; a scientific mode with angles in degrees or radians.", self.theme());
                window.open_window(vergence_widgets::window(about));
            }
            _ => {}
        }
        window.redraw();
    }

    fn style(&self) -> vergence_ui::Style {
        vergence_ui::Style { min: (300.0, 440.0), ..Default::default() }
    }
}

impl Calc {
    /// The scientific keys, on or off, kept in its table so it opens the same
    /// way; the window asks for the room they take (or gives it back).
    fn toggle_scientific(&mut self, window: &mut Window) {
        self.scientific = !self.scientific;
        self.write_key("scientific", if self.scientific { Some("true") } else { None });
        let (w, h) = self.size;
        if w > 0.0 {
            let w = if self.scientific { w + SCI_W } else { (w - SCI_W).max(300.0) };
            window.request_size(w.round() as u32, h.round() as u32);
        }
    }

    fn toggle_angle(&mut self) {
        let degrees = self.degrees();
        self.write_key("angle", if degrees { Some("\"radians\"") } else { None });
    }

    fn write_key(&mut self, key: &str, value: Option<&str>) {
        let Some(path) = vergence_widgets::prefs::desktop_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = vergence_widgets::prefs::set_key(&text, SECTION, key, value);
        if new != text {
            let beside = path.with_extension("toml.new");
            if std::fs::write(&beside, &new).and_then(|_| std::fs::rename(&beside, &path)).is_ok() {
                self.reread();
            }
        }
    }
}

/// Its settings window's rows.
struct CalcRows;

impl Rows for CalcRows {
    fn parts(&self, text: &str) -> Vec<Part> {
        let mut places = vec![Choice::text("As many as needed", "auto")];
        places.extend((0..=10).map(|n| Choice::number(&n.to_string(), n)));
        let opacity = vergence_ui::opacity_setting(text, "[appearance]");
        let locked = vergence_ui::desktop_pick(text).map(|light| format!("{} for every app: Settings > Appearance > Style", if light { "Light" } else { "Dark" }));
        let styles = vec![Choice::text("Dark", "dark"), Choice::text("Light", "light")];
        vec![
            Part {
                title: "Calculator".into(),
                rows: vec![
                    Row::new("angle", "Angles", "For sin, cos and tan and their inverses.", Control::Choose { choices: vec![Choice::text("Degrees", "degrees"), Choice::text("Radians", "radians")], desktop: None, app_default: 0 }),
                    Row::new("places", "Decimal places", "How many digits after the point an answer shows.", Control::Choose { choices: places, desktop: None, app_default: 0 }),
                    Row::new("separators", "Group thousands", "1,234,567 rather than 1234567.", Control::Switch { default: true }),
                    Row::new("scientific", "Scientific keys", "Also View › Scientific in its menu.", Control::Switch { default: false }),
                ],
            },
            Part {
                title: "On this desktop".into(),
                rows: vec![
                    Row::new("style", "Style", "While the desktop's Style is App default.", Control::Choose { choices: styles.clone(), desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked.clone()),
                    Row::new("bar-style", "Title bar", "", Control::Choose { choices: styles, desktop: Some("Built-in (Light)".into()), app_default: 0 }).locked(locked),
                    Row::new("opacity", "Opacity", "30 to 100: how much of the window's ground shows; the text stays as it is.", Control::Percent { min: 30, desktop: format!("Desktop default ({:.0})", opacity * 100.0), now: (opacity * 100.0).round() as u32 }),
                    Row::new("", "Window places", "Where the desktop puts the calculator's windows, for each set of screens.", Control::Button { label: "Forget…".into(), id: 1 }),
                ],
            },
        ]
    }

    fn button(&self, id: u32) {
        if id == 1 {
            let _ = std::process::Command::new("vergence-settings").args(["--page", "windows", "--forget", APP_ID]).spawn();
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-calc {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let calc = Calc::new();
    let width = if calc.scientific { 360 + SCI_W as u32 } else { 360 };
    if let Err(e) = vergence_widgets::run(calc, (width, 560)) {
        eprintln!("vergence-calc: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped_but_not_short_numbers_or_exponents() {
        assert_eq!(group("1234567.5"), "1,234,567.5");
        assert_eq!(group("-12345"), "-12,345");
        assert_eq!(group("1234"), "1234");
        assert_eq!(group("1.5e+25"), "1.5e+25");
        assert_eq!(group("0.000123"), "0.000123");
    }

    #[test]
    fn typing_after_an_answer_goes_on_or_starts_afresh() {
        let mut c = Calc { expr: String::new(), shown: Shown::Typing, ans: None, tape: Vec::new(), scientific: false, desktop: String::new(), stamp: None, prefs_open: Rc::new(Cell::new(false)), waker: None, size: (0.0, 0.0) };
        for t in ["2", "+", "3"] {
            c.put(t);
        }
        c.equals();
        assert_eq!(c.answer().map(|v| v.show(16)), Some("5".into()));
        assert_eq!(c.tape, vec![("2+3".to_string(), "5".to_string())]);
        c.put("×");
        c.put("4");
        c.equals();
        assert_eq!(c.answer().map(|v| v.show(16)), Some("20".into()), "× goes on from 5");
        c.put("7");
        assert_eq!(c.expr, "7", "a digit starts afresh");
        c.sign();
        assert_eq!(c.expr, "−(7)");
        c.sign();
        assert_eq!(c.expr, "7");
        c.put("÷");
        c.put("0");
        c.equals();
        assert!(matches!(c.shown, Shown::Error(CalcError::DivideByZero)));
        c.put("sin(");
        c.update_back();
        assert_eq!(c.expr, "", "a function goes as one");
        c.put("sin(");
        c.put("30");
        c.equals();
        assert_eq!(c.tape.last().map(|t| t.0.as_str()), Some("sin(30)"), "the tape closes the bracket");
    }
}
