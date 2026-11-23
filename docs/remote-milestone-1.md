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

## Questions for Dave (to settle before building)

1. **Which clients first**: Vergence to Vergence (the natural start); a viewer
   for the Mac (Control); Windows (the hamshack PC); later perhaps the iPhone.
2. **Standard protocols**: also speak VNC or RDP (pixels, a borrowed library
   or our own encoder) so any existing viewer works, or native only at first?
3. **Which modes in milestone 1**: all three, or mirror and take over first
   (the daily use) and extend after?
4. **The running session or a login of its own**: attach to the session at the
   desk, start one of its own, or both (the modes above suggest both).
