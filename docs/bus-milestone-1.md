# Bus milestone 1: the session bus core (whitepaper §6.8)

Started 2026-11-08 (Dave: "start on the session bus"). The server is the bus:
session services are part of Vergence's native protocol, and a D-Bus front
end written by the project sits at the edge. All our own code (whitepaper §8,
"Our own parts"): the protocol crate has no dependencies.

## Steps

1. **Core bus** (this step, done 2026-11-08): `crates/vergence-protocol`
   (wire format, bus messages, a blocking client, the `vbus` tool) and
   `crates/vergence-server/src/bus.rs` (the server side). One topic, the
   monitors.
2. **D-Bus edge**: our own D-Bus wire protocol on the session bus address;
   connection basics (Hello, RequestName, AddMatch, GetNameOwner), routing
   for names an app owns, "service unknown" for the rest, no activation.
   First measure (whitepaper §10, question 13): log which interfaces
   Firefox, Thunderbird, GTK, Qt and Avalonia apps call.
3. **Notifications**: `org.freedesktop.Notifications` on the edge, a
   notifications service in the core, drawn by the server.
4. **The session uses it**: `DBUS_SESSION_BUS_ADDRESS` points at the edge
   (tools/vergence-session).
5. Then **settings** on the bus (live apply for the settings app, the
   server's title font and cursor, XSETTINGS republished).

## Design (step 1)

- **Socket**: `$XDG_RUNTIME_DIR/vergence-bus-<wayland socket>`, mode 0600,
  peer uid checked (getpeereid) against the server's. Named after the
  Wayland socket so a nested server never collides with the session's. The
  server exports the full path as `VERGENCE_BUS` to everything it starts.
  A socket a live server answers on is never replaced (the control socket
  now checks this too: a nested server used to delete the session's).
- **Frames**: `[u32 length][u16 opcode][payload]`, little-endian, at most
  64 KiB. Fields in order, no tags; strings and lists length-prefixed.
  Trailing bytes are ignored, so later versions add fields at the end.
- **Conversation**: Hello (version, app id) first, answered by Welcome
  (client id). Anything else first: NoHello error, disconnected. Requests
  carry a serial; replies echo it; events have serial 0. Unknown opcodes
  and topics get an error with the request's serial; the connection stays.
  A frame that breaks the format ends the connection.
- **Version 1 messages**: Ping/Pong, Subscribe/Unsubscribe/Done,
  GetOutputs, Error, Outputs. Subscribing sends Done, then the topic's
  current state at once.
- **Outputs topic**: every monitor's connector, EDID label, logical
  position and size, and scale. Sent to subscribers whenever the layout
  changes (hot-plug, nested window resize), and only if it changed.
- **Flow control**: replies are written at once; what the socket won't take
  waits in the connection's outbox behind a write source that exists only
  while the outbox is non-empty. A client that leaves more than 1 MiB
  unread is disconnected.
- **Identity**: the app id from Hello is recorded, not trusted (§7.4). The
  uid is checked. The peer's pid (FreeBSD LOCAL_PEERCRED, Linux SO_PEERCRED)
  is for a later step, with permissions.

## Acceptance (step 1)

1. `cargo test -p vergence-protocol`: every message round-trips; frames
   split at every byte reassemble; oversized, undersized and lying counts
   are rejected; unknown requests keep their serial; trailing fields are
   ignored. **Passed** on Hubble (10 tests).
2. `vergence-server` builds with no warnings. **Passed** (the only warning
   is the vendored drm-rs's, as before).
3. Nested on Hubble: `vbus ping` round trip (0.12 ms), `vbus outputs` lists
   both virtual outputs with their scales, `vbus watch outputs` gets the
   state on subscribe, a frame without Hello gets NoHello and is cut off.
   The nested server left the session's control socket alone. **Passed**
   2026-11-08.
4. On the real monitors (`--drm`): `vbus watch outputs` reports an unplug
   and replug. **Passed** 2026-11-08 on Hubble's live session: the ASUS
   unplugged gave one event with the 14" alone, replugged one event with
   both (the second ~1 s after the plug, when the server sets the monitor
   up). One event per change: the server calls in from both the removal
   and the layout, and the unchanged-list check sends it once.

## Step 2a: a logging D-Bus edge (built 2026-11-08)

`crates/vergence-dbus` (our own D-Bus wire protocol from the specification,
no dependencies: SASL EXTERNAL, values in both byte orders, messages, match
rules; 14 tests) and `crates/vergence-server/src/dbus.rs`:

- Socket `$XDG_RUNTIME_DIR/vergence-dbus-<wayland socket>`, 0600, peer uid
  checked, the client's claimed uid checked against it. No fd passing yet
  (NEGOTIATE_UNIX_FD answered ERROR).
- The server sets `DBUS_SESSION_BUS_ADDRESS` to it when none is set
  (`VERGENCE_DBUS=1` always, `=0` turns the edge off).
- org.freedesktop.DBus: Hello, RequestName (queueing, replacement),
  ReleaseName, GetNameOwner, NameHasOwner, ListNames, ListActivatableNames,
  ListQueuedOwners, AddMatch/RemoveMatch, GetConnectionUnixUser/
  UnixProcessID/Credentials, GetId, StartServiceByName (never starts
  anything), UpdateActivationEnvironment (ignored), Peer, Introspectable,
  Properties. Signals NameOwnerChanged, NameAcquired, NameLost.
- Routing: method calls and replies to names a connection owns; signals by
  match rule only (no eavesdropping); calls to names nobody owns get
  ServiceUnknown ("not provided by this session"), never an activation.
- Every call is counted per destination and method (`vctl dbus`), and
  logged the first time with the calling program (pid from LOCAL_PEERCRED).

Tested nested on Hubble 2026-11-08: dbus-send (libdbus) and gdbus (GLib)
ListNames, GetId, GetNameOwner; notify-send gets ServiceUnknown; a second
xfce4-terminal's Launch call reached the first instance, which owned
org.xfce.Terminal5 (routing works). First calls seen from xfce4-terminal:
org.gtk.vfs.Daemon (gvfs), org.a11y.Bus.GetAddress (AT-SPI), org.xfce.Xfconf
(~150 property reads, Xfce's settings daemon).

Next (2b): run the live session on it with the everyday apps (Firefox,
Thunderbird, AlbumWall, the settings app) for a while, then read `vctl dbus`
to choose what the edge translates first.

## Step 2b: what the everyday apps asked for (2026-11-08)

Live session, ~4 minutes with the settings app, Firefox and AlbumWall
(`vctl dbus`). Unanswered calls by count: org.gnome.Mutter.IdleMonitor
GetIdletime 14 (Firefox polls idle time), org.gtk.vfs mount tracker 7,
org.freedesktop.portal.Settings ReadOne 5 + a portal property read
(Slint, Avalonia: color scheme), portal Inhibit and GNOME SessionManager
Inhibit 1 each (Firefox), com.canonical.AppMenu.Registrar 1 (Avalonia),
org.a11y.Bus 1. Firefox owned its single-instance name; AlbumWall owned
org.mpris.MediaPlayer2.albumwall. No notification was attempted in the run.
Decision (Dave): the settings portal first.

## Step 2c: the settings service and portal (built 2026-11-08)

- `vergence-server/src/settings.rs`: one settings map from
  `desktop.toml [appearance]` (color-scheme default/dark/light, accent
  #rrggbb, contrast normal/high, reduced-motion) and `settings.toml`
  (theme, icons, cursor, fonts), under the namespaces toolkits ask for
  (org.freedesktop.appearance, org.gnome.desktop.interface). Both files
  re-read within a second; only changed settings are pushed.
- Core bus: topic `settings` (2), GetSettings, Settings (complete, or the
  changed ones). `vbus settings`, `vbus watch settings`.
- D-Bus edge: provides org.freedesktop.portal.Desktop as ":1.0" with the
  Settings interface (ReadAll, Read, ReadOne, SettingChanged, version 2).
  Other portal interfaces answer UnknownMethod. Each setting a program
  reads is logged once.
- Settings app: an Appearance page (style, accent, high contrast, reduce
  motion) that applies at once; saving keeps desktop.toml's other tables
  (it used to rewrite the whole file, dropping the menu's items).
- Program names for the log are looked up off the main thread.

Tested nested with a test home: gdbus ReadOne/ReadAll/version/GetNameOwner
(":1.0"); appending `color-scheme = "dark"` and an accent to desktop.toml
sent SettingChanged for accent-color, color-scheme (u 1) and the GNOME
color-scheme ("prefer-dark") to gdbus monitor, and the same three as one
"changed" event to `vbus watch settings`. Server tests: 3 new.

## Step 3: notifications, drawn by the server (built 2026-11-08)

- `vergence-server/src/notify.rs`: one notification service. Cards stack
  down from the top-right of the monitor the pointer was on when the stack
  started, 340 points wide, text rasterized at each monitor's pixels,
  summary (2 lines) and body (4 lines, ellipsis), action buttons; light or
  dark from the color-scheme setting; a red edge for critical ones. A click
  runs the "default" action (if any) or a button's action, then closes the
  card. Timeouts: the app's, 5 s when it leaves it to the server, never for
  critical ones; at most 5 cards (the oldest closes as expired). Cards with
  buttons get 10 s; while the pointer is over any card every countdown
  pauses (at least 2 s left when it resumes).
- Ownership: only the connection that made a notification can replace or
  close it, and only it receives NotificationClosed / ActionInvoked
  (unicast; never broadcast).
- Core bus: Notify, CloseNotification, Notified, NotificationClosed,
  ActionInvoked. `vbus notify <summary> [body] [--critical] [--action k=Label]`.
- D-Bus edge: org.freedesktop.Notifications (GetCapabilities: body,
  actions, persistence; GetServerInformation; Notify with the urgency hint;
  CloseNotification). Body markup and icons are not drawn yet (not
  advertised, so apps send plain text).
- `vctl notes` lists the cards.

Live on Hubble 2026-11-08: three cards (normal, critical, with buttons)
sharp on the 14" at 3.28; a card clicked through the control socket and one
clicked by Dave ("Later") reported the right action to notify-send; hover
paused a countdown at 8.5 s and it resumed on leaving.

Tested nested: gdbus GetCapabilities/GetServerInformation; `notify-send -w`
returned when its card expired at 5.08 s (the unicast NotificationClosed
arrived); closing another connection's notification is refused; `vbus
notify` got Notified and "closed (expired)". Tests: 1 new (wrapping) plus
the new protocol messages.

## Wallpaper per screen, and your own pictures (2026-11-08)

- Server (`wallpaper.rs`): `[[wallpaper.monitor]]` entries (id = the
  monitor's EDID hash, as window memory names it; label for people; image,
  mode, color each optional over `[wallpaper]`; image "" = color only).
  The background color is per monitor too. The id comes from the output's
  own MonitorId: the first version read it from the backend, which is out
  of the state during a frame, so every monitor fell back to its port name
  and no entry matched (found live, fixed the same hour).
- Core bus: OutputInfo carries `id`.
- Settings app: Screen chooser (from the bus's outputs, followed live),
  "Its own wallpaper" switch, and "Your pictures" ([pictures] folders and
  files in desktop.toml) with our own picture browser (folder or pictures
  mode). Edits change only their own keys (`wall.rs`, 3 tests).
- Live: the ASUS showed its own picture (xfce-flower) beside the 14"'s
  (xfce-cp-dark).

## Not in this step

Permissions beyond the uid check; the pid; settings and notifications
topics; the D-Bus edge; cleaning up the sockets when the server is killed
by a signal (a stale bus socket is detected and replaced at the next start).
