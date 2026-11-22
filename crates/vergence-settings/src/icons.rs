//! The sidebar's page icons and the Advanced part's chevron.
//! The app's skeleton (the Program impl, the page list and routing, shared
//! state and helpers) is in main.rs.

use super::*;

/// A page's icon in the sidebar: colored rounded squares with a white mark, as macOS's, so a
/// page is found by color and shape at a glance (see docs/settings-milestone-1.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Icon {
    WiFi,
    Network,
    Appearance,
    Fonts,
    Wallpaper,
    Windows,
    Dock,
    Screen,
    Keyboard,
    Pointer,
    Touchpad,
    Touchscreen,
    Sound,
    Accessibility,
    Energy,
    Battery,
    About,
    Notifications,
    Screenshot,
    Apps,
    Clock,
    Lock,
}

impl Icon {
    /// Its square's color (one per kind of page, readable under a white mark).
    fn color(self) -> Color {
        match self {
            Icon::WiFi | Icon::Network => Color::rgb(0x1e, 0x88, 0xe5),
            Icon::Appearance | Icon::Fonts | Icon::Wallpaper => Color::rgb(0x2f, 0x6f, 0xd6),
            Icon::Windows | Icon::Dock | Icon::Apps => Color::rgb(0x5b, 0x5f, 0x6b),
            Icon::Screen => Color::rgb(0x1f, 0x8a, 0x8a),
            Icon::Keyboard | Icon::Pointer | Icon::Touchpad | Icon::Touchscreen => Color::rgb(0x6e, 0x55, 0xc9),
            Icon::Sound => Color::rgb(0xd2, 0x3f, 0x6a),
            Icon::Accessibility => Color::rgb(0x1f, 0x6f, 0xe0),
            Icon::Energy | Icon::Battery => Color::rgb(0x3f, 0x9a, 0x3a),
            Icon::About => Color::rgb(0x70, 0x74, 0x7c),
            Icon::Notifications => Color::rgb(0xe5, 0x48, 0x3a),
            Icon::Screenshot => Color::rgb(0x3a, 0x42, 0x50),
            Icon::Clock => Color::rgb(0x70, 0x74, 0x7c),
            Icon::Lock => Color::rgb(0x3d, 0x6b, 0xb8),
        }
    }
}

/// A page's icon: a rounded square of its color with a white mark drawn in it.
pub(crate) fn icon_mark<M: 'static>(icon: Icon) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, _: &Theme| {
        let s = 20.0f32;
        let (x, y) = (r.x + (r.w - s) / 2.0, r.y + (r.h - s) / 2.0);
        scene.fill_rounded(Rect::new(x, y, s, s), 5.0, icon.color());
        let w = Color::rgb(255, 255, 255);
        let p = |dx: f32, dy: f32| Point::new(x + dx, y + dy);
        let line = |scene: &mut Scene, a: (f32, f32), b: (f32, f32)| {
            scene.line(p(a.0, a.1), p(b.0, b.1), 1.6, w);
        };
        match icon {
            // Waves over a dot.
            Icon::WiFi => {
                scene.fill_rounded(Rect::new(x + 8.9, y + 13.2, 2.2, 2.2), 1.1, w);
                line(scene, (7.4, 11.4), (10.0, 9.6));
                line(scene, (10.0, 9.6), (12.6, 11.4));
                line(scene, (5.0, 9.0), (10.0, 5.8));
                line(scene, (10.0, 5.8), (15.0, 9.0));
            }
            // Three linked boxes.
            Icon::Network => {
                scene.stroke(Rect::new(x + 7.5, y + 4.0, 5.0, 4.0), 1.0, 1.3, w);
                scene.stroke(Rect::new(x + 3.5, y + 12.0, 5.0, 4.0), 1.0, 1.3, w);
                scene.stroke(Rect::new(x + 11.5, y + 12.0, 5.0, 4.0), 1.0, 1.3, w);
                line(scene, (10.0, 8.0), (10.0, 10.0));
                line(scene, (6.0, 10.0), (14.0, 10.0));
                line(scene, (6.0, 10.0), (6.0, 12.0));
                line(scene, (14.0, 10.0), (14.0, 12.0));
            }
            // Half light, half dark.
            Icon::Appearance => {
                scene.stroke(Rect::new(x + 5.0, y + 5.0, 10.0, 10.0), 5.0, 1.6, w);
                scene.fill(Rect::new(x + 10.0, y + 5.5, 4.4, 9.0), w);
            }
            // An A.
            Icon::Fonts => {
                line(scene, (6.0, 15.0), (10.0, 5.0));
                line(scene, (10.0, 5.0), (14.0, 15.0));
                line(scene, (7.6, 11.0), (12.4, 11.0));
            }
            // A picture: hills under a sun.
            Icon::Wallpaper => {
                scene.stroke(Rect::new(x + 4.0, y + 5.0, 12.0, 10.0), 1.5, 1.4, w);
                scene.fill_rounded(Rect::new(x + 11.6, y + 6.8, 2.4, 2.4), 1.2, w);
                line(scene, (5.0, 14.0), (8.5, 9.5));
                line(scene, (8.5, 9.5), (11.0, 12.5));
                line(scene, (10.5, 12.0), (12.5, 10.5));
                line(scene, (12.5, 10.5), (15.0, 13.5));
            }
            // A window with its bar.
            Icon::Windows => {
                scene.stroke(Rect::new(x + 4.5, y + 5.5, 11.0, 9.0), 1.5, 1.4, w);
                scene.fill(Rect::new(x + 4.5, y + 5.5, 11.0, 2.5), w);
            }
            // Four squares: apps.
            Icon::Apps => {
                for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    scene.fill_rounded(Rect::new(x + 5.0 + i * 5.5, y + 5.0 + j * 5.5, 4.5, 4.5), 1.2, w);
                }
            }
            // A dock of three.
            Icon::Dock => {
                scene.stroke(Rect::new(x + 4.0, y + 11.0, 12.0, 5.0), 2.0, 1.4, w);
                for i in 0..3 {
                    scene.fill_rounded(Rect::new(x + 5.5 + i as f32 * 3.5, y + 12.5, 2.0, 2.0), 1.0, w);
                }
            }
            // A screen on its stand.
            Icon::Screen => {
                scene.stroke(Rect::new(x + 4.0, y + 5.0, 12.0, 8.0), 1.5, 1.4, w);
                line(scene, (10.0, 13.0), (10.0, 15.5));
                line(scene, (7.0, 15.5), (13.0, 15.5));
            }
            // Keys.
            Icon::Keyboard => {
                scene.stroke(Rect::new(x + 3.5, y + 6.5, 13.0, 8.0), 1.5, 1.4, w);
                for (kx, ky) in [(6.0, 9.0), (9.0, 9.0), (12.0, 9.0), (7.0, 12.0)] {
                    scene.fill(Rect::new(x + kx, y + ky, 1.6, 1.4), w);
                }
                scene.fill(Rect::new(x + 9.0, y + 12.0, 4.6, 1.4), w);
            }
            // A mouse.
            Icon::Pointer => {
                scene.stroke(Rect::new(x + 6.5, y + 4.5, 7.0, 11.0), 3.5, 1.4, w);
                line(scene, (10.0, 4.8), (10.0, 8.5));
            }
            // A pad with its two buttons.
            Icon::Touchpad => {
                scene.stroke(Rect::new(x + 4.5, y + 5.0, 11.0, 10.0), 2.0, 1.4, w);
                line(scene, (4.8, 12.0), (15.2, 12.0));
                line(scene, (10.0, 12.0), (10.0, 14.8));
            }
            // A screen and a fingertip on it.
            Icon::Touchscreen => {
                scene.stroke(Rect::new(x + 4.0, y + 5.0, 12.0, 8.5), 1.5, 1.4, w);
                scene.fill_rounded(Rect::new(x + 9.0, y + 8.0, 3.0, 3.0), 1.5, w);
                line(scene, (8.0, 15.5), (12.0, 15.5));
            }
            // A speaker.
            Icon::Sound => {
                scene.fill(Rect::new(x + 5.0, y + 8.0, 3.0, 4.0), w);
                line(scene, (8.0, 8.0), (11.0, 5.5));
                line(scene, (11.0, 5.5), (11.0, 14.5));
                line(scene, (11.0, 14.5), (8.0, 12.0));
                line(scene, (13.5, 8.0), (14.5, 10.0));
                line(scene, (14.5, 10.0), (13.5, 12.0));
            }
            // A person.
            Icon::Accessibility => {
                scene.fill_rounded(Rect::new(x + 8.6, y + 3.8, 2.8, 2.8), 1.4, w);
                line(scene, (5.5, 8.0), (14.5, 8.0));
                line(scene, (10.0, 8.0), (10.0, 12.0));
                line(scene, (10.0, 12.0), (7.5, 16.0));
                line(scene, (10.0, 12.0), (12.5, 16.0));
            }
            // A bolt.
            Icon::Energy => {
                line(scene, (11.5, 4.0), (7.0, 10.5));
                line(scene, (7.0, 10.5), (13.0, 9.5));
                line(scene, (13.0, 9.5), (8.5, 16.0));
            }
            // A battery.
            Icon::Battery => {
                scene.stroke(Rect::new(x + 3.5, y + 6.5, 11.5, 7.0), 1.5, 1.3, w);
                scene.fill(Rect::new(x + 15.2, y + 8.6, 1.6, 2.8), w);
                scene.fill(Rect::new(x + 5.2, y + 8.2, 6.0, 3.6), w);
            }
            // A card with a dot at its corner.
            Icon::Notifications => {
                scene.stroke(Rect::new(x + 4.0, y + 6.0, 11.0, 9.5), 2.0, 1.4, w);
                scene.fill_rounded(Rect::new(x + 12.6, y + 3.4, 4.4, 4.4), 2.2, w);
                line(scene, (6.5, 9.5), (11.0, 9.5));
                line(scene, (6.5, 12.3), (12.5, 12.3));
            }
            // A camera in a selection's corners (the Screenshot app's).
            Icon::Screenshot => {
                line(scene, (3.5, 7.0), (3.5, 3.5));
                line(scene, (3.5, 3.5), (7.0, 3.5));
                line(scene, (13.0, 3.5), (16.5, 3.5));
                line(scene, (16.5, 3.5), (16.5, 7.0));
                line(scene, (16.5, 13.0), (16.5, 16.5));
                line(scene, (16.5, 16.5), (13.0, 16.5));
                line(scene, (7.0, 16.5), (3.5, 16.5));
                line(scene, (3.5, 16.5), (3.5, 13.0));
                scene.fill_rounded(Rect::new(x + 6.0, y + 7.5, 8.0, 6.0), 1.2, w);
                scene.fill_rounded(Rect::new(x + 8.4, y + 6.4, 3.2, 1.6), 0.6, w);
            }
            // A clock face with its hands.
            Icon::Clock => {
                scene.stroke(Rect::new(x + 3.5, y + 3.5, 13.0, 13.0), 6.5, 1.4, w);
                line(scene, (10.0, 10.0), (10.0, 6.0));
                line(scene, (10.0, 10.0), (13.0, 11.5));
            }
            // A padlock.
            Icon::Lock => {
                scene.stroke(Rect::new(x + 6.8, y + 3.8, 6.4, 8.0), 3.2, 1.4, w);
                scene.fill_rounded(Rect::new(x + 5.0, y + 9.0, 10.0, 7.5), 1.5, w);
            }
            // An i.
            Icon::About => {
                scene.fill_rounded(Rect::new(x + 9.1, y + 4.5, 1.8, 1.8), 0.9, w);
                line(scene, (10.0, 8.5), (10.0, 15.0));
            }
        }
    })
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(22.0))
}

/// The Advanced part's chevron: pointing right while closed, down when open.
pub(crate) fn chevron<M: 'static>(open: bool) -> Element<M> {
    drawing(move |scene: &mut Scene, _: &mut Fonts, r: Rect, t: &Theme| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let pts = if open { [(-3.5, -1.5), (0.0, 2.0), (3.5, -1.5)] } else { [(-1.5, -3.5), (2.0, 0.0), (-1.5, 3.5)] };
        let p = |(dx, dy): (f32, f32)| Point::new(cx + dx, cy + dy);
        scene.line(p(pts[0]), p(pts[1]), 1.6, t.dim);
        scene.line(p(pts[1]), p(pts[2]), 1.6, t.dim);
    })
    .width(Length::Fixed(14.0))
    .height(Length::Fixed(14.0))
}
