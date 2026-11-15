# App settings milestone 1: the app button, commands by role, one settings window per app

Drafted 2026-11-14 from Dave's words of the afternoon ("settings per app
(terminal and settings itself right now), and the ability to choose per app
display settings ... dark terminal with light toolbar, light terminal with
dark titlebar, etc.") and docs/apps-roadmap.md item 2, then reshaped the
same evening by a discussion of where an app's settings and menus belong.
Mockup: the "Vergence app button" canvas (claude.ai artifact
9ZQQRsjNcDuRuP76iaPMTo), drawn from a kepler screenshot.

**Accepted 2026-11-14** (Dave): "1 yes" (an app's own look wins over the
desktop's forced Dark/Light), "3 a" (the terminal's whole list, cursor and
scrollback included), "4 per app settings in prefs but defaults to system
wide setting" (the terminal's colors are a per-app row whose default is a
desktop-wide setting kept in the settings app), and on where it all lives:
"your proposal is the best I have seen, lets try it on terminal and
settings and see". He had thought of a menu next to the new-tab button, as
Windows Terminal has, but that "wouldn't work in other apps".

## Why (Dave, 2026-11-14)

Windows 95 onward, nearly every app had a menu of the same layout and the
same keys. Then apps (Firefox, Office and others) moved to hamburger menus,
each its own, and the uniformity went. macOS keeps it better with the top
bar, but apps it does not control still do their own thing (Firefox has
both its menus and a hamburger; Windows apps hide menus under Alt), and
GNOME and KDE make it worse. Vergence draws every window's bar, its own
apps' and everyone else's, which none of those had: it can give every
window one uniform entry point.

## Where things stand

- **The rule** (whitepaper §6.10, decided 2026-11-10, not built): an app's
  own settings live in the app; each one with a desktop default inherits it
  until overridden, and says which it is. Window furniture "is the desktop's
  alone and has no per-application override". The desktop paradigm
  (2026-11-09): menus stay in each window, no global top bar.
- **Per-app tables exist already, by accident**: desktop.toml `[terminal]`
  (`scheme`, `opacity`); the toolkit's `opacity_setting` reads a program's
  own table before `[appearance] window-opacity`. Absent means inherit.
- **The terminal's colors** are on the settings app's Theme and fonts page;
  its font is the desktop's Monospace; scrollback is fixed at 10,000 lines.
- **Bar style**: the desktop's, except a built-in list (decor.rs
  `LIGHT_BAR_APPS`, the settings app) light under "App default".
- **Bar layout**: `[wm] button-layout`, GNOME's syntax
  (`close,minimize,maximize:`), parsed by vergence-bar
  `decoration_layout`; unknown items are ignored.
- **Menus the server draws**: the window's layouts menu (menu.rs), the
  dock's app menu (shelf.rs, with "Forget window places...").

## Design

### The app button

One button in every window's bar, drawn by the server, for every app,
native or not: the app's icon (the one the dock shows) and a small chevron.
It is an item of the bar layout, `appmenu`, which GNOME's layout syntax
already has, so it goes wherever the user's layout puts it. The default
puts it at the **end opposite the window controls**:
`close,minimize,maximize:appmenu`; reversed controls are
`appmenu:minimize,maximize,close`, which is Windows 95's arrangement
(system menu left, controls right). A layout without `appmenu` gets it at
the far end anyway: the button is not optional, or the uniformity is gone.
Under GNOME, our toolkit's own bar follows GNOME's layout, `appmenu`
included. With many tabs the tabs shrink; the button keeps its place.
Its key is **Alt+Space** (Windows' system menu key; Alt+Space is free in
the server today); the menu opens toward the middle of the window.

### The app menu: a fixed skeleton the app fills in

Top to bottom, a part shown only when it has something in it:

1. **Search commands…**: typing filters every command of the app,
   submenus included, each shown with its keys.
2. **About <App>**, **Settings…** (Ctrl+,).
3. The app's menus, in this order: **File, Edit, View, Tab**, then the
   app's own (by name).
4. **Window ›**: the server's (the layouts menu as a submenu, move to
   screen, minimize).
5. **Quit <App>** (Ctrl+Q).

A native app does not send "a menu": it sends **commands**, each with a
**role** (Copy, Paste, Select all, Find, New window, New tab, Close tab,
Close window, Zoom in/out/reset, Settings, About, Quit ...) or, for its own
commands, a menu name and a label. The toolkit puts a role in its place and
gives it its key, the same in every app; an app cannot move Copy or give it
another key. Its own commands may ask for a key that no role holds.

Bus protocol v10:
- `WinRequest::Commands(Vec<Command>)` (id, role or menu+label, keys,
  enabled, checked), sent after Create and whenever they change.
- `WinEvent::Command(id)`; `WinEvent::Settings`, `WinEvent::About`,
  `WinEvent::Quit` for the skeleton's own lines.
- `WinRequest::BarStyle(Default | Dark | Light)`.

**A foreign app** (Firefox, GTK, X11) gets the skeleton without its
middle: About (the name, icon and version its .desktop file gives),
Settings… (the desktop's part, below), a line "Firefox keeps its own
settings and commands in its own menu", Window, Quit (closes its windows
politely). Reading GTK's or Firefox's exported menus into the middle is
later and a borrowed protocol: Dave's call then.

### One settings window per app

**Settings…** opens one window per app (the dialog kind, not modal;
brought forward if open) with two parts:

- **<App>**: what only the app knows. The terminal: Font (default: the
  desktop's Monospace), Colors (default: the desktop's terminal colors),
  Lines kept, Cursor shape, Cursor blinks, Ask before closing.
- **On this desktop**: what the desktop does with the app: **Style**
  (content: Desktop default / Dark / Light), **Title bar** (Desktop
  default / Dark / Light), Opacity, Notifications, Window places
  (Forget…), later which GPU.

Each row with a default is an **inherit-or-override row**: its first choice
is "Desktop default (Dark)", naming the default now and following it live;
choosing it removes the override; an overridden row shows "Use the
desktop's". An app's own look wins over the desktop's forced Dark or Light
(Dave's "1 yes"): the desktop's Style is the default, an app's choice is
the later, narrower word.

**Where it is kept**: the app's table in desktop.toml, `[terminal]`,
`[settings]`, and for an app that is not ours `[app."org.mozilla.firefox"]`
by its app id; absent keys inherit. Keys: `style`, `bar-style`, `opacity`,
and the app's own (`font`, `scheme`, `scrollback`, `cursor`, `blink`,
`ask-close`). Every program re-reads desktop.toml within a second, so a
change shows at once. Notifications stay in notify-apps.toml and window
places in the server's memory; the window only shows them.

**Who draws it**: for our apps, the toolkit (vergence-widgets `prefs`), in
the app's own process; the "On this desktop" part only in a Vergence
session. For a foreign app, the settings app draws the same window, with
only the desktop's part (`vergence-settings --app <app-id>`). The settings
app gets an **Apps** page (Desktop group) listing every app the desktop
has seen, each opening that app's window: ours ask the app to open its own
(the server sends `WinEvent::Settings`), others open the settings app's.

**Desktop-wide defaults stay in the settings app**: Style, Monospace font,
window opacity, and the terminal colors (Theme and fonts keeps its
Terminal group, now saying it is the default and that the terminal can
choose its own).

### What moves

- The dock's "Forget window places..." becomes Settings… (the window's
  "On this desktop" part); the dock menu gets "Settings…" in its place.
- The Notifications page's per-app switches stay, and the same switch is in
  each app's window.
- `LIGHT_BAR_APPS` goes: the settings app sends `BarStyle(Light)` as its
  built-in default under App default, so nothing changes until chosen.
- The terminal's right-click menu stays (copy, paste, split: quick, under
  the hand), built from the same commands.

## Steps

1. **Protocol v10 and the button**: `appmenu` in vergence-bar's layout
   (default `close,minimize,maximize:appmenu`, forced in if absent),
   `Part::AppMenu`, the icon drawn from the dock's icon cache; `Commands`,
   `Command`, `Settings`, `About`, `Quit`, `BarStyle` on the bus.
2. **The server's app menu**: the skeleton (menu.rs), search, the Window
   submenu, Alt+Space; foreign apps' About, Quit, Settings… (launches
   `vergence-settings --app`); `bar_light` from `BarStyle`,
   `LIGHT_BAR_APPS` removed.
3. **Toolkit commands by role**: vergence-ui `Command`/`Role`, the role
   table (place and keys), keys handled in the toolkit, sent to the server;
   Ctrl+, opens settings.
4. **Toolkit settings window**: vergence-widgets `prefs` (inherit-or-
   override rows, the two parts), app table read/write in vergence-ui,
   `style` read by `Theme::from_desktop`, `bar-style` sent as `BarStyle`.
5. **The terminal**: its commands (Edit, View, Tab), its settings
   (font, colors, scrollback, cursor, blink, ask-close; grid and draw code
   take them live).
6. **The settings app**: its commands and settings window; the Apps page;
   `--app <id>` for foreign apps; the dock menu's Settings….
7. **Docs**: whitepaper §6.10 (the app button, commands by role, the bar's
   light/dark as the one per-app furniture choice), §6.9, §9, the Map.

## Acceptance

- Terminal and settings app on Hubble: the button at the far right with
  controls left, at the far left with the layout reversed (Windows
  arrangement), and still there with a layout that omits it.
- Dark terminal with a light bar, light terminal with a dark bar, with the
  desktop on App default and on Dark.
- Ctrl+C/V and the other roles do the same in both apps; search finds
  "Split" in the terminal and shows its keys.
- Firefox's app button: About, Settings… (desktop part), Window, Quit.
- A row back on "Desktop default" follows a desktop change live.
- Hubble, kepler and Techbench full test suites; installs on all three.
