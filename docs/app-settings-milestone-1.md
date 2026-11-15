# App settings milestone 1: an app's own settings, and its own look

Drafted 2026-11-14 from Dave's words of the afternoon ("settings per app
(terminal and settings itself right now), and the ability to choose per app
display settings ... dark terminal with light toolbar, light terminal with
dark titlebar, etc.") and docs/apps-roadmap.md item 2. **Draft: the
questions at the end are for Dave.**

## Where things stand

- **The rule** (whitepaper §6.10, decided 2026-11-10, not built): an app's
  own settings live in the app; each one with a desktop default inherits it
  until overridden, and says which it is. Window furniture "is the desktop's
  alone and has no per-application override".
- **Per-app tables exist already, by accident.** desktop.toml has
  `[terminal]` (`scheme`, `opacity`) and the toolkit's `opacity_setting`
  reads a program's own table before `[appearance] window-opacity`: absent
  means inherit, which is the model §6.10 wants. Nothing else uses it.
- **The terminal's settings are in the settings app**: Theme and fonts has a
  Terminal group with Colors (Catppuccin or Tango). Its font is the
  desktop's Monospace; scrollback is fixed at 10,000 lines (grid.rs).
- **Content style**: `Theme::from_desktop` takes `[appearance]
  color-scheme`; "dark"/"light" force every app, anything else ("App
  default") leaves the app's own choice.
- **Bar style**: the server draws the bar in the desktop's style, except a
  built-in list (decor.rs `LIGHT_BAR_APPS`, the settings app only) whose bar
  is light under "App default". That list is the stopgap this milestone
  replaces.
- No app has a preferences window; the dock's app menu has "Forget window
  places..." (settings milestone 2).

## Design

### Where an app's settings are kept

Each app's table in desktop.toml, named by its short name, as `[terminal]`
is today: `[terminal]`, `[settings]`. A key that is absent inherits; a key
that is present is an override. The app writes only its own table (the same
atomic write the settings app uses), and every program already re-reads
desktop.toml within a second, so a change shows at once in every window of
that app. The whitepaper's "settings service" (§6.8) stays the reader and
announcer for what the server needs; apps don't need a new bus call to keep
their own settings.

### The look, per app

Two settings every native app gets, both inheriting by default:

- **Style** (the content): Desktop default / Dark / Light. Key `style`.
- **Title bar**: Desktop default / Dark / Light. Key `bar-style`.

So a dark terminal with a light bar is `[terminal] style = "dark"`,
`bar-style = "light"`. The content style goes through
`Theme::from_desktop(text, default_dark, section)`, which reads the
section's `style` after `[appearance]`. The bar style goes to the server:
**bus protocol v10**, `WinRequest::BarStyle(Default | Dark | Light)`, sent
after Create and again whenever the setting changes; decor.rs `bar_light`
obeys it and `LIGHT_BAR_APPS` goes (the settings app sends Light as its
built-in default under "App default", so nothing changes for Dave unless he
chooses). The bar's layout, buttons, corners and font stay the desktop's.
This **changes §6.10**: the bar's light/dark becomes a per-app override;
the rest of the furniture stays desktop-wide.

Opacity joins them on the same page, since `[terminal] opacity` already
works: Desktop default / 100% ... 70%.

### The preferences window (toolkit)

vergence-widgets gets a `prefs` module so every app's settings look and
behave alike:

- A **preferences window**: the dialog kind (window styles milestone 1) but
  not modal, one per app, brought forward if already open; groups and rows
  as the settings app draws them, no sidebar while an app has one page.
- An **inherit-or-override row**: a choice whose first entry is
  "Desktop default (Dark)", naming what the default is now and following it
  live. Picking it removes the key. An overridden row shows a small
  "Use the desktop's" link beside it, the way back §6.10 asks for.
- The app hands it a list of rows (name, help line, key, choices, whether a
  desktop default exists); the toolkit reads and writes the app's table.

**Opening it**, three ways for every native app: **Ctrl+,** (as macOS and
GNOME apps), **"Preferences..."** in the dock's app menu (next to "Forget
window places...", the server sends the app `WinEvent::Preferences`), and
in the terminal's right-click menu.

### The terminal's preferences

| Group | Row | Default (inherits from) |
|---|---|---|
| Look | Style | desktop's Style |
| | Title bar | desktop's Style |
| | Opacity | desktop's window opacity |
| Text | Font and size | desktop's Monospace |
| | Colors | Catppuccin (moved from Theme and fonts) |
| Scrolling | Lines kept | 10,000 (1,000 / 10,000 / 100,000 / unlimited to a cap) |
| Cursor | Shape | Block (bar, underline) |
| | Blink | On |
| Closing | Ask before closing | On (several tabs or a program running) |

The Terminal group leaves the settings app's Theme and fonts page, with a
line in its place: "The terminal's colors and font are in the terminal's
preferences (Ctrl+,)".

### The settings app's preferences

Look only: Style, Title bar (light by default, as now), Opacity.

## Steps

1. **Toolkit**: section `style` in `Theme::from_desktop`; app table read and
   write in vergence-ui; the `prefs` window and the inherit-or-override row,
   with tests (absent key = default, override, back to default, live
   default change).
2. **Bus v10 bar style**: `WinRequest::BarStyle`, decor.rs obeys it,
   `LIGHT_BAR_APPS` removed; vergence-ui sends it from `[app] bar-style`.
   Server and apps installed together (exact version rule).
3. **Opening**: Ctrl+, in toolkit apps, dock menu "Preferences..." and
   `WinEvent::Preferences`.
4. **Terminal preferences**: the table above; scrollback, cursor and the
   close question become settings; Colors moves out of the settings app.
5. **Settings app preferences.**
6. **Docs**: whitepaper §6.10 (the bar exception, the window as built),
   §9 status, the Map.

## Acceptance

- Dark terminal with a light bar, and light terminal with a dark bar, side
  by side with the desktop on App default and on Dark.
- A row back on "Desktop default" follows a desktop change live (change the
  desktop's Monospace: a terminal on default follows, one with its own font
  does not).
- Preferences opens by Ctrl+, by the dock menu and by the terminal's menu,
  and a second open brings the same window forward.
- Hubble, kepler and Techbench full test suites; installs on all three.

## Questions for Dave

1. **Who wins when the desktop's Style is forced?** With desktop Style
   "Dark", does a terminal set to Light stay light? (a) Yes: an app's
   override is the user's later, narrower word (recommended). (b) No: the
   desktop's Dark/Light forces every app, overrides apply only under "App
   default".
2. **Where it opens**: Ctrl+, plus the dock menu plus the terminal's
   right-click menu. (a) All three (recommended). (b) Something else.
3. **Terminal scope**: (a) the whole table above, cursor and scrollback
   included (recommended: each is small). (b) Look and Colors only for now.
4. **The terminal's Colors in the settings app**: (a) move it into the
   terminal, leave a pointer line (recommended, §6.10). (b) Keep it in both.
