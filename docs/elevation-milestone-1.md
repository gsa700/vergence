# Elevation milestone 1: asking for permission, properly

*Drafted 2026-11-16 (Dave: "go ahead with elevation prompts"). Follows VDM
milestone 1 (docs/vdm-milestone-1.md), whose plan set it aside for a
milestone of its own: "the server draws the prompt (no app can draw a
look-alike or read it), PAM checks the administrator, vdm carries out one
named action with checked arguments, never an arbitrary command; a yes is
for that request only."*

**Decisions (Dave, 2026-11-16):** 1 a, only administrators (the wheel
group) may approve; 2 a, with their own password; 3 a, a yes is never
remembered, every action asks; 4 b, turning something off asks too
(automatic login's Turn Off, which skipped the prompt, goes through it).

## Why

Some things a person at the machine should be able to do from Settings are
the system's to do (whitepaper §3, principle 8) and need root: turning
automatic login on, the login screen at boot on or off, and next, Backup's
snapshots and the Disk utility's changes, later the system's own services
(sshd started or stopped with the system's tool). Today each is a command
for a terminal (`sudo vdm autologin on`). A button that does it without
asking would let any program running as the person do it too, silently.
So: a button that asks, in a way no program can fake, read or skip.

## How it works

1. **A program asks vdm** for one **named action** with its arguments, on
   a socket of its own (review M7: never the lock's socket): "autologin.on
   derickson". vdm learns who asks from the kernel (uid and pid), and
   refuses at once unless the asker has a session at one of the machine's
   consoles (as power actions since the code review), the action is one
   vdm knows, and its arguments pass that action's own checks.
2. **vdm asks the person's own server to show the prompt**, over the
   server's standing session connection. The server draws it as it draws
   the lock: every screen dimmed, a panel in front of everything, the seat
   taking every key and press until it is answered or cancelled; it says
   **what** is asked in vdm's words ("Turn on automatic login for
   derickson") and **who** asks (the program's name, from the kernel's pid,
   not from the program); the password goes into the server's locked,
   wiped memory as at the lock; no paste.
3. **vdm checks the answer through PAM** (its own service, `vdm-elevate`,
   in a forked worker with a time limit, as every check), with the same
   pause after a wrong answer that the lock uses (one count per person).
4. **On a yes, vdm does the action itself**, with its own code for that
   action, and tells the program the outcome. The yes is spent: the next
   action asks again (question 3).
5. **Every request is logged** (who, which program, what, yes or no, and
   the outcome), in the system log beside vdm's other lines.

What it never does: run a command a program names, take a path or text it
did not check, act for anyone not at the machine, or act without the
prompt (turning something *off* that only takes access away, as automatic
login's Turn Off today, may skip it: question 4).

## The first actions

- `autologin.on` (person): the rules of VDM milestone 1 still apply (one
  person, nothing encrypted); the Login page's Turn On.
- `login-screen.on`, `login-screen.off`: the login screen at boot (today
  `install.sh --vdm` / `--no-vdm`); the Login page.
- Later milestones add theirs: Backup (snapshot create/destroy, send),
  Disk utility, a service started or stopped with the system's own tool.

## Threats, and what answers each

- **A program draws a look-alike prompt** to collect the password: only
  the server draws on top of everything; the real prompt dims every
  screen and names the asking program; an app's window cannot cover the
  server's panel, and the seat gives no key to an app while it shows.
- **A program asks again and again** (to wear the person down into
  typing, or to keep the screen busy): one request at a time per person;
  a cancelled or refused request makes the next from the same program
  wait (the pause); the prompt always says which program it is.
- **A confused deputy**: vdm runs only named actions with arguments it
  checks itself; nothing the program sends reaches a shell or a path.
- **Someone over ssh, or another account**: only a person with a session
  at a console may ask, and the prompt appears on their own screen.
- **The password**: the lock's path exactly (server-drawn, Secret, PAM in
  vdm's worker); never in the asking program.
- **A request with no one watching**: the prompt waits at most two
  minutes, then is refused.

## Steps

1. vergence-dm: the elevate socket, the protocol's messages (an action
   request; vdm's ask to the server; the conversation), the named actions
   with their checks, the PAM service file (install.sh), the log.
2. The server: the prompt (lock.rs's dialog code, shared), the seat's
   lock state while it shows, the conversation with vdm.
3. Settings: the Login page's Turn On for automatic login and On/Off for
   the login screen at boot, through a small client library.
4. Tests (the protocol fuzzed; each action's argument checks), all three
   machines, then an independent review of the code as VDM's was.

## Questions

1. **Who may elevate:** (a) only members of the system's administrators'
   group (wheel on all three; derickson is in it on each), as sudo and
   macOS ask for an administrator; (b) anyone with a session at the
   machine.
2. **Whose password:** (a) the person's own (as sudo and macOS); (b) root's.
3. **A yes remembered:** (a) never, every action asks; (b) for five
   minutes, for the same program.
4. **Turning something off that only takes access away** (automatic login
   off, the login screen at boot off?): (a) no prompt; (b) a prompt too.
