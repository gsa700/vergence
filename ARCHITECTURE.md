# Vergence architecture

Vergence is a desktop written in Rust. It has its own display server, its own
native protocol between that server and applications, its own toolkit, and a
set of applications built on that toolkit. FreeBSD is the main platform;
Void Linux is the supported Linux (see
[docs/release-pathway.md](docs/release-pathway.md)). Other Wayland
applications, and X11 applications through Xwayland, run beside the native
ones.

The design, and the reasons for it, are in
[docs/Vergence-Whitepaper.md](docs/Vergence-Whitepaper.md). Its three
contracts come up throughout this file:

1. **Geometry in visual size.** Sizes on the wire are in reference units
   (points at 96 to the inch). The server calculates each screen's scale from
   its EDID, and every native window is drawn at the exact scale of the
   screen it is on.
2. **Placement owned by the server.** Applications may ask for a place; the
   server decides and remembers it per monitor setup.
3. **Meaning, not just pixels.** Every native window publishes a tree of
   roles, names and values. Accessibility tools, the magnifier and software
   agents read it.

How to read this file: look at the diagram, then read the tour in order. Each
section names the files to open. The crate table at the end says what every
crate is for. Paths are relative to the repository root.

## The picture

```text
                 applications: one process each, run as the user
 .---------------------------. .-------------------. .--------------------.
 | toolkit apps              | | other Wayland     | | vctl (test         |
 | (vergence-ui, -widgets):  | | apps              | |   sessions only)   |
 | Terminal, Settings, Edit, | |                   | | vagent, vergence-  |
 | FileView, Reader, ...     | | X11 apps through  | |   agent-mcp        |
 |                           | | Xwayland (opt-in) | | vtile, vbus        |
 '-------------+-------------' '---------+---------' '---------+----------'
               | native protocol         | Wayland,            | control
               | (VERGENCE_BUS: bus,     | X11                 | socket,
               |  windows, agents)       |                     | bus
 .-------------v-------------------------v---------------------v----------.
 | vergence-server: one authority over the screens and the input          |
 |                                                                        |
 |  the bus: bus.rs, native.rs,      | the edges: handlers/ (Smithay's    |
 |   agents.rs, meaning.rs           |  Wayland front end), Xwayland,     |
 |  control.rs (vctl)                |  dbus.rs (D-Bus), portal.rs        |
 |  ---------------------------------+----------------------------------  |
 |  window space: win.rs place.rs memory.rs snap.rs style.rs focus.rs     |
 |  input: input.rs keys.rs dispatch.rs  -->  vergence-seat (decisions)   |
 |  its own parts: decor.rs (bars, from vergence-bar), shelf.rs (dock),   |
 |   launcher.rs, menu.rs, tiles.rs, notify.rs, lock.rs, greeter.rs       |
 |  drawing: scenes.rs (app scenes, vergence-render), backdrop.rs (frost) |
 |   composited on the GPU (Smithay's GLES renderer, our shaders)         |
 |  output: drm.rs (DRM/KMS, GBM, EGL, libinput, libseat); winit.rs when  |
 |   nested in another desktop; headless screens for tests                |
 '--------+---------------------------------------+-----------------------'
          | unlock, session,                      | started by the server
          | greeter sockets                       | and by apps
 .--------v----------.  .-------------------------v-----------------------.
 | vdm (root)        |  | confined helpers (libexec/vergence)             |
 | the only PAM      |  |  vergence-decode  vergence-pdf  vergence-hymie  |
 | caller; starts    |  |  vergence-hymie-aac  (Capsicum or seccomp)      |
 | sessions and the  |  | FreeBSD root helpers: media-eject,              |
 | login screen      |  |  sound-default                                  |
 '-------------------'  '-------------------------------------------------'
                 kernel: DRM/KMS, evdev, the GPU (EGL, GLES)
```

One server process owns every screen and every input device. Applications
reach it three ways: toolkit apps on the native protocol, everything else on
Wayland (or X11 through Xwayland). The session bus is the server itself, on
the same socket as native windows. `vdm` is a separate root daemon. The
helpers are short-lived processes that parse untrusted files and can do
nothing else.

## The tour

### How an app gets a window

A toolkit app opens its window through `vergence-ui`. In a Vergence session
the server exports `VERGENCE_BUS`, the path of its bus socket in
`$XDG_RUNTIME_DIR`. The toolkit connects, sends `ClientMsg::Hello` with the
protocol version and its application id, and then
`WinRequest::Create`: a role, a persistence token, a title, a size in
reference units, an optional parent and an optional place.

The server decides where the window goes: window memory for that identity
first, then the place the app asked for (if it fits wholly on a screen),
then the screen under the pointer. It answers `WinEvent::Created` and a
`WinEvent::Configure` with the size in reference units and the exact scale
of the screen. From then on the app sends pictures or scenes (next section)
and the server sends input, configures and frame callbacks.

The server draws every native window's bar, tabs included. The app describes
its tabs, buttons and commands (`WinRequest::Bar`, `Commands`, `Style`); the
server lays them out, hit-tests them and tells the app what was pressed.
This is why a window's name cannot be faked by the app.

If there is no Vergence server, or it refuses the window, the toolkit falls
back to Wayland with `vergence-wl`, its own Wayland client code. The program
cannot tell which road it is on. `VERGENCE_UI_WAYLAND=1` forces the Wayland
road for comparison.

Applications that are not ours speak Wayland to Smithay's front end, or X11
to Xwayland (off by default; `[x11] enabled = true` in desktop.toml turns it
on). The server wraps every kind of window in one type, `Win`, so placement,
focus, bars and memory treat them alike.

Files to open:
- [crates/vergence-protocol/src/window.rs](crates/vergence-protocol/src/window.rs):
  `WinRequest` and `WinEvent`, opcodes 100 to 199.
- [crates/vergence-server/src/native.rs](crates/vergence-server/src/native.rs):
  the server's side of native windows, placement and the clipboard.
- [crates/vergence-ui/src/native.rs](crates/vergence-ui/src/native.rs): the
  toolkit's native road; [crates/vergence-ui/src/lib.rs](crates/vergence-ui/src/lib.rs)
  holds the Wayland road and the `App` trait.
- [crates/vergence-server/src/win.rs](crates/vergence-server/src/win.rs),
  [place.rs](crates/vergence-server/src/place.rs),
  [memory.rs](crates/vergence-server/src/memory.rs),
  [style.rs](crates/vergence-server/src/style.rs): the window space.
- [crates/vergence-server/src/handlers/](crates/vergence-server/src/handlers/):
  the Wayland and X11 handlers.

### How a frame reaches the screen

1. **The app builds a scene.** A widget program (`vergence-widgets`) builds a
   tree of elements from its state, lays it out at the window's size and
   draws it into a `vergence_scene::Scene`: a display list of fills, lines,
   glyph runs, images, clips and terminal cell grids, in reference units,
   with no scale in it.
2. **The toolkit sends it.** On the native road the scene is encoded in
   vergence-scene's keyed wire form, written into a memfd and sent as
   `WinRequest::Scene`. Images inside it are kept by the server under keys,
   so a later scene can name them without sending them again.
3. **The server draws it.** `scenes.rs` validates the scene, estimates its
   cost and draws it with `vergence-render` on the GPU, at the scale of the
   screen the window is on, into one of three buffers the window takes in
   turn. A window that straddles two screens of different scales is drawn
   again for the other screen, so both halves are sharp. Where there is no
   GPU, the CPU reference renderer draws it.
4. **The server composites.** `decor.rs` builds each screen's list of
   elements front to back: windows, their bars and borders, the dock, menus
   and notifications. The bar is laid out and drawn from `vergence-bar`, the
   same code the toolkit uses under other desktops. The frost behind the dock
   and frosted windows is `backdrop.rs`, using vergence-render's blur shader.
   Final compositing uses Smithay's GLES renderer with our own shaders for
   rounded corners and frost.
5. **The screen shows it.** `drm.rs` drives DRM/KMS: one buffer per screen at
   its native resolution, page flips at that screen's pace. A screen on a
   second GPU is drawn by the primary GPU and copied across. After a screen
   is drawn the server sends `WinEvent::Frame` to the windows on it.

Scenes are untrusted input. A scene over the cost budget, or too slow to
draw, switches that window off scenes: the server sends `WinEvent::ScenesOff`
and the toolkit sends its own pixels from then on. Pixels (shared memory or
a GPU buffer) are the other way a window can be shown; `VERGENCE_UI_PIXELS=1`
keeps an app on them. Wayland and X11 apps always send pixels.

The server's own panels (the dock, bars, menus, the lock) are scenes too,
drawn by vergence-render and kept until something in them changes.

Files to open:
- [crates/vergence-scene/src/lib.rs](crates/vergence-scene/src/lib.rs) and
  [wire.rs](crates/vergence-scene/src/wire.rs): the display list and its
  validation.
- [crates/vergence-render/src/lib.rs](crates/vergence-render/src/lib.rs): the
  CPU renderer (`cpu.rs`) and the GPU renderer (`gpu.rs`, `gl.rs`), which
  must agree.
- [crates/vergence-server/src/scenes.rs](crates/vergence-server/src/scenes.rs),
  [decor.rs](crates/vergence-server/src/decor.rs),
  [backdrop.rs](crates/vergence-server/src/backdrop.rs),
  [drm.rs](crates/vergence-server/src/drm.rs),
  [gpus.rs](crates/vergence-server/src/gpus.rs).
- [crates/vergence-text/src/lib.rs](crates/vergence-text/src/lib.rs): fonts,
  fallback and the glyph cache, shared by the server and the toolkit.

### Where input goes

Devices come in through libinput and libseat (seatd on FreeBSD and Void;
logind where it runs). From there:

1. `input.rs` takes each event. Keys go through the server's own xkb state
   (`keys.rs`); touch is mapped to its panel's screen (`touch.rs`); the
   pointer is kept on the screens.
2. The server tells `vergence-seat` what is under the point: an open menu or
   the launcher first, then a notification card, then the dock and the
   windows front to back, else the bare desktop. The seat knows no Smithay
   types; windows are numbers to it.
3. The seat decides who gets the event: pointer focus and the implicit grab,
   moving and resizing, menus, the lock. For keys it filters the server's own
   first (Ctrl+Alt+Backspace, console switching, volume keys, Super alone,
   Alt+Tab, Super with an arrow), then the focused window has the rest.
4. `dispatch.rs` carries out the decision after the seat returns, never
   inside one of Smithay's callbacks. A native window hears the event as a
   `WinEvent` (`PointerMotion`, `PointerButton`, `Key`, `Touch`...) in its own
   reference units. A Wayland or X11 surface hears it through Smithay. The
   server's own parts (bars, the dock, menus, tiles, the launcher) act on it
   in the server.

Keyboard focus moves only on the user's action. `opened.rs` decides whether a
new window takes the keyboard; when a window closes, the keyboard goes back
to its parent or the window in front. While the screen is locked, every key
and press is the lock's.

Files to open:
- [crates/vergence-seat/src/lib.rs](crates/vergence-seat/src/lib.rs): the
  rules, each with tests.
- [crates/vergence-server/src/input.rs](crates/vergence-server/src/input.rs),
  [keys.rs](crates/vergence-server/src/keys.rs),
  [dispatch.rs](crates/vergence-server/src/dispatch.rs),
  [focus.rs](crates/vergence-server/src/focus.rs),
  [opened.rs](crates/vergence-server/src/opened.rs).

### The bus, the control socket and agents

**The session bus.** The server is the bus. It listens on
`$XDG_RUNTIME_DIR/vergence-bus-<wayland socket>` (mode 0600, the peer's uid
checked) and exports the path as `VERGENCE_BUS`. Frames are
`[u32 length][u16 opcode][payload]`, little-endian, at most 64 KiB; file
descriptors travel beside them. Clients subscribe to topics (`Outputs`,
`Settings`, `Inputs`, `Drives`) and get the current state, then changes. They
make requests: notifications, opening files with the right application,
live tiles on the dock, Force Quit's list, ejecting a drive, and more.

The server and the apps speak **one exact protocol version**,
`vergence_protocol::bus::VERSION`. The server refuses any other, so they are
built and installed together. Changing a message means raising the version.

**The D-Bus edge.** Programs that expect a D-Bus session bus get one from the
server, written by the project (`vergence-dbus` for the wire format,
`dbus.rs` for the bus). Well-known interfaces are translated into core
services (notifications, the desktop portal's file chooser and settings). A
name nobody owns gets "service unknown"; nothing is started on demand.

**The control socket.** `vctl` (in [tools/vctl](tools/vctl)) drives a running
server: screenshots, window lists, moving, typing, fake screens. It goes
around the security model, so it exists only when the server starts with
`VERGENCE_CONTROL=1`. A session started from a checkout has it; an installed
session does not.

**Agents (Contract 3).** Each native window sends its meaning tree
(`WinRequest::Meaning`) as the toolkit lays it out. `meaning.rs` keeps every
tree with its place on the screens and finds widgets by path
(`"Calculator > button:7"`). The agent interface (opcodes 200 to 299 in
`agent.rs`) lets an agent list windows, read trees and act on a widget by
name: the server sends the window `WinEvent::Act` and the toolkit performs
the action on that widget. No input is faked. The first time an agent asks
about an app, the server asks the person in a dialog it draws itself, and
grants are per agent (name and program) and per app. A secret field is a
role and a name, never a value. `vergence-agent-mcp` adapts this to the
Model Context Protocol on stdio, for any MCP agent.

Files to open:
- [crates/vergence-protocol/src/bus.rs](crates/vergence-protocol/src/bus.rs),
  [wire.rs](crates/vergence-protocol/src/wire.rs),
  [agent.rs](crates/vergence-protocol/src/agent.rs),
  [tile.rs](crates/vergence-protocol/src/tile.rs),
  [commands.rs](crates/vergence-protocol/src/commands.rs).
- [crates/vergence-server/src/bus.rs](crates/vergence-server/src/bus.rs),
  [dbus.rs](crates/vergence-server/src/dbus.rs),
  [control.rs](crates/vergence-server/src/control.rs),
  [meaning.rs](crates/vergence-server/src/meaning.rs),
  [agents.rs](crates/vergence-server/src/agents.rs).
- The small clients in
  [crates/vergence-protocol/src/bin/](crates/vergence-protocol/src/bin/):
  `vbus`, `vtile`, `vagent`, `vergence-agent-mcp`, `vergence-screenshot`.

### Untrusted files: decoding apart

Pictures, PDFs and sound files come from anywhere, so no app and not the
server parses them in their own process. Each kind has a helper program in
`libexec/vergence/`:

| Helper | Crate | Reads |
|---|---|---|
| `vergence-decode` | vergence-decode | PNG, JPEG, GIF, WebP, SVG, BMP |
| `vergence-pdf` | vergence-pdf (drawing with vergence-raster) | PDF documents |
| `vergence-hymie` | vergence-hymie | WAV, FLAC, MP3, ALAC, MP4, tags, cover art |
| `vergence-hymie-aac` | vergence-hymie-aac | AAC (GPL-2.0-or-later, kept apart) |

The caller opens the file and hands it to the helper as standard input, so
the helper needs no file system. Before it parses anything the helper calls
`vergence_decode::confine`: on FreeBSD it enters Capsicum capability mode;
on Linux it sets no-new-privileges and enters a seccomp filter. On both it
closes every other descriptor and lowers its limits on CPU time, memory and
open files. A crash or a hang in the helper is an error for the caller, not a
crash.

Two other helpers are small setuid programs on FreeBSD only: `media-eject`
(vergence-media) unmounts a removable drive, and `sound-default`
(vergence-sound) sets the default sound output. Each does one fixed thing.

Files to open:
[crates/vergence-decode/src/lib.rs](crates/vergence-decode/src/lib.rs)
(`confine`, `decode_file`),
[crates/vergence-pdf/src/helper.rs](crates/vergence-pdf/src/helper.rs),
[crates/vergence-hymie/src/helper.rs](crates/vergence-hymie/src/helper.rs).

### Login, lock and the session

`vdm` ([crates/vergence-dm/src/bin/vdm.rs](crates/vergence-dm/src/bin/vdm.rs))
is a root daemon started by the init system, and **the only program that
talks to PAM**. It learns who is asking from the kernel (the socket peer),
never from the request, and runs each check in a child process with a time
limit.

- **The login screen** is the same server run as `vergence-server --greeter`
  under an account that owns nothing. In greeter mode it starts no program
  and opens no bus, D-Bus edge or Xwayland. It draws the login dialog itself
  (`greeter.rs`) and passes the PAM conversation to vdm.
- **A session** runs on a virtual terminal of its own, through the user's
  login shell, as [tools/vergence-session](tools/vergence-session), which
  runs `vergence-server --drm` and starts it again after `vctl restart`. A
  session can also be started by hand from a text console with `vergence`.
- **The lock** is the server, not a separate program (`lock.rs`). It draws
  only the wallpaper and the lock while locked, keeps the typed password in
  locked, wiped memory (vergence-dm's `secret`), and asks vdm to check it.
  vdm locks every session before the machine sleeps.
- **Elevation:** vdm can carry out a short list of named actions for a
  program after the person approves them in a prompt the server draws
  (`elevate.rs` in both crates).

When the server starts as a session it brings back the applications that
were running (`session.rs`), and they put their windows and documents back.

### Where settings and state live

All of it is the user's alone: the server makes these folders 0700 at start.

- `~/.config/vergence/desktop.toml`: the desktop's settings, one table per
  area (`[appearance]`, `[shelf]`, `[windows]`, `[displays]`, `[lock]`,
  `[agents]`, `[x11]`...) and one per app (`[app."<id>"]`). The settings app
  edits it; the server rereads it within a second and pushes changes on the
  `Settings` topic. See
  [crates/vergence-server/src/settings.rs](crates/vergence-server/src/settings.rs).
- `~/.config/vergence/settings.toml`: theme, icons, cursor and fonts, also
  served to X11 apps as XSETTINGS.
- `~/.config/vergence/server-windows.toml`: window memory (Contract 2), per
  monitor setup and per resolution and scale of it.
- `~/.config/vergence/control-socket`: if this file exists, an installed
  session turns on the control socket.
- `~/.local/state/vergence/`: the server's log (`server.log`), the running
  apps for session restore (`session.toml`), monitors and input devices seen,
  agent grants and their log (`agents.tsv`, `agents.log`), dock and app-menu
  state, and each app's own state (the editor's unsaved work, the file
  dialog's recent places, Reader's places, and so on).
- `$XDG_RUNTIME_DIR`: the sockets (Wayland, bus, D-Bus, control). The server
  refuses a runtime directory that is a link, not its own, or open to others.

## What each crate is for

There are 40 crates in [crates/](crates/). Each has a doc comment at the top
of its entry file saying what it is for and where to start.

**The server and its libraries**

| Crate | What it is | Start at |
|---|---|---|
| vergence-server | The display server: screens, input, windows, the bus, the dock and everything it draws | `src/main.rs` |
| vergence-seat | Who gets each pointer, keyboard and touch event, as plain decisions | `src/lib.rs` |
| vergence-shell | The shell's logic with no drawing: dock geometry, applications from desktop entries, icons, snapping, named actions; `vergence-open` | `src/lib.rs` |
| vergence-dbus | Our own D-Bus wire protocol, for the session bus's edge and a small system-bus client | `src/lib.rs` |
| vergence-edid | EDID parser: identity, physical size, native mode | `src/lib.rs` |
| vergence-profile | Monitor-setup keys and scales calculated from EDIDs | `src/lib.rs` |
| vergence-xsettings | The XSETTINGS wire format, for X11 apps | `src/lib.rs` |

**The protocol**

| Crate | What it is | Start at |
|---|---|---|
| vergence-protocol | The native protocol: bus, windows, agents, tiles, commands, wire format; no dependencies. Also `vbus`, `vtile`, `vagent`, `vergence-agent-mcp`, `vergence-screenshot` | `src/lib.rs`, `src/bus.rs` |

**The toolkit** (also drawn with by the server)

| Crate | What it is | Start at |
|---|---|---|
| vergence-scene | The display list: a window as validated data in reference units | `src/lib.rs` |
| vergence-render | A scene into pixels: CPU reference renderer and GPU renderer; frost and corner shaders | `src/lib.rs` |
| vergence-text | Fonts, fallback per character, glyph positions, glyph cache | `src/lib.rs` |
| vergence-wl | Our own Wayland client code, for the toolkit's Wayland road | `src/lib.rs` |
| vergence-ui | A window, its input and its frames, on the native or the Wayland road | `src/lib.rs`, `src/native.rs` |
| vergence-widgets | Widgets, layout and meaning; text areas, the Open/Save dialog, file views, app settings windows | `src/lib.rs`, `src/ui.rs` |
| vergence-bar | The window bar: buttons, title or tabs; laid out, hit-tested and drawn for server and toolkit alike | `src/lib.rs` |
| vergence-look | The shared look: radii, sizes, colors, light and dark | `src/lib.rs` |

**Applications**

| Crate | What it is | Start at |
|---|---|---|
| vergence-term | Terminal: tabs in the bar, split panes | `src/main.rs` |
| vergence-settings | The settings app | `src/main.rs` |
| vergence-edit | Edit, the text editor | `src/main.rs` |
| vergence-files | FileView, the file manager; also the desktop's icons and Space preview | `src/main.rs` |
| vergence-closeup | Close-Up, the picture and text viewer | `src/main.rs` |
| vergence-reader | Reader, for PDF documents | `src/main.rs` |
| vergence-calc | Calculator with exact decimal arithmetic | `src/main.rs`, `src/lib.rs` |
| vergence-quit | Force Quit | `src/main.rs` |
| vergence-hw | Hardware: what the machine is and is doing | `src/main.rs` |

**Helpers and file formats**

| Crate | What it is | Start at |
|---|---|---|
| vergence-decode | Pictures decoded in a confined helper; `confine` for all helpers | `src/lib.rs` |
| vergence-pdf | Our own PDF reader and its confined helper | `src/lib.rs` |
| vergence-raster | Our own vector rasterizer, for PDF pages | `src/lib.rs` |
| vergence-hymie | Audio decoders, tags, peaks, and their confined helper | `src/lib.rs` |
| vergence-hymie-aac | AAC decoder and its own helper (GPL-2.0-or-later) | `src/lib.rs` |
| vergence-thumbs | Thumbnails by the freedesktop spec | `src/lib.rs` |
| vergence-types | File types by name and first bytes; which app opens each | `src/lib.rs` |
| vergence-spell | Spelling checker over the shipped word list | `src/lib.rs` |

**System interfaces**

| Crate | What it is | Start at |
|---|---|---|
| vergence-dm | VDM: the root daemon `vdm`, PAM, the unlock protocol, the password buffer | `src/lib.rs`, `src/bin/vdm.rs` |
| vergence-media | Removable drives and disk images: udisks2 on Linux, autofs on FreeBSD | `src/lib.rs` |
| vergence-sound | Sound outputs, volume, playback: OSS, ALSA, the PulseAudio protocol | `src/lib.rs` |
| vergence-net | Network interfaces and a wpa_supplicant client | `src/lib.rs` |
| vergence-power | Batteries, mains power, GPU power state | `src/lib.rs` |
| vergence-sysinfo | Processors, memory, disks, temperatures | `src/lib.rs` |

**Testing**

| Crate | What it is | Start at |
|---|---|---|
| vergence-harness | Test tool: EDID sweeps, fake monitors, picture checks for the display suite | `src/main.rs` |

## What is not ours

The server started from Smithay's example compositor and still uses Smithay
for its Wayland front end, DRM/KMS and libinput back ends, and final
compositing. Files marked "Based on Smithay's smallvil example" at the top
carry that code. The seat (input decisions) has already been replaced; the
window space and the renderer are next. Everything borrowed, and the plan for
each part, is in [docs/borrowed-review.md](docs/borrowed-review.md); licenses
are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Testing in brief

- `cargo test --workspace` runs the unit tests, including the server's pixel
  geometry at every clean scale
  ([crates/vergence-server/src/geometry_tests.rs](crates/vergence-server/src/geometry_tests.rs))
  and fuzz tests of the protocol, D-Bus and terminal parsers (`FUZZ_ITERS=<n>`
  for a longer run).
- The display suite, `sh tools/display-suite/run.sh [scenario...]`, runs each
  scenario in [tools/display-suite/](tools/display-suite/) in a headless
  server with screens in memory, and checks the pictures with
  `vergence-harness`. It builds into `target-check`, apart from your normal
  build.
- `sh tools/agent-check.sh` runs an MCP agent against Edit, Settings and
  FileView in a headless session.
- [tools/vergence-headless](tools/vergence-headless) starts a headless session
  by hand; `eval "$(tools/vergence-headless env)"` points `vctl` at it.

Building, running a nested session and the project's rules are in
CONTRIBUTING.md.
