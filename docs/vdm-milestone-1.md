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
| T6 | U | Attaching a debugger to the server (or the greeter) and unlocking in memory | The server and the greeter forbid it: `PR_SET_DUMPABLE` 0 on Linux, `procctl(PROC_TRACE_CTL)` on FreeBSD, so a process of the same user cannot ptrace them; what remains (root, a kernel bug) is §7.4's limit | a test that ptrace is refused |
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

### Open points, settled (Dave, 2026-11-16: "1 yes, 2 refuse, 3 yes")

1. **PAM is an operating system interface**, not borrowing: linked from the
   system (libpam, OpenPAM) through our own declarations, as DRM and evdev.
2. **No paste** into the password field, at the lock or the greeter: only
   keys typed at the machine (and the PAM prompts' own devices) answer.
3. **Initials** at the greeter in this milestone; users' pictures later,
   from a copy vdm keeps itself, never read from a home by root.

## Steps

1. The lock screen in the server (drawing, input, Super+L, idle and sleep,
   a named action), checking passwords through a small `vdm` that does only
   that at first. Usable at once on all three.
2. vdm runs the PAM conversation with its prompts (u2f, fingerprint, codes);
   tried with Dave's YubiKey where pam_u2f is set up.
3. The greeter: `_vdm`, vergence-server as the greeter's session, the
   greeter app, its socket.
4. Starting and ending the user's session from vdm (PAM session, groups,
   XDG_RUNTIME_DIR, seat handover on one VT).
5. Sleep, Restart, Shut down; text size and keyboard layout.
6. Service files and `install.sh --vdm` for runit, rc.d, systemd.
7. Settings: a Lock Screen part (lock after, lock on sleep, show the
   user's picture) and a Login part (autologin, by question 4).
8. Security pass over vdm, the sockets, the lock screen; fuzz tests of the
   protocol; whitepaper (VDM in §6.9 and §7) and its PDF.

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
