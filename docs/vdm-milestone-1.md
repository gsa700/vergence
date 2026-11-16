# VDM milestone 1: the login screen and the lock screen

**Drafted 2026-11-16** (Dave: "we will need (a) if we want to run on systems
without gnome or some other login manager as people expect that these
days"). The app side of the alternating order, after dock milestone 1. VDM,
the Vergence Display Manager, is on the apps roadmap (2026-11-15: "VDM
display manager + lock screen first; YubiKey/fingerprint via PAM/fprintd").

**Answers 2026-11-16** (Dave): "1 a" (a list of the machine's people, with
Other…), "2 b (I plan to remove gnome)" (VDM replaces GDM on Techbench
too), "3 discuss" (below), "4 a but also not allowed with any encrypted FS"
(automatic login offered, off by default, never with more than one person,
and never on a machine with an encrypted file system: autologin would
hand the machine to whoever powers it on, undoing the encryption's point),
"5 b" (Vergence and the installed Wayland sessions), "6 a" (the lock screen
first). "Let's make sure we get this right, it's really important."

**Question 3, settled 2026-11-16 (Dave: "c"):** vdm is the one part that
checks passwords, and **it always runs**: `install.sh` enables it as a
system service whether or not it is the machine's login screen, so every
session can lock however it was started. Two rules with it: **never show
a lock that is not one** (if vdm cannot be reached, Super+L says so and the
desktop stays visibly unlocked); and **the server is the lock** (no locker
app whose crash could leave the desktop open; under vdm a server that dies
ends the session and the greeter comes back).

**Before any code** (Dave: "yes to the threat model and review"): a threat
model in this document, then an independent review of the design by a
reviewer who sees the design and the code, not this reasoning; then the
steps; the security pass at the end as well.

**Later, its own milestone: elevation prompts** (Dave: "can this same
mechanism give up per command elevation prompts on the desktop as well?").
Yes: the server draws the prompt (no app can draw a look-alike or read it),
PAM checks the administrator, vdm carries out **one named action with
checked arguments**, never an arbitrary command; a yes is for that request
only. It can take over the FreeBSD setuid helpers and the Linux polkit rule,
and is the permission question §6.5 and §7 ask for agents. Not in this
milestone; vdm's protocol and its PAM conversation are built so that it is
one more kind of request, not a rework.
This is security code: a part of it runs as root and it handles passwords,
so it ends with a security pass of its own (as 2026-11-08 and 2026-11-15).

## Where things stand

- A session starts from a text console: log in on agetty, type `vergence`
  (tools/vergence-session): seatd for the seat where there is no logind
  (Hubble on Void, kepler on FreeBSD), logind on Fedora (Techbench, whose
  GDM offers "Vergence" and "Vergence (dev)").
- The script makes XDG_RUNTIME_DIR itself where no PAM session made one
  (/tmp/xdg-<uid>, checked to be the user's).
- There is no lock screen. Energy (energy.rs) turns screens off and runs a
  sleep command; nothing asks for a password when the screens come back.
- Nothing of ours runs as root except two small setuid helpers on FreeBSD
  (sound-default, media-eject), each built for one task and reviewed.

## Progress

- **Step 1, part 1a** (2026-11-16): crates/vergence-dm. `sys` (the peer's
  uid; a locked page out of swap and core files; no core files; no
  debugger: PR_SET_DUMPABLE, procctl PROC_TRACE_CTL_DISABLE_EXEC), `secret`
  (a password in such a page, wiped, no Debug or Display), `proto` (the
  unlock protocol, lengths checked first, a fuzz test), `pam` (our own
  declarations; OpenPAM's and Linux-PAM's result numbers apart, checked by
  asking each system's PAM to name them), and **vdm**: root, refuses to
  start without /etc/pam.d/vdm-unlock, the unlock socket (0666, the caller
  by its uid), root and system accounts refused, one check per uid at a
  time, each in a forked child with a 120 s limit, the pause after
  failures (1, 2, 4 ... 300 s) kept in /var/lib/vdm or /var/db/vdm,
  everything to the system log. `vdm-try` asks from a terminal.
  tools/install.sh installs vdm in sbin, writes /etc/pam.d/vdm-unlock on
  the system's stack (password-auth on Fedora, system-auth on Void, system
  on FreeBSD) and enables the service (systemd, rc.d through daemon(8) to
  syslog, runit with vlogger). Seen on kepler and Hubble: running, wrong
  passwords refused with the growing pause, in the auth log. Tests (9) on
  all three. The right password waits for Dave's own hand (`vdm-try`).

- **Step 1, parts 1b and 1c** (2026-11-16): the lock in the server
  (lock.rs). The seat's lock state checked before every other decision
  (`Seat::lock`/`unlock`, `KeyOut::Lock`/`Locked`, `Part::Lock`; while
  locked every key but volume and console switching, every press, motion,
  wheel and finger is the lock's; tests); locking ends grabs, menus, the
  launcher, cards, the drawer and dock drags, takes the keyboard from every
  window (Smithay's focus and native windows told) and goes back to the
  desktop's cursor; no program's cursor is taken while locked, no window
  takes focus, no window is told modifiers; each screen's scene is only the
  lock (on the pointer's screen), a dimming and the wallpaper. The dialog
  talks to vdm in a thread (vergence-dm's protocol), shows PAM's prompts and
  messages, takes typed keys only, honours vdm's pause, and stays locked
  saying it waits if vdm goes (I8). Locks on Super+L, Ctrl+Super+Q, the
  named action "Lock the screen", `vctl lock now`, when the screens go off
  for idle, before the server's own sleep, and on coming back to the
  console after the idle time (I7). It refuses honestly (a notification;
  once only for idle) when vdm's socket is not there or the session could
  fall back to a shell: the session script's parent must be login(1), vdm
  or a display manager's session runner (C2). The marker
  $XDG_RUNTIME_DIR/vergence-locked: the session script does not start a
  server again after one that ended locked, removes a stale one at a fresh
  login, and a server that finds it starts locked (C1). The session's
  server refuses debuggers and core files (T6, I1). While locked vctl
  answers only `shot` and `lock` (T4). Seen on kepler: asked to lock in a
  session started from zsh, it stayed unlocked and said why. Tests 666
  Hubble, 659 kepler. Next: Dave starts kepler with `exec vergence` and
  tries the lock by hand.

- **Step 1, by hand on kepler** (Dave, 2026-11-16, started with `exec
  vergence`): Super+L locks (after a fix: L and Q had never been mapped to
  the seat's keys; a test now covers the seat's letters); wrong passwords
  refused with vdm's growing pause (8 s by the fourth); the right password
  unlocks; console switching works while locked (as decided, "2 a"). Asked
  by Dave: the pause now counts down on the screen each second, the field
  greyed and taking nothing until it is over.

- **Step 2** (2026-11-16): sleep from anywhere locks (review C4). vdm's
  session socket: each user's server stays connected (the connection is the
  registration, its account from the kernel; four per account at most);
  vdm's control socket, root's only; `vdm before-sleep` (from the sleep
  hooks) makes vdm tell every session LockNow and wait up to 5 s for
  Presented or CannotLock, logging the rest. The server answers Presented
  only once a frame queued after locking has reached every screen (its
  VBlank came), or at once if the screens are off (their CRTCs cleared).
  Hooks by tools/install.sh: FreeBSD's rc.d/vdm has the suspend keyword and
  a suspend command (/etc/rc.suspend runs it before the lid, the power
  button, acpiconf or zzz sleep); Void /etc/zzz.d/suspend/50-vdm; Fedora
  /usr/lib/systemd/system-sleep/vdm (pre). Seen on Hubble: before-sleep
  locked the session and had "1 of 1 sessions locked" back in 51 ms.
  Found by Dave on Void: Void's pam_unix waits about 2 s after a wrong
  password (FreeBSD's does not), and the countdown began only at the next
  key because the PAM child sent the "no" without the pause; now vdm's
  parent, which keeps the pauses, sends the answer with the pause in it, so
  the countdown starts at once everywhere; a child closes the connections
  of other checks it inherited.

- **A real suspend on kepler** (Dave at the machine, 2026-11-16): `zzz` →
  /etc/rc.suspend → vdm's rc.d suspend → the session locked ("the machine
  is going to sleep", "shown on every screen") before FreeBSD slept;
  resumed at 12:24:05 to the lock; unlocked. Earlier: vdm's "no" now comes
  in about 60 ms on Linux too (PAM_FAIL_DELAY taken over by vdm, which
  keeps its own pause; finished checks reaped every 50 ms while one runs;
  Dave: FreeBSD had been "smoother and more consistent").
- **Step 3, C6** (2026-11-16): on kepler the server's children held no
  input or display device (only their own GPU render node), but every one
  had the console the session started on as its controlling terminal (so
  could open /dev/tty). Every program the server starts, and the portal's
  file dialog, now runs in a session of its own (setsid) with no
  controlling terminal (main.rs `no_terminal`). Checked on kepler: none
  left. Revoking input devices at a session's end is step 5's reaper.

- **Step 3 done** (2026-11-16): I5: FreeBSD's setuid helpers (sound-default,
  media-eject) are root:vergence 4750, a group of their own the installing
  user joins (from the next login), not the seat's video group. I10: the
  greeter's account `_vdm` (a system uid: 900 on kepler, 994 on Hubble),
  home /var/empty (root's, not writable), shell nologin, in the seat's group
  (video, _seatd; on Fedora logind will give the greeter's session its
  devices), not in vergence; vdm refuses it for unlock as a system account.
  Its state on a tmpfs made fresh at each start is step 4's. Seen on kepler
  and Hubble.

- **The log out dialog** (Dave, 2026-11-16: "dim the screen and have a
  proper dialog prompt", in place of the second small menu "Exit: close all
  apps / Cancel"; built now, "a"): session_dialog.rs. Every screen dimmed,
  a panel on the pointer's screen: "Log out of Vergence?", Lock, Cancel and
  Log Out (the default, in the accent; Enter); Escape, Cancel or a press
  outside closes it; Tab and the arrows move; the pointer over a button
  makes it the default. Every key and press is its while open (seat
  `Part::Session`). Opened by the desktop menu's "Exit Vergence…" and the
  named action. Log Out is exit.rs's: every app asked to close as its close
  button would. Sleep, Restart and Shut Down join it in step 6. Seen on
  kepler.

- **Step 4, a change of design** (2026-11-16): the server draws the login
  dialog itself, as it draws the lock, instead of running a greeter app:
  the toolkit's text field passes text around as ordinary strings, which
  cannot be wiped, while the server's dialog keeps it in a `Secret` from
  the first key; greeter mode can then start no program at all; and the
  lock and the login share one dialog and one conversation with vdm.
- **Step 4, part 4a** (2026-11-16): greeter mode (`vergence-server
  --greeter`, review C3). `crate::spawn` refuses every program; not started
  at all: applets, the battery history, network control, the lock, the
  session bus, the D-Bus edge (so no portal), Xwayland; off: the dock, the
  launcher, the desktop menu, session restore and autostart, the desktop's
  icons, Super+L and the log out dialog, the sleep command. The control
  socket stays as it is (off unless VERGENCE_CONTROL=1, and owner-only:
  _vdm's).

- **Step 4, part 4b** (2026-11-16): logging in. The protocol's Login
  (Hello for Login, then the user and the session's index; a user name of
  letters, digits, . _ - only, never starting with -); vergence-dm's
  sessions list (the system's wayland-sessions entries, Vergence first,
  shared by the greeter and vdm); vdm's greeter socket (0600, owned by
  _vdm, peer checked), which refuses root, system accounts, accounts with
  no real shell and unknown ones before PAM runs, applies the same pause
  per account, and runs PAM service `vdm` in a forked child (install.sh
  writes /etc/pam.d/vdm on the system's login stack: password-auth with
  postlogin and pam_loginuid on Fedora, system-local-login on Void, login on
  FreeBSD). The server's greeter.rs: the people from /etc/passwd (same
  rules) with initials, Other… (a name typed first), the password in a
  `Secret`, PAM's prompts, the pause counting down, the session line (F2 or
  a press changes it), the clock and the host's name; the seat in its lock
  state the whole time. A "yes" says so; starting the session is step 5.
  Not yet seen: vdm starting the greeter on a VT of its own (4c).

- **Step 4, part 4c** (2026-11-16): vdm runs the login screen. `vdm
  greeter` (root, over the control socket) has vdm start
  `vergence-server --drm --greeter` from $prefix/bin as _vdm on its own VT
  (vergence-dm vt.rs: VT 7 on Linux, past Void's agetty 1-6; VT 9, ttyv8,
  on FreeBSD, past /etc/ttys' 0-7): made the one on the screen first so
  seatd gives it the seat, in a session of its own with the VT as its
  terminal, groups, gid and uid set (and a check that root cannot be had
  back), a cleared environment, a home and runtime directory made fresh
  each time under vdm's run directory, output to STATE_DIR/greeter.log
  (root's, 0600). vdm starts it again when it ends, after 1, 2, 4 ... s
  if it ended within 30 s, and gives up (and says so) after five such
  ends in a row. `vdm greeter stop` ends it. Seen on kepler: it starts
  as _vdm on ttyv8, one person and one session (Vergence), no Xwayland.
  From step 7 vdm starts it at boot.
  Dave logged in at it on kepler (2026-11-16 12:55: "it works, welcome
  showed up"; vdm logged the login as yes). Step 4 is done.

- **Step 5** (2026-11-16, built; seen on kepler: below): starting and
  ending sessions. vdm's login worker keeps the PAM transaction open on a
  yes: it tells vdm at once (vdm answers the login screen, stops it, and
  starts it again when the session is over), sets PAM_TTY, XDG_SEAT,
  XDG_VTNR, XDG_SESSION_TYPE and XDG_SESSION_CLASS (I4), establishes
  credentials and opens the session, becomes the reaper
  (PR_SET_CHILD_SUBREAPER / PROC_REAP_ACQUIRE), and starts the chosen
  session's Exec (from vdm's own reading of the system's session list,
  M2) through the user's login shell ("-zsh -c 'exec ...'") on a VT of
  its own (Linux 8-11, FreeBSD 10-13; at most four sessions): a session
  of its own with the VT as its terminal, setlogin on FreeBSD, groups,
  gid, uid, root checked gone, the environment from nothing but PAM's
  and the session's own (T17), XDG_RUNTIME_DIR from PAM (pam_xdg on
  FreeBSD, logind on Fedora) or else made by vdm (/run/user/<uid>, I3;
  left in place at the end, as another login may use it). When the
  session's process ends, everything left below the worker is sent
  SIGTERM, then after 3 s SIGKILL (C6), and the PAM session is closed and
  credentials deleted. vergence-dm's sessions.rs reads Exec lines
  (field codes left out); pam.rs's `authenticate_then` and `Handle`.
  Seen on kepler (2026-11-16 13:17-13:19, Dave): logged in at the login
  screen, Vergence started on VT 10 (server <- vergence-session <- vdm's
  worker; pam_xdg made /var/run/xdg/derickson), Super+L locked and vdm
  unlocked, Log Out ended it: vdm ended 3 processes the session had left,
  closed the PAM session (pam_xdg removed the runtime directory), and the
  login screen came back. Seen on Hubble the same way (13:22-13:24):
  VT 8, vdm made /run/user/1000 (Void has no PAM module for it) and left
  it at the end, 1 leftover process ended. Found on the way: `vdm greeter stop` left the
  screen on the empty VT 9 (vdm now switches back to the VT it was asked
  from), and the server tried a frame ten times a second while paused
  (it now draws nothing until its VT is back). Not yet: Techbench (logind; the greeter's own
  logind session, I4). One session per user for now: a second login as
  the same user would share its runtime directory and single-instance
  app sockets (to be refused or given its own, step 6).

## Design

### The parts

1. **`vdm`, the daemon** (a new crate, vergence-dm): the only part that
   runs as root. Started by the init system (runit on Void, rc.d on
   FreeBSD, a systemd unit on Fedora). It owns the login: it runs PAM, starts
   the greeter and the user's session, and verifies passwords for the lock
   screen. Small on purpose, every line read in the security pass; it opens
   no network socket and draws nothing.
2. **The greeter**: `vergence-server` itself, run by vdm as an unprivileged
   user of its own (`_vdm`), on the seat through seatd or logind as any
   session is, with one native app on it, **`vergence-greeter`**. So the
   login screen has the desktop's look, its frost and its fonts, and is
   **right-sized on every monitor from the first frame** (Contract 1: the
   EDID-calculated scale applies before anyone logs in, which no other
   display manager does). The greeter asks vdm, over a socket only `_vdm`
   may open, to log a user in; it never sees a password check's result
   beyond "yes" or "no", and cannot start anything itself.
3. **The PAM conversation is the greeter's dialog**: vdm passes every PAM
   prompt to the greeter (a password; a one-time code; "touch your security
   key"; "place your finger") and its answer back, so whatever the system's
   PAM is set up for works without code of ours for each: pam_unix
   passwords, **pam_u2f for a YubiKey** (Dave's login key), **pam_fprintd
   for a fingerprint** on Linux (FreeBSD has no fprintd), pam_oath codes.
4. **Starting the session**: on success vdm opens the PAM session (on
   Fedora pam_systemd registers it with logind), sets the user's groups and
   identity, makes XDG_RUNTIME_DIR where nothing else does (/run/user/<uid>,
   0700, the user's, removed at the end), and starts
   `vergence-session` through the user's login shell on the same virtual
   terminal; the greeter's server has given up the seat first. When the
   session ends, vdm closes the PAM session and shows the greeter again.
5. **The lock screen** is drawn by the user's own server, not an app: it
   covers every screen, takes all input, keeps windows' contents out of
   screenshots and the Alt+Tab list, holds back notifications, and asks for
   the password in the same dialog as the greeter's. It locks on **Super+L**
   (and Ctrl+Super+Q, the Mac's), on a named action ("Lock the screen"),
   after a set idle time, and before the machine sleeps. The password is
   checked by vdm (the server asks over a second socket; vdm checks that the
   asker's uid is the user being unlocked), so no setuid helper is needed:
   FreeBSD's PAM cannot check a password without root.

### The greeter

6. A clock and date, the machine's name, and the person: their name and
   picture, a password field, the PAM prompts as they come. Below it: Sleep,
   Restart, Shut down (vdm carries them out; on a machine with other people
   logged in it asks first).
7. Keyboard layout and text size for the field (Accessibility's size and
   contrast apply at the login screen too).
8. After a wrong password, a pause that grows (PAM's own faillock where the
   system has it); the field shakes and says so, nothing more.

### What changes for each machine

9. **Void (Hubble)**: `tools/install.sh --vdm` adds the runit service and
   points tty1's getty elsewhere (a text console stays on tty2).
10. **FreeBSD (kepler)**: an rc.d script (`vdm_enable`), ttyv1 stays a
    text console.
11. **Fedora (Techbench)**: by Dave's answer (question 2).
12. Nothing is installed or enabled without `--vdm`: today's console start
    keeps working, and so does GDM.

## Threat model

Written 2026-11-16, before any code, at Dave's word ("let's make sure we
get this right"). It is checked again by an independent review of the
design, and against the code in the security pass at the end. It builds on
whitepaper §7: deny by default, the server draws whatever must be trusted,
nothing on the network, and §7.4's honest limit.

### What is protected

- **A1. Every account's password** (and second factors: a YubiKey's
  touch, a fingerprint). Learning one is the prize: with it come the
  account, its files, and `sudo` if it is an administrator.
- **A2. A locked session's contents**: what the windows show, what is
  typed, the clipboard, the session's power to act as its user.
- **A3. Who may start a session at all** (the greeter's one job).
- **A4. The point of disk encryption**: a machine powered on by a stranger
  must still ask for something before it is anyone's desktop.
- **A5. The machine's power and its other users' sessions** (Restart and
  Shut down from the greeter; sessions of others on the same machine).

### Who might attack, and from where

- **P. A person at the machine** without a password: keyboard, mouse,
  touch, a USB device plugged in (a keyboard emulator typing guesses), the
  power button, a monitor plugged in. **The adversary the lock screen and
  the greeter exist for.**
- **L. Another local account**: another person's session on the same
  machine, or someone logged in over SSH as someone else.
- **U. A program running as the user** (malware, a malicious script, a
  confused agent). §7.4 applies: it can already read the user's files and
  run `su`/`sudo` attempts. VDM must give it **nothing it does not already
  have**, and must not let it reach other accounts.
- **G. The greeter, compromised** (a bug in vergence-server or the
  greeter app exploited through what it handles: input devices, fonts,
  monitors' EDIDs). It runs as `_vdm` and sees passwords typed at it.
- **N. The network.** VDM opens no network socket; nothing here listens.
- **D. Someone with the disk** (the machine off, the disk taken). Only
  disk encryption defends; VDM's part is A4 (no automatic login with an
  encrypted file system).

### The parts and what each is trusted with

| Part | Runs as | Trusted with | Must never |
|---|---|---|---|
| **vdm** | root | checking passwords (PAM), starting sessions as a user, power actions | read files a user controls, run a command it was sent, open a network socket, keep a password after its check |
| **greeter** (vergence-server + vergence-greeter) | `_vdm`, nothing else | showing the login dialog; the keys typed into it | check a password itself, start any program, keep a typed password after sending it, reach a user's files |
| **user's server** | the user | drawing the lock, holding all input while locked | unlock on anything but vdm's yes to its own request; show a window while locked |
| **vdm's greeter socket** | root, `_vdm` only | the greeter's requests (log in, power) | answer anyone but `_vdm` |
| **vdm's lock socket** | root, any local user | "is this the right password for **me**?" | check anyone but the caller (its uid from the socket, not from what it says) |

### Threats and their defenses

| # | Who | Threat | Defense | Checked by |
|---|---|---|---|---|
| T1 | P | Guessing the password at the lock or the greeter (by hand or a USB keyboard emulator) | A pause after each wrong answer that grows; PAM's own lockout where the system has it (pam_faillock on Fedora); every failure in the system log | a test of the pause; the log |
| T2 | P | Getting past the lock without the password: switching to a text console, Ctrl+Alt+Backspace, killing or crashing the server, unplugging or plugging a monitor | Under vdm, the session's terminal has no shell: a server that ends brings the greeter, never a logged-in console. While locked: Ctrl+Alt+Backspace and every server key but volume and brightness are off; a new monitor shows the lock; a text console (Ctrl+Alt+F2) asks for its own login | tests by hand on all three; seat tests for the keys |
| T3 | P | Seeing the session while locked: a frame of the desktop shown on waking, after sleep, on a new monitor, in a notification | The lock is drawn and **presented on every screen before** sleep proceeds or the screens go off; notifications held back; a new monitor's first frame is the lock | tests with vctl and by hand: sleep, wake, hot-plug |
| T4 | U | Unlocking its own session: telling the server to unlock (bus, D-Bus edge, vctl), injecting keys, drawing over the lock | The server unlocks **only** on vdm's yes to the request the lock dialog sent; no bus, D-Bus or control-socket command unlocks; while locked the control socket refuses everything but reports; input goes to the lock only; nothing is drawn above it | tests: each command while locked |
| T5 | U | Reading the lock's password as it is typed (keylogging the server, a look-alike dialog) | Keys while locked reach no client (native, Wayland, X11); the dialog is drawn by the server, and no window can stand above it | tests: a client listening while locked hears nothing |
| T6 | U | Attaching a debugger to the server (or the greeter) and unlocking in memory | The server and the greeter forbid it: `PR_SET_DUMPABLE` 0 on Linux, `procctl(PROC_TRACE_CTL_DISABLE_EXEC)` on FreeBSD; this stops an opportunist, not a program that controls the user's files (it can change the login shell's startup files and so the next server): §7.4, review I1 | a test that ptrace is refused |
| T7 | U, L | Using the lock socket to guess **another** account's password | vdm checks only the caller's own account, by the uid the kernel gives for the socket (`SO_PEERCRED` on Linux, `getpeereid` on FreeBSD), never a name sent in the request | a test from a second account |
| T8 | U | Using the lock socket to guess **its own** user's password faster than `su` allows | The same growing pause per account as T1, kept by vdm; so it gains nothing over `su`/`sudo` attempts it can already make | a test of the pause |
| T9 | L | Reaching the greeter's socket (start sessions, power off) | The greeter socket is in a directory only root and `_vdm` may enter, and vdm checks the peer is `_vdm` | a test from another account |
| T10 | G | A compromised greeter collects passwords as they are typed | Accepted in part, as for every login screen: whatever draws the login sees the keys. Contained: `_vdm` owns nothing, starts nothing, cannot reach users' files; vdm starts the greeter fresh after each login; it is the same well-tested server code as the session | the review; the security pass |
| T11 | G | A compromised greeter starts a session without a password, or as another user | It cannot: vdm decides from PAM's answer, which the greeter only relays words into; vdm starts sessions, the greeter has no way to | the review; a fuzz test of the greeter socket |
| T12 | U, L | vdm (root) tricked through files a user controls: a symbolic link in the runtime directory, a crafted session file, a user's picture | vdm reads no file a user can write; it makes the runtime directory with mkdir (failing if anything is there), checks it, and removes it without following links; the session list comes from system directories only; users' pictures are not read by vdm (initials in this milestone) | tests with a planted link |
| T13 | U, G | vdm's own parser fed bad messages: crashes, overflows | Rust; a strict small format with lengths checked first (as the bus's); fuzz tests; no `unsafe` but the PAM and socket calls | fuzz tests |
| T14 | P | Automatic login undoing disk encryption | Never offered with any encrypted file system mounted (LUKS/dm-crypt on Linux; GELI and ZFS encryption on FreeBSD); if it cannot tell, it is not offered; never with more than one person on the machine | tests of the detection |
| T15 | P | Restart or Shut down from the greeter, ending other people's sessions | Accepted for the machine's own power, as its power button; with others logged in, vdm asks first and names them | by hand |
| T16 | all | A password left in memory or a log | Typed passwords are kept in buffers that are wiped when done, in the greeter, the server and vdm; never logged, never in an error message; core dumps of vdm, the greeter and the server off | the review; the security pass |
| T17 | U | Starting a session's programs with an environment vdm inherited (root's) | vdm builds the session's environment from nothing: HOME, USER, LOGNAME, SHELL, PATH, LANG and what PAM gives; the session is started through the user's login shell as the user | a test of the environment |
| T18 | P | The lock that is not one: Super+L when vdm is not reachable | Super+L then says the screen cannot be locked and leaves it visibly unlocked; idle and sleep do the same, with a notification | a test with vdm stopped |

### Rules for the code

- vdm is small: PAM, the two sockets, starting and ending sessions, power.
  Anything that can run unprivileged does (drawing, fonts, input, the list
  of sessions shown).
- Every request vdm takes is in a fixed, versioned format, its lengths
  checked before anything is read; anything unknown ends the connection.
- No `unsafe` outside the calls to PAM, the socket peer, `setuid`/
  `setgroups`/`initgroups`, `ptrace` protection and the VT; each such call
  commented with what it relies on.
- PAM is linked from the system (libpam on Linux, OpenPAM on FreeBSD)
  through our own declarations: it is the operating system's interface for
  logging in, as DRM and evdev are (Dave, 2026-11-16: not borrowing).
- Passwords: one buffer type that wipes itself, never `String`, never
  formatted into text; no `Debug` for it.
- Every login, failure, lock, unlock failure, session start and end goes to
  the system log (user and time, never what was typed): §7.2's audit log.

### What is accepted, and why

- **A program running as the user** can end its own session (it can kill
  its server), read its files, and watch what the user does while
  **unlocked**: §7.4. The lock protects against a person at the machine,
  not against malware already inside the session.
- **Anyone at the greeter** sees the list of the machine's people (Dave's
  answer 1 a), and can restart or shut the machine down (T15).
- **Root, and kernel bugs,** are out of reach of any display manager.
- **Cold boot, DMA over Thunderbolt, a keyboard that logs**: physical
  attacks on the hardware are out of scope.
- **The greeter sees typed passwords** (T10): contained, not prevented.
- **A text console the user is logged in on** gets around the lock by switching to it (review M5; Dave: VT switching stays); every other console asks for its own login.

### Open points, settled (Dave, 2026-11-16: "1 yes, 2 refuse, 3 yes")

1. **PAM is an operating system interface**, not borrowing: linked from the
   system (libpam, OpenPAM) through our own declarations, as DRM and evdev.
2. **No paste** into the password field, at the lock or the greeter: only
   keys typed at the machine (and the PAM prompts' own devices) answer.
3. **Initials** at the greeter in this milestone; users' pictures later,
   from a copy vdm keeps itself, never read from a home by root.

## Design review, 2026-11-16: findings and how each is handled

An independent reviewer read this design, whitepaper §7, the session
script and the server and seat code (nothing built or changed). Every
finding is accepted; the most serious were checked against the code here
(the session script's restart loop, the modifiers sent to the focused
native window, client cursor surfaces drawn on top). Dave's decisions on
the three it raised: "1 a, 2 a, 3 a".

### Critical

- **C1. A server restarted by the session script comes back unlocked**
  (tools/vergence-session restarts on status 75 and on a quick failure).
  *Handled:* a locked server tells vdm; a session that was locked never
  restarts (the script asks vdm before each restart), and if one does start
  again it starts **locked** because vdm says the session was. A test
  crashes the server while locked.
- **C2. Step 1 alone would be a lock that is not one** for a session
  started from a text console: a crash a person at the machine can cause
  (a USB device, a monitor's EDID, memory) returns to the logged-in shell.
  *Handled (Dave: "1 a"):* the server locks only when the session cannot
  fall back to a shell: started by vdm, or by `exec vergence` (the login's
  own process, checked by the server: no shell above it). Otherwise Super+L
  says the screen cannot be locked and how to start so that it can (T18).
  Hubble and kepler start with `exec vergence` until the greeter exists.
- **C3. The greeter could start programs**: vergence-server starts them
  from about twenty places (Force Quit's keys, the launcher, the dock,
  actions, menus, the portal, autostart, session restore, the D-Bus edge).
  *Handled:* a **greeter mode** in which `crate::spawn` refuses everything
  but vergence-greeter, and the launcher, dock, drawer, tiles, session
  restore, autostart, Xwayland, the portal, the D-Bus edge and the control
  socket are off. A test goes through every key and pointer path in greeter
  mode.
- **C4. Sleep the server did not start is not locked** (the lid, the power
  button, `zzz`, `systemctl suspend`, `acpiconf`), and after resume a CRTC
  may still show the last frame. *Handled:* vdm locks every session before
  any sleep and waits (with a time limit) until each server says the lock
  is **presented**: on Fedora a logind sleep-delay inhibitor (and logind's
  Lock/LockedHint followed); on Void an /etc/zzz.d/suspend hook; on FreeBSD
  /etc/rc.suspend. Both ask vdm.
- **C5. PAM in the root daemon itself**: pam_systemd would move vdm into the
  user's session scope; pam_fprintd and pam_u2f block; NSS can hang; copies
  of the password inside modules cannot be wiped. *Handled:* every PAM
  transaction runs in **its own forked worker** with a time limit, killed if
  it overruns; a login's worker opens the PAM session and forks the user's
  processes (as greetd and GDM do); at most one request in flight per uid.
- **C6. FreeBSD may not revoke input devices from a session that ended**
  (seatd's revoke may do nothing there; seat descriptors may be inherited by
  apps), so a leftover program could read keys typed at the greeter or in
  the next session. *Handled:* verified on kepler before step 3; every seat
  descriptor close-on-exec; at a session's end vdm kills all of it through a
  reaper (`procctl(PROC_REAP_ACQUIRE)` on FreeBSD, `PR_SET_CHILD_SUBREAPER`
  on Linux) before the greeter starts.

### Important

- **I1. T6 cannot hold against U**: the session starts through the user's
  login shell, whose files U can change (LD_PRELOAD, PATH). *Handled:* T6
  is reworded as §7.4: the ptrace block stops an opportunist, not a program
  that controls the user's files. FreeBSD uses `PROC_TRACE_CTL_DISABLE_EXEC`
  (plain DISABLE would follow every app the server starts); `RLIMIT_CORE` 0
  as well; password buffers locked in memory (`mlock`) and kept out of core
  files (MADV_DONTDUMP, MADV_NOCORE).
- **I2. PAM modules read users' files as root** (pam_u2f's
  ~/.config/Yubico/u2f_keys, pam_env, pam_mkhomedir). *Handled:* our own
  /etc/pam.d/vdm, vdm-greeter and vdm-unlock (pam_u2f with an authfile in
  /etc, or openasuser); vdm refuses to run if they are missing rather than
  fall back to "other". T12 says PAM's reads are governed by those files.
- **I3. XDG_RUNTIME_DIR.** *Handled:* pam_xdg on FreeBSD (/var/run/xdg/<user>,
  today's path) where it is there, logind's on Fedora, vdm's own only on
  Void; removed only when the user's **last** session ends (an SSH session
  may still use it); vctl and the session script follow the same rule.
- **I4. logind's needs.** *Handled:* PAM_TTY, XDG_SEAT, XDG_VTNR and
  XDG_SESSION_CLASS set before the session opens; the greeter has a logind
  session of its own (class greeter, PAM service vdm-greeter) so libseat
  gives `_vdm` the devices; **each user session runs on a VT of its own**
  (as GDM does), which also ends the handover race between the greeter's
  server and the user's.
- **I5. FreeBSD's setuid helpers are run by the seat group** (video), which
  `_vdm` would join. *Handled:* sound-default and media-eject get a group of
  their own (`vergence`), the user is in it and `_vdm` is not.
- **I6. The lock's input has holes in today's code**: modifiers go to the
  focused native window whatever the seat decides; Escape for the dock is
  handled before the seat; Ctrl+Alt+Backspace, Force Quit, menus and Alt+Tab
  are decided before any lock check could be; new windows take focus; a
  client's own cursor surface is drawn above everything. *Handled:* **the
  lock is a seat state checked first** in keys, pointer and touch (only
  volume, brightness and VT keys pass); locking ends grabs, menus, Alt+Tab
  and drags, clears every client's keyboard and pointer focus, and goes back
  to the desktop's own cursor; while locked only the lock and the wallpaper
  are drawn (no windows under the frost, no direct scanout of a client).
- **I7. The idle timer stops while another VT is shown**, so coming back
  could show the desktop. *Handled:* on returning to the session's VT, if
  the idle time is past "lock after", the lock is drawn before the first
  frame.
- **I8. vdm going away while a session is locked.** *Handled:* the session
  stays locked, the dialog says it is waiting for vdm and reconnects;
  sessions outlive a vdm restart (their workers do not depend on it).
- **I9. Unlock and PAM.** *Handled:* unlock runs `pam_authenticate`, and
  `pam_acct_mgmt` tolerating an expired password (so it never shuts a user
  out of a running session); step 1 already speaks the whole conversation
  (prompts with and without echo, messages), so a stack that asks for a
  YubiKey can unlock; a fingerprint runs as a second PAM service beside the
  password (as GDM), so fprintd's wait never holds the field.
- **I10. The greeter's state and abuse.** *Handled:* `_vdm` has a home it
  cannot write and its state on a tmpfs wiped at each start; vdm refuses
  root and system accounts at login (Dave: "3 a"; including FreeBSD's
  `toor`, which shares uid 0, and nologin shells); a greeter that keeps
  failing is backed off and after a few tries leaves a text console; a
  greeter gone in the middle of PAM has its worker killed.

### Minor

- **M1.** The pause after failures is per uid, kept across connections and
  vdm restarts; requests in parallel do not get around it.
- **M2.** "vdm asks first" before a restart with others logged in is the
  greeter's courtesy, not a control; the session chosen is an id from vdm's
  own list, and its Exec line is read after vdm has dropped to the user.
- **M3.** The control socket's blocking read on the server's main loop
  (2 s) could delay presenting the lock; it moves off the main loop or is
  closed while locking.
- **M4.** While locked, the portal's file chooser and OpenURI and the bus's
  ContextMenu and Activate are deferred until unlock.
- **M5.** Another VT where the user is logged in at a text console gets
  around the lock (Dave: "2 a": VT switching stays, every other console
  asks for its own login, and this is said plainly in the accepted list);
  Void's `kernel.sysrq` is set deliberately.
- **M6.** Any LUKS, ZFS or GELI encryption counts, even unlocked by a TPM;
  checked at every boot, not only when automatic login is turned on.
- **M7.** Elevation, later, gets its own socket and PAM service, never the
  lock socket any local user can reach.

### Found sound by the review

The peer uid from SO_PEERCRED and getpeereid (as the control socket uses
it); FreeBSD's pam_unix checking any user's password as root; screens off
clearing the CRTCs; keys that wake the screens swallowed; the control
socket off on installed sessions and the bus checking its peer's uid; no
screencopy, virtual-keyboard or keyboard-grab protocol offered to clients;
Linux's PR_SET_DUMPABLE blocking same-uid ptrace.

## Steps

(Revised after the design review.)

1. **The lock screen and the first vdm.** The lock as a seat state checked
   first (I6), drawn and presented before sleep and on every screen and VT
   return (C4 by the server's own sleep and I7), Super+L, idle, a named
   action, no paste; vdm as a system service doing only the unlock check, a
   forked PAM worker per request with a time limit (C5), the whole PAM
   conversation (I9), the pause per uid (M1), our own /etc/pam.d/vdm-unlock
   (I2), the lock socket by peer uid (T7); the server locks only when the
   session cannot fall back to a shell (C2) and never restarts while locked
   (C1); ptrace and core protection, password buffers that wipe and stay out
   of swap and core files (I1, T16). Started with `exec vergence` on Hubble
   and kepler.
2. **Sleep from anywhere** (C4): logind's inhibitor on Fedora, zzz.d on
   Void, rc.suspend on FreeBSD, each waiting for the lock to be presented.
3. **Before the greeter:** C6 checked on kepler (input revoked after a
   session); the setuid helpers' own group (I5); `_vdm` with a home it
   cannot write and its state on a tmpfs (I10).
4. **The greeter:** greeter mode (C3), `_vdm`'s logind session (I4),
   vergence-greeter, the greeter socket (T9), refusing root and system
   accounts (I10), backing off a failing greeter.
5. **Starting and ending sessions** on a VT of their own (I4): the login
   worker opening the PAM session (C5), environment from nothing (T17),
   XDG_RUNTIME_DIR by platform (I3), the reaper at the end (C6).
6. Sleep, Restart, Shut down; text size and keyboard layout; the session
   list (M2).
7. Service files and `install.sh --vdm` for runit, rc.d, systemd (vdm
   itself always enabled: question 3); Techbench's GDM replaced (2 b).
8. Settings: Lock Screen (lock after, lock on sleep) and Login (automatic
   login, by 4 a and M6).
9. Security pass over vdm, the sockets, the lock and the greeter; fuzz tests
   of both protocols; whitepaper (VDM in §6.9 and §7) and its PDF.

## Acceptance

- Hubble and kepler boot to the greeter, right-sized on every monitor; a
  password logs in to the usual session; logging out comes back to it.
- Super+L locks; the password unlocks; nothing of a window shows while
  locked, in a screenshot neither; idle and sleep lock.
- A YubiKey (pam_u2f) logs in where PAM asks for it.
- A wrong password is refused with a growing pause.
- Restart and Shut down from the greeter.
- The console start and GDM still work where VDM is not enabled.
- Tests green on all three; the security pass's findings fixed.

## Questions

1. **Who logs in:** (a) a list of the machine's people (name and picture;
   one person: just them), with "Other…" to type a name; (b) a name field
   only. *Suggested: (a).*
2. **Fedora (Techbench):** (a) GDM stays, VDM is tried there by hand; (b)
   VDM replaces GDM there too. *Suggested: (a) for this milestone.*
3. **The lock screen without vdm** (started from a console, or under GDM):
   (a) it locks and checks the password through vdm when vdm is running,
   and without it offers only to blank the screens; (b) a small setuid
   helper of ours checks the password everywhere. *Suggested: (a).*
4. **Automatic login:** (a) offered, off by default, never on a machine
   with more than one person; (b) not offered. *Suggested: (a).*
5. **Other sessions** at the greeter: (a) Vergence only; (b) also the
   Wayland sessions installed (/usr/share/wayland-sessions); (c) Wayland
   and X11 sessions. *Suggested: (b).*
6. **Order:** (a) the lock screen first (step 1, used every day), then the
   login screen; (b) the login screen first. *Suggested: (a).*
