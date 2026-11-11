# Security pass, 8 November 2026 (narrow)

Dave asked for it after the high-level review of the same evening: a short
pass over the places where trust boundaries are being set this week, not a
full review (the code changes daily; that one belongs at the milestone where
other people run the desktop). Tree: 10dbb53 plus the working tree. Scope:

1. the control socket (`crates/vergence-server/src/control.rs`);
2. the identity model behind the three sockets (control, session bus, D-Bus edge);
3. the three parsers that take bytes from another process: the bus frames
   (`vergence-protocol`), the D-Bus wire protocol (`vergence-dbus`), the
   terminal's escape sequences (`vergence-term`), each read and then fuzzed.

Method: read the code against whitepaper §7.3; check the live state on Hubble
(socket modes, directory modes, the running server's environment); write
deterministic random-input tests for the parsers and run them in debug and in
release at high counts on Hubble.

## Findings

| # | Severity | Where | Finding | Status |
|---|---|---|---|---|
| 1 | High (by the project's own rules) | control.rs | The control socket was always on. Owner-only means any program running as the user, and its commands (`spawn`, `pointer`, `press`, `focus`, `move`, `shot`) bypass every mitigation in §7.3: input injection, focus stealing, screen capture, running commands. | **Fixed.** Off unless `VERGENCE_CONTROL=1`. tools/vergence-session and tools/run-server-drm.sh set it (that line is what makes a session a development session); the server logs loudly when it is on. |
| 2 | Medium | vergence-term grid.rs | Restoring a saved cursor (CSI u, ESC 8, and leaving the alternate screen with DECSET 1049) did not clamp to the screen. Save at the bottom right, shrink the window, restore, print: index out of bounds, the terminal process dies. Ordinary editors do exactly this on a resize; any program's output can also do it on purpose. Found by the fuzz test in its first run. | **Fixed:** `restore_cursor` clamps; regression test `restored_cursor_stays_on_a_smaller_screen`. |
| 3 | Low | control.rs `save_png` | Screenshots were written with the umask: on Hubble `~/vergence-shots` was 0755 and the PNGs 0644. A screenshot is the user's screen. | **Fixed:** directory 0700, file 0600, written beside and renamed into place (old files are replaced with the right mode on the next shot). |
| 4 | Low | control.rs | A connection is served on the compositor's thread with blocking I/O: a 2 s read timeout, no write timeout, no cap on the line. A client that connects and stalls holds the display for 2 s; one that never reads its reply could hold it indefinitely once the reply exceeds the socket buffer. Owner-only, so a stuck `vctl`, not an attack. | **Mitigated:** write timeout 2 s, line capped at 64 KiB. Proper fix open: a per-connection calloop source with an outbox, as bus.rs does. |
| 5 | Info | bus.rs, dbus.rs, control.rs | Same uid is the whole trust model: every socket admits any process of the user and refuses everyone else. `app_id` (Hello) and the D-Bus program name are recorded, not trusted, as the code says. There is no per-application identity yet, so none of §7.3's per-application permissions can be built. | Design recommendation below. |
| 6 | Info | main.rs, dbus.rs | `std::env::set_var` (unsafe in edition 2024) sets WAYLAND_DISPLAY, VERGENCE_BUS and DBUS_SESSION_BUS_ADDRESS in the process environment after the backend is up. If any library thread reads the environment at that moment it is a data race. | Open, low. Alternative: keep a server-side map of variables for children and apply it in `spawn()` with `Command::env`, so the process environment is never written. |
| 7 | Info | vergence-dbus message.rs | `Message::big` is a public field, but the constructors marshal the body at once in little-endian; flipping the flag afterwards labels a little-endian body big-endian (the fuzz test tripped on this before it was understood). The edge never does this: a client's big-endian message is routed in its own byte order. | Open, API nit: make the field read-only or marshal lazily. |
| 8 | Info | control.rs | The socket is bound, then chmod 0600: a window at the umask mode. Harmless: the directory is 0700 and every connection's uid is checked anyway. | None needed. |
| 9 | Info | vergence-session | `vctl restart` starts whatever binary is at target/release: any process of the user that can write there runs as the compositor on the next restart. Inherent in a development launcher; same uid again. | tools/install.sh (2026-11-09) puts root-owned binaries under /usr/local/bin and an installed session runs those, with the control socket off unless the user asks (~/.config/vergence/control-socket); a checkout's session still runs target/release. |

## What the reading confirmed

- Frames: at most 64 KiB; a count is checked against the bytes left before
  anything is allocated; strings must be UTF-8; a frame that breaks the format
  ends the connection; a client with more than 1 MiB unread is cut off.
- D-Bus: messages at most 8 MiB (header and body each checked, and their
  sum); values nested at most 64 deep; array lengths checked against the
  buffer before items are read (no allocation by claimed length);
  authentication EXTERNAL only, the claimed uid must equal the kernel's;
  NEGOTIATE_UNIX_FD refused (no descriptors cross the edge); 512 match rules
  per connection; 32 MiB outbox. Apps that need descriptor passing fall back.
- Terminal: 32 parameters, each saturating at 65535; OSC strings capped at
  4096 bytes; DCS/SOS/PM/APC skipped.
- Peer credentials: `getpeereid` on FreeBSD, `SO_PEERCRED` on Linux
  (struct ucred layout and length checked). A peer whose uid cannot be read
  is refused.

## Fuzz tests (new, run by `cargo test`)

Deterministic (fixed seeds), 20 000 cases per test by default,
`FUZZ_ITERS=<n>` for more; the terminal's replays one case with
`FUZZ_CASE=<n>` and prints the failing byte stream.

- `crates/vergence-protocol/tests/fuzz.rs`: random client and server
  messages encode and decode to the same value (floats included); mutated
  frames (byte flips, truncation, insertion, 0x00/0xff runs) and random
  payloads under plausible headers, fed in random chunks, never panic.
- `crates/vergence-dbus/tests/fuzz.rs`: well-formed messages with nested
  arrays, dictionaries, structs and variants round-trip through serialize,
  parse and args; mutated and random messages, the SASL exchange and match
  rules never panic.
- `crates/vergence-term/src/fuzz.rs`: random streams biased towards escape
  sequences with edge values (0, 65535, 65536, 99999999, 2^32, sub-parameters,
  private markers, intermediates, bad UTF-8, wide and combining characters,
  alternate screen, 5000-byte OSC) with random resizes between chunks, on
  screens from 2x1 to 130x60; plus 40 edge sequences from 5 cursor positions
  on 4 screen sizes.

Results: finding 2 on the first run of the terminal fuzzer (two code paths).
No other panics. Long runs: see the line at the end of this file.

## The identity model (finding 5): recommendation

Same uid is where every Unix desktop stands today, and §7.4 says so
honestly: without a sandbox, code running as the user can impersonate the
user. The question is what identity the server can attach to a connection so
that permissions in §7.3 have a subject. Two steps:

1. **Now, cheap, both platforms: attribution by pid.** The kernel gives the
   peer's pid with the uid (FreeBSD `LOCAL_PEERCRED` xucred has `cr_pid`
   since 13; Linux `SO_PEERCRED`); the D-Bus edge already logs it. From the
   pid, the executable (FreeBSD `kern.proc.pathname`, Linux `/proc/<pid>/exe`)
   and, when the server started the program, the desktop entry it started it
   from. Record this per connection next to the self-declared `app_id` and
   show it where identity is shown (the server-drawn title bar's app name,
   `vctl dbus`). It is attribution, not proof (pids are reused; a program can
   exec another), but it is the server's own observation rather than the
   client's claim.
2. **Later, for permissions: the server as launcher.** The server starts
   applications (menu, autostart, activation), so it can give each a private
   token in the environment (or an inherited socket) and tie the connection
   to the launch. Children inherit it, which is the known limit; a sandbox
   (Capsicum, Landlock) is what closes it, and §7.3 already plans one for
   display-list rasterization. Permissions attach to the launch identity, with
   the indicator and audit log §7.3 asks for.

Do not spend effort on per-application secrets before step 1: the data it
produces (who really connects, and how) decides what step 2 has to protect.

## Not covered

Smithay's handlers (xdg_shell, X11), wallpaper decoding (`image`, `resvg`,
on the user's own files), EDID parsing (from hardware), `vergd` (phase 1,
frozen), settings and desktop TOML (the user's own files), text rasterizing
of notification strings (`ab_glyph`, glyph lookup only), the Xwayland
boundary. The settings app and the terminal as Wayland clients were not
looked at as clients.

## Next

- Finding 4's proper fix when control.rs is next touched.
- Finding 5 step 1 when the bus gets its next service.
- Run the fuzz tests at a high count after any change to the three parsers
  (`FUZZ_ITERS=1000000 cargo test --release -p vergence-protocol -p vergence-dbus --test fuzz`;
  `FUZZ_ITERS=100000 cargo test --release -p vergence-term fuzz`).
- A full review at the milestone where someone other than Dave runs the
  desktop, or before the repository goes public.

**Long runs, Hubble, 2026-11-08 18:15 CST (release builds, after the fixes):**
1 000 000 cases per test for vergence-protocol (3.6 s) and vergence-dbus
(10 s), 100 000 random streams plus the edge sequences for vergence-term
(27 s): no panics. The whole workspace's `cargo test` passes (debug).
