// Slint spike: title bar with tabs, two panes of colored monospace text, a
// divider, a right-click menu.

slint::slint! {
    import { Palette } from "std-widgets.slint";
    struct Run { text: string, color: color }
    struct Line { runs: [Run] }

    component Pane inherits Rectangle {
        in property <[Line]> lines;
        background: #1e1e2e;
        clip: true;
        VerticalLayout {
            padding: 8px;
            alignment: start;
            for line in root.lines: HorizontalLayout {
                alignment: start;
                for r in line.runs: Text { text: r.text; color: r.color; font-family: "Monospace"; font-size: 13px; }
            }
        }
    }

    component Tab inherits Rectangle {
        in property <string> title;
        in property <bool> active;
        callback clicked;
        width: 160px;
        border-radius: 6px;
        background: root.active ? #3b3b4f : (ta.has-hover ? #2e2e3e : transparent);
        Text { text: root.title; color: #e6e6f0; font-size: 12px; overflow: elide; x: 10px; width: parent.width - 20px; vertical-alignment: center; height: parent.height; }
        ta := TouchArea { clicked => { root.clicked(); } }
    }

    export component Spike inherits Window {
        title: "Slint spike";
        no-frame: true;
        preferred-width: 900px;
        preferred-height: 560px;
        background: #1e1e2e;
        in property <[Line]> left;
        in property <[Line]> right;
        in-out property <int> tab: 0;
        in-out property <float> split: 0.5;
        callback drag-window();
        callback close();
        VerticalLayout {
            // The title bar: tabs, +, and the window buttons; drag it to move.
            Rectangle {
                height: 36px;
                background: #181825;
                TouchArea {
                    pointer-event(e) => { if (e.kind == PointerEventKind.down && e.button == PointerEventButton.left) { root.drag-window(); } }
                }
                HorizontalLayout {
                    padding: 4px;
                    spacing: 4px;
                    Tab { title: "~/src/vergence"; active: root.tab == 0; clicked => { root.tab = 0; } }
                    Tab { title: "htop"; active: root.tab == 1; clicked => { root.tab = 1; } }
                    Rectangle { width: 28px; border-radius: 6px; background: plus.has-hover ? #2e2e3e : transparent;
                        Text { text: "+"; color: #e6e6f0; font-size: 16px; }
                        plus := TouchArea {} }
                    Rectangle { horizontal-stretch: 1; }
                    Rectangle { width: 28px; border-radius: 6px; background: cl.has-hover ? #c0392b : transparent;
                        Text { text: "\u{2715}"; color: #e6e6f0; }
                        cl := TouchArea { clicked => { root.close(); } } }
                }
            }
            // Two panes and a divider.
            body := Rectangle {
                vertical-stretch: 1;
                Pane { x: 0; width: body.width * root.split - 2px; height: parent.height; lines: root.left; }
                Rectangle {
                    x: body.width * root.split - 2px; width: 4px; height: parent.height;
                    background: dv.has-hover || dv.pressed ? #89b4fa : #313244;
                    dv := TouchArea {
                        mouse-cursor: col-resize;
                        moved => { root.split = Math.clamp((parent.x + self.mouse-x) / body.width, 0.15, 0.85); }
                    }
                }
                Pane { x: body.width * root.split + 2px; width: body.width * (1 - root.split) - 2px; height: parent.height; lines: root.right; }
                ContextMenuArea {
                    Menu {
                        MenuItem { title: "Copy"; }
                        MenuItem { title: "Paste"; }
                        MenuSeparator {}
                        MenuItem { title: "Split side by side"; }
                        MenuItem { title: "Split top and bottom"; }
                        MenuSeparator {}
                        MenuItem { title: "Close pane"; }
                    }
                }
            }
        }
    }
}

fn session() -> Vec<Line> {
    let (g, b, y, w, m) = (
        slint::Color::from_rgb_u8(0xa6, 0xe3, 0xa1),
        slint::Color::from_rgb_u8(0x89, 0xb4, 0xfa),
        slint::Color::from_rgb_u8(0xf9, 0xe2, 0xaf),
        slint::Color::from_rgb_u8(0xcd, 0xd6, 0xf4),
        slint::Color::from_rgb_u8(0xcb, 0xa6, 0xf7),
    );
    let run = |t: &str, c| Run { text: t.into(), color: c };
    let line = |runs: Vec<Run>| Line { runs: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(runs))) };
    let mut out = vec![line(vec![run("derickson", g), run("@", w), run("hubble", g), run(" ~/src/vergence $ ", b), run("ls -l crates", w)])];
    for (n, c) in [("vergd", b), ("vergence-dbus", b), ("vergence-edid", b), ("vergence-protocol", b), ("vergence-server", b), ("vergence-settings", b)] {
        out.push(line(vec![run("drwxr-xr-x  4 derickson derickson  6 Nov  8 13:40 ", w), run(n, c)]));
    }
    out.push(line(vec![run("derickson", g), run("@", w), run("hubble", g), run(" ~/src/vergence $ ", b), run("cargo build --release", w)]));
    for i in 0..14 {
        out.push(line(vec![run("   Compiling ", g), run(&format!("crate-number-{i} v0.{i}.0"), w)]));
    }
    out.push(line(vec![run("    Finished ", g), run("`release` profile [optimized] target(s) in 41.02s", w)]));
    out.push(line(vec![run("warning: ", y), run("unused variable `x`", w), run("  --> src/main.rs:12:9", m)]));
    out
}

fn main() -> Result<(), slint::PlatformError> {
    let t0 = std::time::Instant::now();
    let ui = Spike::new()?;
    let model = |v: Vec<Line>| slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(v)));
    ui.set_left(model(session()));
    ui.set_right(model(session()));
    let weak = ui.as_weak();
    ui.on_drag_window(move || {
        use slint::winit_030::WinitWindowAccessor;
        if let Some(ui) = weak.upgrade() {
            ui.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });
    ui.on_close(|| {
        let _ = slint::quit_event_loop();
    });
    eprintln!("slint-spike: window built in {} ms", t0.elapsed().as_millis());
    ui.run()
}
