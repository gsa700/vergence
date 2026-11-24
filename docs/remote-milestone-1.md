# Remote milestone 1: Vergence Remote, using a computer from another one

*Redrafted 2026-11-23* with Dave, starting over from what we want (the
2026-11-22 draft, a screen with three modes, is kept below under "The first
draft"). Dave: "a remote desktop application that uses SSH or WireGuard to
access a system remotely. it should be able to really use the computer as
though you were sitting there". And: "This could replace the need for a KVM
if done correctly (not the hardware part obviously) but for actual USE."

**Status: answered 2026-11-23** (Dave: "all leans, go ahead"): the order of
the steps as below, the names Vergence Remote and `vergence-remote`, our own
machine list (ssh config hosts offered), our own compression for pixels
first. Building, step 1.

## What it is

**Vergence Remote**, an application: open it, pick a machine, and its
desktop is in a window (or the whole screen), to use as if you were at it.
It is what `ssh -X` was for a single program, for a whole desktop, with
today's security, drawing Vergence's own windows from their display lists
(sharp at any size, light on the network) rather than sending video.

The whitepaper fixes the direction: remote graphical access is the display
server's job, since only it has the frames and the input and can show who
is watching (principle 8); transport by SSH or WireGuard, logins through
vdm and PAM, never an unauthenticated TCP port (§7); drawing commands, not
video (§6.6, phase 4).

## Dave's answers (2026-11-23)

- **Hosts: Vergence only, our own protocol.** "eventually we would maybe add
  RDP and VNC support but let's just get our own working first."
- **Sound and video later.** Running Firefox remotely is rare ("surely it
  would be faster on the local machine"); with an SSH connection the local
  browser can reach the remote network anyway (below, "Browse through this
  machine").
- **One video encoder for two uses.** "we need a video codec for screen
  recording locally so maybe we can get them both with one": the recording
  milestone's encoder (our own, decided 2026-11-21 in
  docs/screenshots-milestone-1.md) is the one remote video uses later.
  Claude's lean for it: VP8 in WebM (open, RFC 6386, plays everywhere, the
  simplest open codec to write); Opus for sound.
- **Sessions: attach to the running session, the remote monitors blanked
  if wished.** Extend (a remote screen as one more monitor) and a separate
  login beside the desk's session come later.
- **The login screen: yes.** With nobody logged in, the viewer shows vdm's
  login screen and logs in through it; the session then starts on the
  machine as at the desk, and the viewer is attached to it.
- **The Dockworker shows it**, as the agent tile does: "Control viewing",
  how long, and End. Whoever is at the desk can always end it; locking the
  screen ends it too.
- **No prompt for the same user.** Connecting as the session's own user
  with an SSH key proves it is you: no question at the desk, the tile always
  shows. A setting, "Ask the person at the desk", for machines others use.
- **Clients on every major platform** (Windows, Mac, Linux, BSD). The Vergence
  viewer first, then the Mac (2026-11-22: "Vergence and Mac first"), through
  a portable build of the Vergence toolkit, as Native AOT builds an
  Avalonia app (2026-11-22). Alternatives Dave named: a .NET client, or
  native apps per platform.

## Milestone 1: what it does

1. **Connect over SSH.** The viewer runs `ssh <host> vergence-remote`
   (the host's own program, installed with Vergence); the whole session
   travels on that one channel. The person's keys, `~/.ssh/config`, jump
   hosts and host-key checks all work; WireGuard is the network path under
   SSH, not a mode of its own. Nothing listens on the network.
2. **See it**: the session's screens, one at a time or all, drawn by the
   host's server at the viewer's own size and scale (text rendered at the
   viewer's resolution, never scaled up), sent as the rectangles that
   changed, compressed (no video codec yet). See "Frames, not display lists".
3. **Use it**: keyboard and pointer as at the desk, every key passed through
   in full screen (Super, Alt+Tab, Ctrl+Alt+…; the Mac's Cmd mapped
   sensibly), touch where the viewer has it.
4. **Clipboard** both ways: text and pictures, with a switch per connection.
5. **Files** both ways: drag onto the remote desktop or out of it, and a
   transfer list, through SFTP on the same SSH connection (no file protocol
   of our own).
6. **Privacy**: blank (lock) the remote monitors while connected; unlocking
   at the desk ends the connection.
7. **The login and lock screens**, reached remotely (vdm takes the viewer).
8. **The machine list**: saved machines (name, SSH host), which are
   reachable, and switching between open ones with a key, as a KVM's button.
9. **Who is watching**: the Dockworker's tile and End, on the remote machine.

## Later (planned, not in milestone 1)

- **Sound** (Opus) and **video** for moving pictures, with the recording
  milestone's encoder.
- **Browse through this machine**: the same SSH connection as a proxy
  (`ssh -D`), so the local browser reaches the remote network's pages
  (pfSense, a NAS) with its own bookmarks and passwords.
- **Wake-on-LAN** for a machine that is asleep.
- **Extend** and a **separate remote login** beside the desk's session (the
  first draft's modes).
- **USB passthrough** (a YubiKey for the remote's sudo or ssh), printing.
- **Accessibility**: the remote apps' meaning trees, so a screen reader on
  the viewer's side reads them.
- **Other hosts**: RDP and VNC as front ends on the remote screen, and
  perhaps viewing Mac or Windows hosts.
- Phones and tablets.

## How it is built

**The host side, in the server.** A remote viewer is a screen of the
session with no monitor behind it, as the headless sessions' in-memory
screens are (`MEM-1`): it inherits drawing at its own scale, window memory
for its setup, the dock, the lock screen and input routing. Its frames go to
the viewer instead of memory. The native protocol is the first **front end**
on that screen, kept apart so RDP or VNC can be added later without touching
the screen.

**`vergence-remote`, the host's program**, run by SSH as the person: it
reaches the session (or vdm's login screen when nobody is logged in), and
relays the stream between the server and the SSH channel. Pictures and
keymaps, which local apps pass as file descriptors, cross as bytes, with a
cache so a picture crosses once.

**Frames, not display lists, in milestone 1** (Dave, 2026-11-23: "yes, go
with A"). Two designs were weighed:
- **A: the host's server draws each frame at the viewer's exact size and
  scale** and sends the rectangles that changed, compressed. Everything
  works at once (the bars' frost and tint, the dock, the pointer, the lock
  screen, Wayland programs), the picture is as sharp as a local one (the
  server draws at the viewer's resolution; native windows' scenes drawn
  again at that scale, as for a window across two monitors), and the viewer
  stays a thin window that shows rectangles. The cost is bytes: screens of
  text and flat color compress well, fine on a LAN and over WireGuard.
- **B: per-window display lists, composed by the viewer.** Lightest on the
  network, but the viewer becomes a compositor: the server's bars, frost,
  dock, menus, lock screen and effects drawn again on every platform and
  kept in step. Dave: "B sound a lot like we'd be shipping the entire
  toolkit and server?" Later, for slow links, B's idea can come in for the
  big parts (a window's contents as its display list) while the server still
  composes the rest.

**The protocol**, documented and versioned as the bus protocol is: frames
as compressed rectangles from the host; input, clipboard and screen sizes from
the viewer. Every frame from the host is untrusted input on the viewer's
side, as an app's display list is on the server's: validated, with limits.

**The client, in two layers.**
- **A portable core** (a Rust crate): the connection, the protocol, drawing
  display lists and pixels into a picture (vergence-render), the clipboard
  and transfer models, the machine list. No platform code.
- **A thin shell per platform**: a window, input, the clipboard, the
  screen's size and density.
  - **The Vergence viewer** first: a vergence-ui app, the easy case.
  - **The Mac viewer** next, as the first app of the toolkit's portable
    build (its AppKit road), signed with the lab's Developer ID. It needs
    Rust on Control (rustup in the home folder, not Homebrew; ask Dave
    first), and an early check that vergence-render's GL road runs on
    macOS's OpenGL 4.1, or whether the Mac needs a Metal road or a CPU
    fallback.
  - **Windows, Linux and BSD desktops** after, through the same build.
  - Should the toolkit's Mac or Windows roads lag, the same core can sit
    under another shell (a .NET one, if Dave wants) without being rewritten.

**Consent and the tile** reuse the agent interface's machinery, as server
features: the connection's identity, the Dockworker tile and End, the log.
The screen stream is its own channel, never the agent interface itself
(the agent layer stays frozen, 2026-11-22).

## Questions (with leans)

1. **The order of the steps** below: host side and protocol, then the
   Vergence viewer, then clipboard and files, then the login screen, then
   the Mac viewer as its own milestone (the portable build)? *Lean: yes.*
2. **Name**: "Vergence Remote" for the app, `vergence-remote` for the host's
   program? *Lean: yes.*
3. **The machine list's source**: our own list, or the hosts in
   `~/.ssh/config` offered to add? *Lean: our own list, with the ssh config's
   hosts offered when adding one.*
4. **Pixels for other programs' windows**: our own simple compression of
   changed tiles (zstd is borrowed; a plain run-length or our own LZ is not)?
   *Lean: our own LZ-style compression first, measured, before borrowing.*

## Steps

1. **The remote screen in the server**: a screen kind whose frames and
   damage go to a front end; attaching to the running session. Tests in the
   headless suite. (Blanking moved to step 4.)
2. **The protocol** (a document and a crate): versioned messages, display
   lists and tiles, input, sizes; the validation and limits on the viewer's
   side; unit tests with hostile frames.
3. **`vergence-remote`**: run by SSH, reaching the session over the bus,
   (found in step 1: while a view is open, remote input counts as activity
   and the machine does not go to sleep from idleness, energy.rs, or the
   person would be cut off mid-session),
   relaying, the picture cache; the Dockworker tile, End, the log; "Ask the
   person at the desk" off for the same user.
4. **The portable core and the Vergence viewer**: connect, draw, input, full
   screen with keys passed through, the machine list and switching; and
   **blanking** (moved from step 1): a curtain, a lock started by Vergence
   Remote that the remote side passes through (its input routed as usual,
   its view drawing the session) while the desk's monitors show the lock
   screen and the desk's keys reach only its password field; keyboard focus
   kept; unlocking at the desk ends the connection; a real lock (Super+L,
   idle) locks the remote too.
5. **Clipboard and files**: text and pictures both ways; drag and drop and
   transfers through SFTP.
6. **The login screen**: vdm takes the viewer before anyone is logged in.
7. **Checks**: a display-suite scenario (a headless session viewed by a
   headless viewer over a local SSH), tests on Hubble, techbench and kepler,
   latency measured on the LAN and over WireGuard, an independent security
   review before anything is used across the network.

Then **the portable build milestone**, with the Mac viewer as its first app.

## The first draft (2026-11-22), kept

The backbone agreed then still holds: a remote viewer is a screen of the
session (Dave: "should it be a new screen? ... it would inherit all the good
work we have done on screen memory, session management and so on"), one
screen type with three modes: **extend** (one more screen), **mirror** (a
chosen screen, redrawn at the viewer's scale), **take over** (local monitors
locked and blank, the remote screen the only one). Milestone 1 now builds
the attaching case (mirror, with blanking as take over's privacy); extend
and a separate remote login come later. Dave then: "client wise, we need to
be able run on multiple platforms. and plan the extra protocols for later
VNC, virt, RDP, etc."; "Vergence and Mac first, all three modes, both"; and
"right now I think the Vergence model is the priority and then building the
ability to compile a version like the aot for avalonia/.NET". Avalonia was
not to be used for the client (2026-11-22); a .NET shell over the same core
is listed above only as Dave's own alternative.

## Progress

- **Step 1, the remote view in the server** (2026-11-23, building):
  - 2217955: a remote view is a screen drawn again at a viewer's size and
    scale, each time the screen is drawn (its elements built afresh at the
    viewer's scale, so text and native windows are sharp), into a texture
    and damage tracker of its own; never an output. Changed rectangles merge
    as they come and wait for the front end's pull (the pull is the flow
    control); a pull sends only pixels that differ from what the viewer has,
    trimmed to what changed. The dock's tiles turned out to be damaged every
    frame with the same pixels (a known issue in STATUS): it costs nothing.
  - 0bd43a4: the Dockworker's tile, "Control is viewing this computer" (or
    "and 1 more"), the screens, since when, and End (ends every view).
  - The view's own clock: drawn alone every 16 ms when its screen is not
    being drawn (monitors off, blanked, or still), so it never freezes with
    the desk.
  - Display suite 21 (13 checks), with `vctl remote`, `vctl energy`.
  - **Blanking moved to step 4** (with remote input): the curtain is "remote
    input passes, the desk's does not", so it is built and tested with
    remote input. It also needs a test path into the lock for headless
    sessions only (the lock needs vdm answering and a session started from
    the login screen, so the display suite has never tested it).
  - Checks: workspace Hubble 1,047, techbench 1,047, kepler 1,036; display
    suite 21 of 21 on Hubble, scenario 21 on techbench.
- **Step 2, the protocol** (2026-11-23): the crate `vergence-remote` (no
  platform code): proto.rs, the stream (the opening `VRMT` and version,
  typed and length-framed messages, viewer to host: Hello, Resize, Ack,
  Key, Pointer, Button, Scroll, Bye; host to viewer: Welcome, Frame,
  Notice, Bye), and lz.rs, our own compression in LZ4's published block
  format (question 4: our own first). The viewer's side refuses what breaks
  the protocol: lengths capped, unknown types, rectangles outside the view
  or not decoding to exactly their size (a bad frame changes nothing).
  Documented for other viewers in docs/remote-protocol.md. Tests: 7
  (round trips, a screen compressing better than 20 to 1, hostile blocks
  and messages), on Hubble, techbench and kepler.
- **Step 3, `vergence-remote`** (2026-11-23): the server listens on a socket
  of its own beside the bus, `$XDG_RUNTIME_DIR/vergence-remote-<display>`
  (owner only, the same user only; remote.rs), so the screen's stream is a
  channel of its own and the bus is unchanged (no version bump). On it the
  same protocol as over SSH, frames uncompressed; `vergence-remote` (run by
  SSH) finds the session's socket, relays both ways checking every message,
  and compresses frames on their way out, so the compositor's loop never
  compresses. One frame in flight, the next after the viewer's Ack. Hello
  starts a view (the main screen: the one at the layout's origin), named on
  the Dockworker's tile; End there tells the viewer ("Ended from the desk")
  and closes it. A viewer is held to 25 megapixels (a whole picture must fit
  one message uncompressed on the local leg). The machine does not sleep
  from idleness while viewed. `vergence-remote-probe`, a viewer without a
  window, for tests and diagnosis. Display suite 22: the first frame of a
  1280x720 desktop was 22.6 KB on the wire (0.6% of raw), 83 ms from start.
  **Over real SSH** (2026-11-23, Dave's go): techbench viewed a headless
  session on Hubble with `vergence-remote-probe -- ssh hubble vergence-remote`:
  2560x1440 at 1.333, the first frame 76 KB, on screen 262 ms from the start
  (SSH's login included); 1280x720, 22 KB in 258 ms. (techbench has had no
  SSH key since its reinstall; the test used Control's key through a
  throwaway forwarded agent, killed after, so techbench's access is
  unchanged. Hubble's host key was added to techbench's known_hosts from
  Control's.)
- **Step 4, input and the Vergence viewer** (2026-11-23, building):
  - 4d89333: a viewer's pointer (mapped from the view's pixels), buttons,
    keys (evdev codes) and scrolling take vctl's paths into the seat; they
    count as activity; none while the elevation prompt is up. A viewer's
    keys reach the lock's password field (the lock screen is reached
    remotely). Display suite 23: a window dragged by its bar, text typed
    into Edit, from the viewer.
  - The portable core (vergence-remote's client.rs, no platform code): a
    session runs the command (`ssh -o BatchMode=yes HOST vergence-remote`),
    keeps the picture from the frames (checked, acknowledged), notes what
    changed, wakes the shell, and says plainly why it ended (ssh's own last
    words when it fails).
  - The toolkit's key events carry the physical key (`code`, evdev), so a
    viewer sends keys the host maps with its own layout.
  - `vergence-remote-viewer HOST`: the Vergence shell, a window showing the
    host's desktop, one pixel to one (the view is the window's pixels below
    its bar, resized with it), drawn as 256-pixel tiles so a frame costs
    only the tiles it touched; pointer, buttons, keys (no repeats: the
    host's) and scrolling sent; held keys and buttons let go when the
    window loses the keyboard; the host's name in the title once known.
    Display suite 24: the viewer viewing its own session (a mirror).
  - Checks: workspace Hubble 1,054, techbench 1,054, kepler 1,043; display
    suite 24 of 24 on Hubble and on techbench.
  - The machine list: `vergence-remote-viewer` with no host lists the saved
    computers (`~/.config/vergence/remote-machines`, one SSH destination a
    line), adds one typed or offered from `~/.ssh/config`'s hosts (question
    3), removes one; choosing one opens its window (each computer a window
    of its own). Destinations are checked (no spaces, no leading dash, no
    shell's characters) and go to ssh after `--`. Display suite 25.
  - **The whole keyboard** (bus v40): the viewer asks the server for every
    key while it has the keyboard (`WinRequest::CaptureKeys`); the seat
    then forwards every key to it, the server's own shortcuts included
    (Super, Alt+Tab, Ctrl+Alt+…), but switching consoles; Ctrl+Alt pressed
    and let go alone gives the keyboard back (`WinEvent::KeysCaptured`
    false), and a click takes it again. The title says how. A view of this
    very computer (`localhost`) is looking only: input there would come
    back to the window and go out again, round and round. Display suite 26
    (with a stand-in host: `--command`); vctl key learns `Alt`.
  - **Blanking, the curtain** (protocol version 2: `Privacy`): the viewer's
    bar button, Blank the desk / Show the desk. The desk locks as by
    Super+L, but the window with the keyboard keeps it; remote views draw
    the session (decor.rs, `remote.drawing`), and the viewer's input passes
    one event at a time (the seat's lock set aside quietly around it,
    `remote.passing`), while the desk's monitors show the lock and its keys
    reach only the password field. Two meanings of locked: the desk's
    (`locked_for_input`: drawing, input routing, dialogs' keys) and the
    session's (`session_locked`: screenshots, the session dialog, apps'
    screenshot requests, which the remote person may use behind the
    curtain). Left fully locked on purpose: agents' requests, and the
    elevation prompt (answered at the machine). Unlocking at the desk ends
    the connection; the viewer leaving leaves the desk locked, as any lock.
    A lock of any other kind is never passed. Tests reach the lock in a
    headless session only (`--headless`: no vdm needed there, `vctl lock
    test-unlock`); real sessions still need vdm and a password. Display
    suite 27.
  - Step 4 built. Settings and packaging (Dave asked 2026-11-23 whether
    Agents and Remote should be compile or install options; proposed: a
    Settings switch, off by default, with no socket while off, and a
    separate package for the remote programs): waiting for his word.
