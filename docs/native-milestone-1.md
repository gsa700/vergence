# Native milestone 1: our tools' windows on our own protocol (whitepaper §6.3 to §6.5)

Drafted 2026-11-10 (Dave: "write the native protocol plan"). **Accepted
2026-11-10 22:32** (Dave: "yes to all three, go": the three open questions
answered as proposed; see Decided).

The goal: the terminal and the settings app open their windows, draw,
take input and use the clipboard over the project's own protocol, on the
connection that already carries the session bus, with no Wayland between
them and the server. Placement (Contract 2) and meaning (Contract 3) get
their first versions on the wire. Other applications are not touched:
Wayland and X11 stay as they are, as edges (X11 off unless turned on).

This is step 4 of the order of work set on 2026-11-09 (whitepaper §9):
the toolkit, the shell and the settings app are on the project's own code;
what is still borrowed between them and the screen is the Wayland
protocol, spoken by our own client code (`vergence-wl`) and answered by
Smithay in the server.

## Where things stand (read 2026-11-10, 9b128a8)

- **The native protocol** (`crates/vergence-protocol`) is the session bus:
  frames of `[u32 length][u16 opcode][payload]`, at most 64 KB, on a Unix
  socket in the private runtime directory, every connection's peer uid
  checked (`bus.rs` in the server). Messages: hello, ping, subscribe,
  outputs, settings, notifications. **It cannot pass file descriptors**,
  and a window's pixels (shared memory, GPU buffers), a keymap and the
  clipboard's data all travel as file descriptors.
- **The toolkit** (`vergence-ui`) speaks Wayland through `vergence-wl`
  (about 1,800 lines of our own): a window, the exact scale through
  fractional-scale and viewporter, shared-memory and GPU buffers
  (linux-dmabuf), pointer, keyboard, touch, the clipboard, cursor shapes,
  move and resize. Every program on it (the terminal, the settings app,
  the demos) goes through `vergence_ui::run`.
- **The server** keeps its windows in Smithay's `Space<Window>`, and
  Smithay's `Window` knows only Wayland and X11 windows. About a hundred
  places in the server name it: focus, the move and resize grabs, title
  bars (`decor.rs`), window memory (`memory.rs`), placement, the dock, the
  menu, touch. A window of our own protocol needs an element type of the
  server's own beside it.

## Design

**One connection.** A program has one connection to the server: the bus
socket it has today. Windows join the bus messages there, as the
whitepaper says (§6.8: "on the same connection as windows, with the same
identity and the same permissions"). The wire gains **file descriptors
alongside frames** (`SCM_RIGHTS`), a few to a frame at most, each counted
in the frame so a reader knows how many to expect; a frame that claims
descriptors it did not bring, or brings some it does not claim, ends the
connection.

**A window.** A client asks for a window with its **identity** (Contract
2: application id, a role such as "main" or "preferences", and a
persistence token it may keep from a previous run), a title, a size asked
for, and whether it draws its own bar. The server answers with the size
to draw at, **in reference units**, and the exact scale of the monitor it
is on (Contract 1); it says again when either changes. The window is
shown when the client first sends a picture.

**Pictures.** A client sends a buffer and the damage since the last one:
shared memory (a descriptor, the size, the stride, the format) or a GPU
buffer (dmabuf: the planes' descriptors, offsets, strides, the format and
modifier), at the scale's exact pixels. The server shows it and says when
it would like the next (a frame event, at the monitor's pace), so a busy
client never draws faster than the monitor shows. Synchronization is
implicit, as on the Wayland path today; explicit sync (§8.3) replaces it
later on both paths at once. **Scenes instead of pixels** (display-list
surfaces, §6.6) are phase 3; the window message leaves room for a second
kind of picture so they need no new window model.

**Input.** The server sends the window what the user does to it: the
pointer (enter, leave, motion, buttons, the wheel already scaled by the
Mouse and touchpad page), the keyboard (the keymap once as a descriptor,
key codes and modifiers, the repeat settings), fingers, focus. A window
gets input only while it is the one being pointed at, touched, or given
the keyboard; there is no global input (§7.3). The client asks for a
cursor shape, to be moved or resized from a press, to be maximized or
minimized; the server decides.

**The clipboard.** Copy offers kinds of data; paste asks for one and is
given a pipe. The server bridges this to Wayland's and X11's clipboards,
so copying from the terminal and pasting into Firefox works as it does
now.

**Placement (Contract 2), a first version.** A window may ask to be at a
place relative to a monitor, or to its parent window; the server decides,
and window memory (`memory.rs`) remembers the window by its identity
rather than by title, per monitor setup. The rule that a request never
covers another application's window, and user rules ("the logbook always
opens on the left monitor"), come later.

**Meaning (Contract 3), a first version.** The toolkit already builds
every window's meaning (`Ui::meaning`: role, name, value, states,
bounds). A window sends it to the server when it changes, as a tree with
stable ids, bounds in reference units, the focus and the caret. The
server keeps it per window and `vctl tree` prints it. Nothing else reads
it yet: the magnifier, screen readers through an AT-SPI bridge and agents
(§6.5) are phase 3, and will read what is already there. A secret field's
text never leaves the program (§7.3), as today.

**The server's own element.** A new element type, `Win`, holds either a
Smithay `Window` (Wayland, X11) or a native window, and the server's
space, focus, grabs, title bars, memory, dock and menu use it. Wayland and
X11 windows behave exactly as now. The change is made first on its own,
with nothing native in it, so it can be soaked before anything else
depends on it.

**The toolkit's two roads.** `vergence-ui` gets a native back end beside
its Wayland one, behind the same `App` interface: a program opens its
window natively when the server offers it (`VERGENCE_BUS` names a server
that speaks windows), and over Wayland otherwise, so the terminal and the
settings app still run under other desktops. Nothing in a program changes.

**Limits and checks.** Every count, size and descriptor is bounded and
checked as the bus's messages are today; a buffer's size must match its
descriptor's; a client's windows, buffers and tree have per-client caps. A
fuzz test feeds the decoder random frames and descriptors.

## Steps

1. **The wire.** Descriptors on the bus socket (`SCM_RIGHTS`, both ways,
   counted per frame, on Linux and FreeBSD); the window, picture, input,
   clipboard, placement and meaning messages in `vergence-protocol`, with
   their encoding, limits and a fuzz test. A client library in the same
   crate. The server accepts and checks the new messages and answers
   "not yet"; `vbus` can open a connection and list what the server says.
2. **The server's element.** `Win` in place of `Window` across the server,
   Wayland and X11 only, behaving as before: whole workspace green, then a
   soak on Hubble and kepler (an evening of Dave's normal use).
3. **Native windows on the screen.** A native window becomes a `Win`:
   configure (size, scale), shared-memory pictures with damage, frame
   events, close, title bars for windows that ask for one, the dock's icon
   by application id, window memory by identity. A small test program
   (`vnative`) draws a window natively.
4. **Input, and GPU pictures.** Pointer, keyboard (keymap, repeat), touch,
   focus, cursor shapes, move, resize, maximize, minimize; dmabuf
   pictures; the clipboard, bridged to Wayland and X11.
5. **The toolkit on it.** The native back end in `vergence-ui`; the
   terminal and the settings app open natively in a Vergence session and
   over Wayland elsewhere. Parity with the Wayland road, checked feature by
   feature from the toolkit's code (as the terminal's parity was checked
   against iced): exact scale, both renderers, opacity, touch, timers, the
   clipboard, minimize, session restore.
6. **Contract 2 and Contract 3, first versions.** Placement requests and
   memory by identity; the meaning tree on the wire, kept by the server,
   `vctl tree`.

## Acceptance

1. In a Vergence session on Hubble, kepler and Techbench, the terminal and
   the settings app run natively (`vctl windows` says so), and everything
   they do over Wayland they do natively: checked against a list written
   from `vergence-ui`'s Wayland code before step 5.
2. The six-tab terminal stress test runs at least as well natively as over
   Wayland: the same `seq 1 1000000` time and flood rates within 10%, no
   click lost.
3. Copy in the terminal and paste in Firefox, and the other way, work.
4. A window that asked for its place is put there when the request is
   allowed, and comes back there after a restart by its identity.
5. `vctl tree` prints the settings app's controls, names and values, and
   the focused one; a secret field shows no text.
6. Wayland applications behave as before (Firefox, AlbumWall, foot), X11
   applications too when they are turned on (Xwayland is off by default
   since 2026-11-10), and the soak of step 2 is clean.
7. The decoder's fuzz test runs clean for a million frames; the server
   refuses a frame with wrong descriptor counts, an oversized buffer or a
   buffer smaller than it claims, and stays up.
8. Outside a Vergence session (Xfce on Hubble, GNOME on Techbench) the
   terminal and the settings app still open over Wayland.

## Security (whitepaper §7.3)

- The bus socket's checks stand: private directory, mode 0600, peer uid.
- A native window gets the same treatment as any other window: no global
  input, no reading other windows, its place a request (§6.4).
- Descriptors are accepted only where a message expects them; a client
  can make the server neither open files nor map more than it was given
  and the caps allow.
- The meaning tree is kept for the user's tools; nothing reads it without
  a permission, and that permission is phase 3's.

## Not in this milestone

- Display-list surfaces (scenes on the wire) and their sharp straddling
  windows and magnification: phase 3, on the window model built here.
- Readers of Contract 3: the magnifier, the AT-SPI bridge, agents.
- Explicit sync, more than one GPU, NVIDIA (§8.3).
- Real popup windows (the toolkit's choice lists stay inside the window,
  as decided 2026-11-10); user placement rules and their settings page.
- Other toolkits on the native protocol (Avalonia, §8.1).
- Removing Smithay's Wayland front end: it stays the edge for everyone
  else's applications.

## Decided (Dave, 2026-11-10 22:32)

1. The toolkit keeps its Wayland road as a fallback; our tools still run
   under other desktops.
2. The server's element change is made and soaked on its own first (step 2).
3. Contract 3's tree goes on the wire in this milestone, read only by
   `vctl tree` for now.

## Progress

- **Step 1 (2026-11-10): the wire.**
  - `vergence_protocol::sock`: descriptors beside the bytes (`sendmsg`,
    `recvmsg` with `SCM_RIGHTS`, declared for Linux and FreeBSD), a pipe,
    `send_all`; past a limit, descriptors nobody claims are closed at once.
    Moved here from `vergence-wl`, which now uses it, so there is one copy
    of that code.
  - `vergence_protocol::window`: the requests (create with identity, destroy,
    title, acknowledge a configure, attach a shared-memory or GPU picture with
    damage, cursor, move, resize, maximize, minimize, place, meaning, copy,
    paste) and events (created with a token, configure, close, frame, pointer,
    keymap, keyboard, modifiers, repeat, touch, clipboard offer, send and
    data), opcodes from 100. A message that carries descriptors says how
    many first; one that claims descriptors that did not come is refused.
    Checked as they are read: picture sizes (16,384 pixels a side at most, a
    stride that holds the width, under 1 GB), counts (256 damage
    rectangles, 4 planes, 32 kinds, 4,096 meaning nodes, 16 states), finite
    numbers.
  - The bus's `ClientMsg` and `ServerMsg` carry window messages;
    `encode_with_fds` and `decode_with_fds` take the descriptors along. The
    client library sends and reads them. A new error code, `NotYet`.
  - The server reads descriptors on every bus connection (64 at most
    waiting), ends a connection whose window message does not add up, and
    answers a window request "not yet". `vbus window [title]` asks for one.
  - Tests: every request and event across a real socket with real
    descriptors; pictures that do not add up refused; a fuzz test of window
    frames with descriptors waiting beside them (a million rounds clean on
    Hubble in 5.4 s, release).
  - On Hubble, installed and restarted: `vbus ping` 0.16 ms; `vbus window
    Hello` answered "refused (NotYet): windows on the native protocol: not
    yet". The settings app's bus client (screens) and every Wayland window
    as before.

- **Step 2 (2026-11-10): the server's own element, `Win`.** `win.rs`: an
  enum with one case today, a Smithay `Window` (Wayland or X11), that the
  space, focus, the move and resize grabs, title bars, window memory,
  placement, the dock, the menu and touch now hold instead of Smithay's
  type. It answers what Smithay's space asks of an element (`SpaceElement`,
  `IsAlive`, `WaylandFocus`, `AsRenderElements`) and passes the calls the
  server makes (geometry, activation, surfaces, frames) to the window
  behind it. A native window becomes its second case in step 3. 14 files,
  about 75 lines each way; nothing behaves differently.
  - **Found while trying it, and fixed** (it was older than this step: kepler's
    build from before showed it, and Dave had met it on FreeBSD): a
    double-click on a maximized window's title bar did nothing. The first
    press took the window out of maximized at once (the "drag a maximized
    window out" rule ran on the press, not the drag), so the second press,
    a double-click, maximized it again. Now a maximized window leaves
    maximized only once the pointer has moved 6 points with the button down
    (`move_grab.rs`), for the server's title bars and an application's own
    (the terminal's bar) alike. On Hubble: double-click restored (880 x 640),
    double-click maximized again, a drag took it out under the pointer.
  - Soak: an evening of Dave's normal use on Hubble and kepler, then step 3.


- **Step 3 (2026-11-11): native windows on the screen.** `native.rs` in the
  server: a native window is `Win::Native`, the second case of step 2's
  element. On `Create` the server places it (window memory by identity
  first, then fitted and centered on the monitor under the pointer),
  answers `Created` with a token (`app/role/index`, memory's key) and a
  `Configure` (size in reference units, the monitor's exact scale), and
  shows it with its first picture.
  - **Pictures** are shared memory, the two formats now named in
    `vergence-protocol` (`SHM_ARGB8888`, `SHM_XRGB8888`: DRM fourcc codes).
    The server reads the damaged rows with `pread` instead of mapping the
    client's file: a client that shrinks its file gets a short read and
    loses its connection, where a mapping would take the server down with
    SIGBUS (no signal handling needed, the same on Linux and FreeBSD). A
    picture that claims more than its file holds is refused before
    reading. The pixels go into a `MemoryRenderBuffer`, drawn by the scene
    (`decor.rs`) 1:1 at the scale they were drawn for.
  - **Frames**: an attach asks for the next; after drawing a monitor the
    window is on, the server sends `Frame` (both back ends). A window that
    moves to a monitor of another scale is sent a new `Configure`.
  - **Configure pacing**: one configure waits for the client to
    acknowledge the last; a newer size waits meanwhile, so a resize drag
    sends one size per acknowledgement, as X11's sync request does.
  - **The server's frame and its parts**: title bars for windows that do
    not draw their own, close (sends `Close`), maximize and restore (by
    configure), minimize to the dock and back, the border band's resize
    (the resize grab's native path), Alt+drag; the dock's icon and window
    memory by identity (`memory.rs`: application id and role).
  - **Fixed on the way**: `focus_window` unwrapped a Wayland surface
    (a click on a native window would have stopped the server), and the
    resize grab did the same; both now handle windows without one.
  - `vnative` (vergence-protocol's second tool, installed with the rest)
    draws a window natively and counts frames; `--short-file` lies about
    its picture. The shared-memory helper moved from vergence-wl to
    `vergence_protocol::shm` (vergence-wl re-exports it), as `sock` did.
  - Tests: the damage copy (only its rectangle, through a stride, clipped,
    a short file an error). 318 green on Hubble, kepler and Techbench.
  - **On Hubble** (14" 4K at scale 3, 30 Hz): a 480 x 320 window drawn at
    1440 x 960 pixels under the server's title bar; 240 frames in 8.1 s,
    a median 33 ms apart (the monitor's pace), the moving bar sent as two
    damage strips; maximize (1280 x 696) and restore; close by its
    button; moved to 150,300, closed, and opened there again by memory;
    resized from its right edge 480 to 587 (configures 527, 557, 587 as
    acknowledged); minimized to the dock and back. The lying client was
    disconnected ("picture of 5535360 bytes in a file of 5529600") and the
    server stayed up. The terminal (Wayland) ran alongside throughout.
  - **Not yet seen**: a move between monitors of different scales (Hubble
    had one monitor connected; the code path is the same `Configure`); a
    live run on kepler (built and tested there; its session is Dave's).
    Until step 4 a native window gets no input of its own, so its title
    bar draws as unfocused even when it is in front.

- **Step 4 (2026-11-11): input, GPU pictures, the clipboard.**
  - **Keyboard**: a native window is a third kind of keyboard focus
    (`focus.rs`, `KeyboardFocus::Native`), so Smithay's seat delivers its
    keys. The keymap goes once per client as a descriptor, then the repeat
    settings (again when they change), keys as evdev codes, modifiers as
    xkb serializes them.
  - **Pointer** beside Smithay's, whose focus type is a Wayland surface: over
    a native window Smithay's focus is none and `native.rs` sends enter,
    leave, motion in the window's reference units, buttons and the wheel,
    keeping the window while a button is held (an implicit grab). The
    window's cursor shape (the cursor-shape protocol's numbers) is drawn
    while the pointer is on it. **Fingers** the same way (`touch.rs`).
  - **Requests**: move and resize with a button held on the window (its own
    bar and edges), minimize, maximize.
  - **GPU pictures**: a dmabuf attach is kept and imported into the
    renderer when the scene is next built, drawn as a texture. The rule a
    client follows, now in the protocol's documentation: a GPU buffer is
    free again once a later picture has been attached and a `Frame` has
    followed (the server swaps pictures at attach), so a client takes two
    or more in turn. No release event needed.
  - **The clipboard**, through Smithay's server-side selection, whose user
    data now says who owns it (`ClipOwner`: X11 or a native client): a
    native copy becomes the clipboard for Wayland clients and Xwayland; a
    native paste gets the read end of a pipe whose write end goes to the
    owner (a Wayland client, X11, or a native client, which hears
    `ClipboardSend`); native clients hear `ClipboardOffer` when it changes.
    Only a client whose window has the keyboard may copy (§7.3). The same
    change closed an older gap: a Wayland paste of X11's clipboard never
    reached X11 (`send_selection` was not implemented).
  - **Found on Hubble and fixed**: the server froze on the first click into a
    native window, because Smithay calls `KeyboardTarget::enter` holding the
    keyboard's lock and the keymap was read there (`with_xkb_state` takes
    the same lock); it is now read before the focus changes. And the bus had
    never sent descriptors to a client: its outbox now keeps each message's
    descriptors and sends them with the write of its first byte.
  - `vnative` moved to vergence-ui (for the renderer): a square follows the
    pointer or a finger, keys printed (Esc quits), `--own-frame` moves and
    resizes by its own strip and corner, `c` copies and `v` pastes, `--gpu`
    draws with the GPU renderer into two dmabufs taken in turn.
  - **On Hubble**: the keymap (35,758 bytes), repeat, "ab" Return as keys
    30, 48, 28; moved by its own strip and resized by its corner (one
    configure per acknowledgement); the crosshair drawn; `--gpu` on the UHD
    630, 240 frames at the monitor's 30 Hz, green bar, no warnings; copy in
    vnative and Paste in the terminal's menu gave "copied from vnative, a
    native window"; the terminal's Copy of its date line and `v` in vnative
    gave "Wed Nov 11 20:36:08 CST 2026"; vnative to itself (native to
    native) too. Tests: 322 green on Hubble, kepler and Techbench.
  - **Not yet seen**: the wheel (vctl has no scroll command), a finger (no
    touch panel on Hubble now; the code path is the pointer's), X11's half
    of the clipboard (X11 is off by default), a live run on kepler.
