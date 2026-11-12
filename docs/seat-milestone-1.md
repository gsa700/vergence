# Seat milestone 1: our own input dispatch, focus and grabs

Drafted 2026-11-11 (Dave: "write the Smithay plan"), the first step of
Smithay's replacement in docs/borrowed-review.md's order, agreed the same
day after native milestone 1 closed. **Accepted 2026-11-12: Dave answered
(a) to all three questions at the end** (xkbcommon used directly for the
seat's own keyboard state; the seat in its own crate, `vergence-seat`;
Smithay's drag and drop kept for now).

The goal: every pointer, keyboard and touch event in the server goes
through **one dispatcher of our own** that decides who gets it: a Wayland
or X11 surface, a native window, or one of the server's own parts (title
bars and edges, the dock, the apps popup, the desktop menu, notification
cards). Focus and grabs (a held button, a move, a resize, a menu that
takes every click) are ours. Smithay stays as the **Wayland edge**: it
turns our decisions into `wl_pointer`, `wl_keyboard` and `wl_touch`
events for the clients that speak Wayland, and nothing more.

## Where things stand (read 2026-11-11)

- **Smithay's seat decides for Wayland and X11 windows only.** Its pointer
  and touch focus are typed `WlSurface`; its grabs (`PointerGrab`) are
  Smithay traits. The server is 12,993 lines; the input part is
  input.rs (455), focus.rs (122), touch.rs (310), pointer.rs (141, device
  settings), grabs/move_grab.rs (193) and grabs/resize_grab.rs (414), plus
  the native input in native.rs (about 150 lines from
  `native_pointer_motion` on).
- **Three paths beside Smithay's.** Native windows have a parallel
  pointer path (`native_pointer_motion`, `native_pointer_button`: Smithay's
  focus is "none" over them) and a parallel touch path; the server's own
  parts are an if-chain at the top of `handle_button` (menu, dock, popup,
  notifications, title bar, Alt+drag, each with its own "swallow the
  release" flag), repeated in spirit in `pointer_to` (`menu_hover`,
  `notify_hover`, `shelf_hover`, `launcher_hover`).
- **The keyboard lock hazard.** Native keyboard focus goes through
  Smithay's `KeyboardTarget` (`KeyboardFocus::Native` in focus.rs), whose
  callbacks run while Smithay holds the keyboard's lock: reading the
  keymap there froze the server (2026-11-11). The rule "never touch the
  KeyboardHandle in a KeyboardTarget callback" is a rule because the
  design allows the mistake.
- **The grabs are smallvil's.** move_grab.rs and resize_grab.rs began as
  Smithay's example (MIT, attributed); they implement all fifteen
  `PointerGrab` methods, most as pass-throughs.
- **Kept from Smithay on purpose:** the Wayland protocol objects
  (`wl_seat`, the data device and its selection, drag and drop by
  `DnDGrab`), libinput through Smithay's back end, and xkbcommon (the
  keymap). The native milestone's clipboard (`ClipOwner`) already stands
  beside Smithay's selection.

## Design

**`vergence-seat`, a crate of our own with no Smithay in it**: the
dispatcher's decisions as plain data, tested on their own (as
vergence-shell is for the dock). It knows:

- **Targets**: `Target::Surface(id)` (a Wayland or X11 surface, the edge's
  to deliver), `Target::Native(window, ...)`, `Target::Server(Part)` with
  `Part` one of Title, Edge(edges), Button(which), Dock, Popup, Menu,
  Note. The server tells it what is under a point (from its stacking and
  `decor::Hit`, which already says this); the crate never looks at
  windows itself.
- **Pointer focus and the implicit grab**: the target under the pointer
  has it, except while a button is held: then the target the press went
  to keeps every motion and the release (what each "swallow" flag does by
  hand now). Enter and leave are generated in one place.
- **Grabs as states, not traits**: Move(window, offset), Resize(window,
  edges, start), Menu (the desktop menu, an icon's menu, the apps popup:
  a click outside closes it and goes no further), and Edge (a Wayland
  drag and drop in progress: the edge's `DnDGrab` has the pointer until
  it ends; see Questions, 3). One grab at a time; a new press while one
  is held is the grab's to take.
- **Keyboard focus**: a stack (the focused window, and above it the apps
  popup or a menu while open), the key filter (the server's own keys:
  volume, Super alone, Ctrl+Alt+Backspace, the launcher's typing) before
  any target, and focus changes as *results* the server applies after the
  event, never inside a callback. Native and Wayland targets are told
  alike.
- **Touch**: each finger gets a target at its first contact and keeps it
  (Wayland's rule, and ours for native windows and the dock); the long
  press and the mouse-back logic of touch.rs move in.

**The server's side**: `pointer_to`, `handle_button`, `handle_key` and
the touch entry points shrink to "ask the seat, apply what it says". For
a Surface target the edge is Smithay's `PointerHandle`, `KeyboardHandle`
and `TouchHandle`, called with the focus we chose (`motion(Some(surface,
location))` or `motion(None)`); Smithay's own grabs are not used except
drag and drop. The native paths and the if-chain go; move and resize
become the seat's grabs, so `grabs/` (smallvil's) is deleted, and
input.rs and focus.rs are rewritten in our own words (their smallvil
attribution goes; borrowed-review.md §1 is updated).

**The keyboard edge**: Smithay's `KeyboardHandle` stays the Wayland edge
and is fed every key (so its xkb state stays right); our seat keeps its
own xkb state from the same keys for native windows and the server
(Questions, 1). `KeyboardTarget` remains only on surfaces, where it is
Smithay's own; nothing of ours runs in its callbacks.

## Steps

1. **`vergence-seat`**: targets, pointer focus with the implicit grab,
   enter and leave, the grab states, keyboard focus stack and filter, touch
   slots. Tests for each rule, and a fuzz test (random event streams: never
   a panic, never two targets entered at once, every press's release goes
   where the press went).
2. **The pointer on it**: the server's parts, native windows and surfaces
   as targets; the if-chain, the swallow flags and the native pointer path
   go; Smithay's pointer called only for surfaces. Move and resize become
   the seat's grabs (`grabs/` deleted).
3. **The keyboard on it**: focus as results applied after the event; the
   key filter in the seat; native keyboard input without `KeyboardTarget`;
   the server's own xkb state (Questions, 1).
4. **Touch on it**: touch.rs's logic in the seat; the native touch path
   goes.
5. **Clean-up**: input.rs, focus.rs in our own words; borrowed-review.md
   and the whitepaper's §8 table updated; CLAUDE.md's keyboard-lock rule
   becomes history.

## Where it stands

- **Step 1 done** (2026-11-12, 6d52716): crates/vergence-seat, 30 tests and
  the fuzz test (a million events, every rule reached).
- **Step 2 done** (2026-11-12): the pointer goes through the seat
  (crates/vergence-server/src/dispatch.rs): what is under a point as a
  target, Smithay's pointer called only for surfaces, native windows by
  their own calls, the server's parts acting on the seat's presses; the
  if-chain, the swallow flags and the native pointer path are gone; move
  and resize are ours (`grabs/`, smallvil's, deleted); a program's move or
  resize request is granted by the seat only with a button held on it;
  grabs and held buttons end when the screens go off. On Techbench
  (Fedora, by `vctl`): clicks 50 of 50 within one frame on both roads;
  title bar drag, double-click maximize and restore, a border resize, the
  terminal moved by its own bar, the desktop menu and a click outside it,
  the dock minimizing and restoring Settings. Hubble and kepler: tests
  green; their sessions not yet run (both went off the network that
  evening). By hand still: Alt+drag, Firefox, an X11 program, a drag and
  drop, the touchscreen.
- **Step 3 done** (2026-11-12): the keyboard through the seat. The server
  keeps its own xkb state (crates/vergence-server/src/keys.rs; xkbcommon
  used directly, the keymap read back from Smithay's before the first key);
  the seat's key filter decides every key; Smithay's keyboard is fed every
  key and delivers only to Wayland and X11 windows; a native window has the
  keyboard, its keys and modifiers from the server, never through
  `KeyboardTarget` (focus.rs has no native kind any more). Who has the
  keyboard is the seat's (`keyboard_window`: the dock's active mark, title
  bars, the clipboard rule). Found on Techbench: the seat must be told
  which menu is open before each key too (a popup opened by Super did not
  get the next keys). On Techbench by `vctl`: typing and Ctrl chords in the
  native terminal and over Wayland, Super's popup taking the keys and
  Escape closing it with nothing leaking. The volume keys reach the sound
  code, which declines on Techbench (PipeWire): to check on Hubble.
- **Step 4 done** (2026-11-12): touch through the seat. touch.rs keeps
  what is the hardware's and the screen's (which screen a panel is on, a
  finger's reach, the half-second timer) and carries out the seat's
  decisions: Wayland touch through Smithay, a native window's over the
  bus, the pointing finger through the pointer's path, a long press's
  menus, a window going with the finger that asked (only by that program's
  own press serial). Its own finger bookkeeping is gone. Smithay's touch is
  framed only after something was sent to it. On Techbench by `vctl
  touch`: a finger dragging a title bar, a long press opening the desktop
  menu and a tap outside closing it, a tap on the settings app's sidebar
  (native touch), taps on a dock icon. Hubble's touchscreen (now on
  kepler's hardware) by hand still.
- **Step 5 done** (2026-11-12): input.rs rewritten in our own words (the
  scroll frame and the motion clamp were the last of smallvil's there; its
  attribution is gone); focus.rs was already ours and now says why it has
  no native kind; borrowed-review.md §1 and §2 and the whitepaper's §8
  table updated (1,025 lines of smallvil's input code gone or rewritten;
  xkbcommon also used directly); CLAUDE.md's keyboard-lock rule is
  history.

### Acceptance, where it stands (2026-11-12, evening)

1. Tests: **met on Techbench** (366, the seat's fuzz test with a million
   events among them). Hubble and kepler green through step 2; after steps
   3 to 5 still to run (both machines were being moved and kepler
   reinstalled that evening).
2. **Partly met on Techbench** by `vctl`: clicks 50 of 50 on both roads,
   typing and Ctrl chords in the terminal on both roads. Still to do:
   repeat and the volume keys (Hubble's ALSA), Firefox, foot, an X11
   program, the clipboard both ways, a drag and drop; Hubble and kepler.
3. **Partly met on Techbench**: title bars (drag, double-click, buttons,
   resize), the dock (click, menu), the apps popup and its keys, the desktop
   menu, by mouse and by finger (`vctl touch`). Still to do: Alt+drag (by
   hand), dragging the dock, notification cards, hiding docks, a real
   touchscreen.
4. **Met**: no `native_pointer_`, no swallow flag, no `PointerGrab` of
   ours, nothing of ours in a `KeyboardTarget` callback.
5. **Met**: smallvil's grabs gone, input.rs rewritten (borrowed-review.md
   §1 has the counts).

## Acceptance

1. All tests green on Hubble, kepler and Techbench, the seat's own and
   its fuzz test (a million events) among them.
2. On all three, in a session: the click test 50 of 50 on both roads;
   typing, shortcuts, repeat and the volume keys in the terminal (native
   and Wayland), Firefox, foot and an X11 program (Xwayland turned on);
   the clipboard both ways as in native milestone 1; a drag and drop
   between two Wayland programs (Firefox's tab to another window, a file
   from one program to another where one has it).
3. The server's parts as before: title bars (move, resize, double-click,
   buttons), Alt+drag on every kind of window, the dock (click, drag,
   right-click menu), the apps popup and its keyboard, the desktop menu,
   notification cards, hiding docks; on Hubble's touchscreen, the same with
   a finger.
4. No path of input outside the seat: `grep` finds no `native_pointer_`,
   no swallow flag, no `PointerGrab` implementation of ours; nothing of
   ours in a `KeyboardTarget` callback.
5. Smithay's share of the server drops: smallvil's files gone or rewritten
   (borrowed-review.md §1 updated with the counts).

## Security (whitepaper §7.3)

- No new socket or protocol. Input goes only to the target the seat
  chose; a client cannot take a grab (a Wayland client's move or resize
  request is honored only with the serial of its own press, as now).
- The key filter runs first, so the server's own keys (Ctrl+Alt+Backspace,
  the volume keys) cannot be swallowed by a program.
- A grab ends if its window goes (closed, crashed, unmapped) or the
  screens go off, so no grab outlives what it was for.

## Not in this milestone

- The window space and the renderer (step 2 of the order): `Win` and
  Smithay's `Space` stay as they are.
- The Wayland protocol objects themselves, the data device, and
  libinput/xkbcommon: kept (the edge).
- Snapping, Alt+Tab and keyboard window switching: the windows milestone;
  but the seat is built so they are a grab and a filter entry each.
- Tablets and gestures beyond what the server does now (nothing).

## Questions for Dave (answered 2026-11-12: (a), (a), (a))

1. **The keyboard's state.** (a) Smithay's `KeyboardHandle` stays the
   Wayland edge, fed every key, and our seat keeps a second xkb state
   from the same keys, through the **xkbcommon** crate used directly (it
   is in already, under Smithay; this makes it a direct dependency, same
   library, same license); (b) our seat reads Smithay's state (no second
   one, but it means reading inside Smithay's lock again, the hazard we
   are removing); (c) our own keymap code instead of xkbcommon (XKB's
   format is large; borrowed-review.md expects xkbcommon to stay for
   good). Proposed: **(a)**.
2. **Where the seat lives.** (a) A crate of its own, `vergence-seat`, no
   Smithay in it, tested alone (as vergence-shell); (b) a module in the
   server. Proposed: **(a)**: it is the piece the later steps (our own
   window space, then the back ends) build on, and it keeps Smithay's
   types out of the rules.
3. **Drag and drop between Wayland programs.** (a) Keep Smithay's
   `DnDGrab`: when a client starts a drag, the seat enters an Edge grab
   and hands the pointer to Smithay until the drop; (b) write our own drag
   and drop now (the data device's offers, the icon surface, the drop
   protocol). Proposed: **(a)** now; our own when the Wayland front end's
   turn comes, if ever (it is the edge we expected to keep).
