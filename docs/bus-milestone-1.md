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
   and replug. **To do**, after the session server is restarted on this
   build.

## Not in this step

Permissions beyond the uid check; the pid; settings and notifications
topics; the D-Bus edge; cleaning up the sockets when the server is killed
by a signal (a stale bus socket is detected and replaced at the next start).
