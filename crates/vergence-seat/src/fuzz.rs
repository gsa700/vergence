//! Random event streams into the seat: never a panic, never two targets
//! entered at once, motion only to the entered target, and every press's
//! release goes where the press went (and no release without a press).
//! Deterministic (a fixed seed); FUZZ_ITERS raises the count for a long run:
//! `FUZZ_ITERS=1000000 cargo test --release -p vergence-seat fuzz`.

use crate::keyboard::{Key, KeyOut, Mods, Switch};
use crate::*;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn iters() -> usize {
    std::env::var("FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000)
}

fn target(rng: &mut Rng) -> Option<Target> {
    let w = 1 + rng.below(3) as WinId;
    Some(match rng.below(14) {
        0 => return None,
        1..=3 => Target::Surface(w, w * 10),
        4..=5 => Target::Native(w),
        6 => Target::Server(Part::Title(w)),
        7 => Target::Server(Part::Edge(w, Edges::RIGHT)),
        8 => Target::Server([Part::Close(w), Part::Maximize(w), Part::Minimize(w), Part::Tab(w, 1), Part::CloseTab(w, 0), Part::Plus(w), Part::AppMenu(w), Part::AppList][rng.below(8)]),
        9 => Target::Server(Part::Dock),
        10 => Target::Server(Part::Menu),
        11 => Target::Server(Part::Popup),
        12 => Target::Server([Part::Note, Part::Switcher, Part::Blocked(w)][rng.below(3)]),
        _ => Target::Server(Part::Desktop),
    })
}

/// What the stream has said so far, to check each new answer against.
#[derive(Default)]
struct Watch {
    entered: Option<Target>,
    /// Each held button and where its press went.
    pressed: Vec<(u32, Target)>,
}

impl Watch {
    fn check(&mut self, out: &[Out], step: usize) {
        for o in out {
            match *o {
                Out::Enter(t) => {
                    assert!(self.entered.is_none(), "step {step}: entered {t:?} while {:?} was", self.entered);
                    self.entered = Some(t);
                }
                Out::Leave(t) => {
                    assert_eq!(self.entered, Some(t), "step {step}: left {t:?}, not the entered one");
                    self.entered = None;
                }
                // A long press on the dock ends the left press with no release on
                // purpose: it is the icon's menu, not a click (touch.rs).
                Out::DockMenu => self.pressed.retain(|(b, t)| !(*b == BTN_LEFT && *t == Target::Server(Part::Dock))),
                Out::Motion(t) => assert_eq!(self.entered, Some(t), "step {step}: motion to {t:?}, not the entered one"),
                Out::Press(t, b) => {
                    assert!(!self.pressed.iter().any(|(x, _)| *x == b), "step {step}: {b:#x} pressed twice");
                    self.pressed.push((b, t));
                }
                Out::Release(t, b) => {
                    let i = self.pressed.iter().position(|(x, _)| *x == b).unwrap_or_else(|| panic!("step {step}: release of {b:#x} with no press"));
                    assert_eq!(self.pressed[i].1, t, "step {step}: {b:#x} released somewhere other than its press");
                    self.pressed.remove(i);
                }
                _ => {}
            }
        }
    }

    /// The seat forgot things without a word (a window went, a drag began).
    fn forget_window(&mut self, w: WinId) {
        if self.entered.and_then(|t| t.window()) == Some(w) {
            self.entered = None;
            self.pressed.clear();
        }
        self.pressed.retain(|(_, t)| t.window() != Some(w));
    }
}

#[test]
fn fuzz_the_seat() {
    let mut rng = Rng::new(0x5EA7_2026_1112);
    let mut seat = Seat::new();
    let mut watch = Watch::default();
    let mut now = 0u64;
    let buttons = [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE];
    // How often each kind of answer came: a stream that never reaches a rule tests nothing.
    let mut seen = [0usize; 8];
    // The same for the keyboard: switching begun, ended, a window key.
    let mut keys_seen = [0usize; 3];
    // The last long-press timer the seat asked for: firing it is the case that matters.
    let mut timer = 0u64;
    let keys = [Key::BackSpace, Key::Escape, Key::Super, Key::VolumeUp, Key::Mute, Key::Vt(2), Key::Vt(0), Key::Tab, Key::Grave, Key::Alt, Key::Left, Key::Up, Key::Space, Key::Other];
    for step in 0..iters() {
        now += rng.below(500) as u64;
        let mods = Mods { ctrl: rng.below(8) == 0, alt: rng.below(3) == 0, shift: rng.below(6) == 0, logo: rng.below(8) == 0 };
        let out = match rng.below(20) {
            0..=4 => seat.motion(target(&mut rng)),
            5..=8 => {
                let b = buttons[rng.below(3)];
                seat.button(b, rng.below(2) == 0, target(&mut rng), mods, now)
            }
            9 => seat.axis(target(&mut rng)),
            10 => {
                let k = keys[rng.below(keys.len())];
                match seat.key(k, rng.below(2) == 0, mods) {
                    KeyOut::Switch(Switch::Begin { .. }) => {
                        assert!(seat.switching(), "step {step}: switching began but the seat is not switching");
                        keys_seen[0] += 1;
                    }
                    KeyOut::Switch(Switch::Choose | Switch::Cancel) => {
                        assert!(!seat.switching(), "step {step}: switching ended but the seat still switches");
                        keys_seen[1] += 1;
                    }
                    KeyOut::Window(_) => keys_seen[2] += 1,
                    _ => {}
                }
                Vec::new()
            }
            11 => {
                if rng.below(2) == 0 {
                    seat.menu_opened([Menu::Menu, Menu::Popup, Menu::AppMenu][rng.below(3)]);
                } else {
                    seat.menu_closed();
                }
                Vec::new()
            }
            12 => {
                let w = 1 + rng.below(3) as WinId;
                let out = seat.window_gone(w);
                watch.forget_window(w);
                out
            }
            13 => {
                if rng.below(4) == 0 {
                    let out = seat.edge_begin();
                    watch.check(&out, step);
                    watch.pressed.clear();
                    Vec::new()
                } else {
                    seat.edge_end(target(&mut rng))
                }
            }
            14 => seat.touch_down(rng.below(3) as Slot, rng.below(2000) as f64, rng.below(1200) as f64, target(&mut rng), now),
            15 => seat.touch_motion(rng.below(3) as Slot, rng.below(2000) as f64, rng.below(1200) as f64, target(&mut rng)),
            16 => seat.touch_up(rng.below(3) as Slot, target(&mut rng), now),
            17 => {
                let press = if rng.below(4) == 0 { rng.below(40) as u64 } else { timer };
                seat.touch_long_press(press)
            }
            18 if rng.below(3) == 0 => {
                let w = 1 + rng.below(3) as WinId;
                let out = seat.grab_from_press(w, [None, Some(Edges::LEFT)][rng.below(2)]);
                // The held press became the grab's: no release will come for it.
                if !out.is_empty() {
                    watch.check(&out, step);
                    watch.pressed.clear();
                    Vec::new()
                } else {
                    out
                }
            }
            18 => {
                let _ = seat.touch_move_request(rng.below(3) as Slot, 1 + rng.below(3) as WinId, 0.0, 0.0);
                if rng.below(30) == 0 { seat.touch_cancel(target(&mut rng), now) } else { Vec::new() }
            }
            _ => {
                if rng.below(50) == 0 {
                    seat.screens_off()
                } else {
                    seat.focus(Some(1 + rng.below(3) as WinId));
                    Vec::new()
                }
            }
        };
        watch.check(&out, step);
        for o in &out {
            if let Out::LongPressTimer(n) = o {
                timer = *n;
            }
            let k = match o {
                Out::Enter(_) => 0,
                Out::Press(..) => 1,
                Out::Release(..) => 2,
                Out::MoveBegin(_) | Out::ResizeBegin(..) => 3,
                Out::GrabEnd(_) => 4,
                Out::CloseMenu => 5,
                Out::TouchDown(..) => 6,
                Out::DockMenu | Out::DesktopMenu => 7,
                _ => continue,
            };
            seen[k] += 1;
        }
        // The seat and the watch agree on what the pointer is on.
        assert_eq!(seat.pointer_focus(), watch.entered, "step {step}: the seat's focus and its enters disagree");
        // The order of use: no window twice, and the focused one first.
        let used = seat.order_of_use();
        assert!(used.iter().enumerate().all(|(i, w)| !used[..i].contains(w)), "step {step}: a window twice in the order of use: {used:?}");
        if let Some(f) = seat.keyboard_focus() {
            assert_eq!(used.first(), Some(&f), "step {step}: the focused window is not the most recently used");
        }
    }
    if iters() >= 100_000 {
        assert!(seen.iter().all(|n| *n > 0), "some rules never ran: {seen:?} (enter, press, release, grab, grab end, close menu, touch down, long press)");
        assert!(keys_seen.iter().all(|n| *n > 0), "some key rules never ran: {keys_seen:?} (switch begin, switch end, window key)");
    }
    eprintln!("fuzz: {} events; enter {}, press {}, release {}, grab {}, grab end {}, close menu {}, touch down {}, long press {}", iters(), seen[0], seen[1], seen[2], seen[3], seen[4], seen[5], seen[6], seen[7]);
}
