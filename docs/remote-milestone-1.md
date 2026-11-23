# Remote milestone 1: remote access, a screen across the network

*Drafted 2026-11-22.* Dave chose remote access as the milestone after Terminal
on the agent interface: "when we next get to a milestone we should work on the
remote access protocol and client(s)". The whitepaper already fixes the
direction: remote graphical access is the display server's job, since only it
has the frames and the input and can show who is watching (principle 8);
transport through SSH or WireGuard, logins through vdm, never unauthenticated
TCP (§7); and drawing commands, not video (phase 4, §6).

## The backbone: a remote screen (Dave, 2026-11-22)

Dave, reading the whitepaper: "should it be a new screen? it seems like there
would be some benefits, it would inherit all the good work we have done on
screen memory, session management and so on. but if you have software running
you would want to interact with directly a new screen wouldn't fit unless we
created a specific screen type that copies the Main screen?" Agreed the same
evening: **a remote viewer is a screen of the session**, one screen type with
three modes.

Vergence already has screens with no monitor behind them: the headless
sessions' in-memory screens (`MEM-1`, headless milestone 1). A remote screen
is one of those whose frames go to a client instead of memory.

What a screen inherits:
- **The viewer's own scale.** The client reports its size and density as a
  monitor's EDID does; the server draws display lists sharp at that scale (a
  Retina laptop and a 1080p one each get crisp text, never scaled video).
- **Window memory per monitor setup.** "At home, two monitors" and "remote,
  the laptop" are two setups: windows go where they were the last time remote,
  and back when home.
- **Session restore, the dock and its tiles, the lock screen, input routing,
  agents and accessibility**, unchanged: to them it is another screen.

The three modes:
1. **Extend**: the remote screen is one more screen, like a monitor plugged in
   across the network. For a remote login of its own, or a machine whose
   monitors are off.
2. **Mirror**: a view of a chosen screen (the main one by default), to see and
   drive what runs there, as Screen Sharing does. Even mirrored, display lists
   are drawn at the viewer's scale: a 4K screen seen from a laptop is drawn
   again sharp, not shrunk.
3. **Take over**: the local monitors lock and blank (privacy for whoever is at
   the desk), and the remote screen becomes the session's only screen; window
   memory moves everything to the remote layout, and unlocking at the desk
   brings it all back.

## Things the design must answer

- **A hybrid stream.** Vergence's own windows send display lists; Wayland and
  X11 programs (Firefox, GTK, Qt) send pixels. The protocol carries both:
  drawing commands where there are any, compressed pixels for the rest, and
  only what changed.
- **Who is watching** (principle 8): a mirrored or taken-over session shows a
  clear indicator on its local screens, and the person at the desk can end it.
- **Input**: the viewer's keyboard and pointer join as seat devices of their
  own, so local and remote input do not fight.
- **Logins and locks**: a remote login goes through vdm and PAM like a local
  one; locking from either end locks the session.
- **Transport**: SSH (a channel or a forwarded socket) or WireGuard; never a
  listening TCP port of our own.

## Clients and protocols (Dave, 2026-11-22)

Dave: "client wise, we need to be able run on multiple platforms. and plan the
extra protocols for later VNC, virt, RDP, etc."

**Clients run on several platforms from the start.** One client, written in
Rust, in two layers:
- **A portable core** (a crate of its own): the connection, the protocol, and
  drawing what arrives (display lists through vergence-render, pixels by
  decoding), with no platform code. The same core in every client.
- **Thin shells per platform**: a window, input, the clipboard and the
  screen's size and density from each system. First Vergence itself and the
  other Linux and FreeBSD desktops (Wayland, X11 later if wanted), the Mac,
  and Windows; phones later.
- **The protocol is documented and versioned** (as the bus protocol is), so
  others can write clients of their own.

**Extra protocols come later, planned for now.** The remote screen produces
frames, damage and display lists and takes input; a protocol is a **front end
on that screen**, so adding one never touches the screen model:
- **Native** (milestone 1): display lists and pixels, ours.
- **VNC (RFB)**: pixels only; any VNC viewer works. Likely the first extra.
- **RDP**: pixels with better compression; Windows' own client works.
- **Virtual machines** ("virt": SPICE, as virt-manager and virt-viewer use):
  for VMs, and for showing a session to those tools.
- Each one arrives with its own security review, and with the same rules as
  native: logins through vdm, transport through SSH or WireGuard (or the
  protocol's own TLS when bound only to a tunnel), the watcher indicator.
- Whether any borrows a library is asked of Dave when its turn comes.

So milestone 1's work includes keeping that front-end seam clean: the native
protocol is the first front end, not wired through the screen.

## Answers (Dave, 2026-11-22)

"Vergence and Mac first, all three modes, both."
1. **Clients**: several platforms; the **Vergence and Mac shells first**,
   Windows after.
2. **Standard protocols**: later, planned now as front ends (above).
3. **Modes**: **all three** in milestone 1 (extend, mirror, take over).
4. **Sessions**: **both**: attach to the session at the desk, or a remote
   login of its own through vdm.

## The order: Vergence first, then a portable build (Dave, 2026-11-22)

Asked whether the Mac shell should use Avalonia, and reminded that the apps
roadmap already plans toolkit roads for macOS and Windows, Dave set the order:
"right now I think the Vergence model is the priority and then building the
ability to compile a version like the aot for avalonia/.NET".

1. **This milestone: the Vergence model.** The remote screen and its three
   modes in the server, the native protocol as the first front end, vdm's
   remote logins, and the client on Vergence (vergence-ui, its native road).
   The client's core stays free of platform code so the next step can carry
   it.
2. **After it: a portable build of Vergence apps**, as `dotnet publish` with
   Native AOT gives an Avalonia app: one command per platform that makes a
   self-contained native app (a signed .app on the Mac, an .exe on Windows)
   from the same code. That is the toolkit roads of the apps roadmap (AppKit,
   then Win32), with vergence-render drawing on each. **The remote client is
   its first app**, so the Mac client arrives through the toolkit, not a
   second codebase; Edit, Reader and Vergence Music can follow.
3. **Avalonia is not used for the client.** It stays the toolkit of Dave's
   existing Mac, Windows and Pi apps (AlbumWall, Deadwax, Marginal).

So "Vergence and Mac first" means: Vergence in this milestone, the Mac as the
first target of the portable build that follows.

## What the answers bring with them

- **A Mac build** (the portable-build milestone, not this one). Control has no Rust today. The Mac shell needs Rust there
  (rustup, which installs into the home folder; not Homebrew) and is signed
  with the Developer ID the lab's Mac apps already use, so Gatekeeper lets it
  run. Ask Dave before installing rustup on Control.
- **The Mac shell draws with vergence-render**, so its GL road must run on
  macOS (OpenGL there is old, 4.1 and deprecated): find out early whether it
  does, or whether the core needs a Metal road or a CPU fallback for the Mac.
- **Two sessions at once** for a remote login while someone is at the desk:
  vdm today runs one session per seat; a remote login is a session with no
  seat of its own (its only screen is the remote one), which vdm must allow
  and show (who is logged in remotely, on the login screen and in About).
- **Take over and the lock**: the local screens lock with vdm's lock screen,
  and unlocking at the desk ends the take-over (the viewer sees why).

## Next

Write the protocol outline and the build steps (the remote screen in the
server, the native front end, the portable client core, the Vergence shell,
the Mac shell, then the modes and the vdm side), each with its tests and a
security review before anything listens. Building starts after the terminal
agent fixes are in and installed.
