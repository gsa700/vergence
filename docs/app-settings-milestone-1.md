# App settings milestone 1: the app button, commands by role, one settings window per app

**Closed 2026-11-14** (built; see "As built").

Drafted 2026-11-14 from Dave's words of the afternoon ("settings per app
(terminal and settings itself right now), and the ability to choose per app
display settings ... dark terminal with light toolbar, light terminal with
dark titlebar, etc.") and docs/apps-roadmap.md item 2, then reshaped the
same evening by a discussion of where an app's settings and menus belong.
Mockup: the "Vergence app button" canvas (claude.ai artifact
9ZQQRsjNcDuRuP76iaPMTo), drawn from a kepler screenshot.

**Built 2026-11-14**, the same evening, on Hubble with Dave trying each
step as it landed; the design changed several times on the way (below,
"As built"), and the sections after that are the plan as accepted, kept
for the record. Commits 586871f to 7c2ceed; tests on Hubble, kepler and
Techbench for each.

**Accepted 2026-11-14** (Dave): "1 yes" (an app's own look wins over the
desktop's forced Dark/Light), "3 a" (the terminal's whole list, cursor and
scrollback included), "4 per app settings in prefs but defaults to system
wide setting" (the terminal's colors are a per-app row whose default is a
desktop-wide setting kept in the settings app), and on where it all lives:
"your proposal is the best I have seen, lets try it on terminal and
settings and see". He had thought of a menu next to the new-tab button, as
Windows Terminal has, but that "wouldn't work in other apps".

## As built (2026-11-14)

What changed from the plan, in the order Dave decided it:

1. **The menu's shape**, tried four ways in an evening. The skeleton with
   File/Edit/View/Tab submenus felt busy; a flat list for small apps was
   close to the everyday few; GNOME Terminal's menu ("click opens basic
   stuff right there, and then there is the option for everything ...
   maybe even an option to add things you use a lot") gave the everyday
   few plus All commands and pins; and last, Dave: the pane's and tab's
   commands were redundant there ("the split and move to tab and such
   really only works from the tab itself or within a split window"). The
   rule that came out of it, for every app: **the app button is the app, a
   right click is the thing under the pointer, the title bar is the
   window.** The menu: the app's featured commands and the user's pins,
   then Settings…, Keyboard shortcuts › (every command with its keys, a
   right click adding one to the menu), About, Quit. The terminal: New
   window, New tab, and a **Text size row** (− 100% +, staying open; a
   while's size, the lasting one in its settings; Dave's idea).
2. **No Window submenu** (Dave): layouts are on the title bar, the
   maximize button and Super with the arrows.
3. **Search field** only for apps with more than 24 commands; typing
   searches in any menu.
4. **Light and dark: the desktop's Style is the master switch** (Dave, at
   22:09, replacing his "1 yes" of 20:30): App default lets each app look
   as it chooses (its own choice, else its built-in look); Dark or Light
   forces every app. The rows that would not take effect are locked and
   say why.
5. **The settings app is light always** (Dave: "so hard to read when dark
   with the small text"), the one exception; its own window keeps Opacity
   and window places.
6. **Snapping on by default** (windows milestone 1's default reversed,
   Dave: "You did such a nice job it changed my mind"), and with the Ctrl
   setting a faint preview saying to hold Ctrl.
7. **Scroll bars start below the title bar** (Dave's note for the next
   settings work), done here: vergence-widgets `Ui::under_bar`.

Built as planned otherwise: the app button opposite the controls (layout
item `appmenu`, forced in), Alt+Space, commands with roles from one table
the server enforces, terminals' Shift rule, bus v10/v11, the toolkit's
settings window with inherit-or-override rows (the app's desktop.toml
table edited line by line), the terminal's rows (font, text size, colors
defaulting to the desktop's new `[appearance] terminal-colors`, lines
kept, cursor shape, blink, ask before closing), the Apps page and
`vergence-settings --app <id>`, foreign apps' `[app."<id>"] bar-style`
read by the server.

Known limits: Firefox and most GTK apps on Wayland draw their own bars,
so they get no app button (Firefox does with its "Title Bar" option on);
a foreign app's content light or dark is not the desktop's to choose; the
dock's and the Apps page's Forget… open a second settings window when one
is open (no single-instance hand-off yet).

### Acceptance

- [x] The button at the far right with controls left; at the far left
  reversed (vergence-bar tests); always there (forced in).
- [x] Dark terminal with a light bar and the reverse (Hubble, 20:55).
- [x] The settings window opens by Ctrl+, and by the menu; a second
  open does nothing while one is open.
- [x] A row on its default follows the desktop (the window re-reads
  desktop.toml each second).
- [x] Firefox's page in Settings (`--app firefox`); its bar style only
  with its own Title Bar option on (not tried live).
- [x] Full suites: Hubble 506, kepler 502, Techbench 506 (at f598ead).

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
