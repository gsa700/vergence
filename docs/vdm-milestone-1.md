# VDM milestone 1: the login screen and the lock screen

**Drafted 2026-11-16** (Dave: "we will need (a) if we want to run on systems
without gnome or some other login manager as people expect that these
days"). The app side of the alternating order, after dock milestone 1. VDM,
the Vergence Display Manager, is on the apps roadmap (2026-11-15: "VDM
display manager + lock screen first; YubiKey/fingerprint via PAM/fprintd").

Questions for Dave are at the end; nothing is built until they are answered.
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
