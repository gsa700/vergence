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
   derickson") and **who** asks (read from the kernel: root's installed
   System Settings by its name, any other program by its file's path; a
   hint, the action's words being what the person decides on); the password goes into the server's locked,
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

## Progress

- **Steps 1-3 built** (2026-11-16, 9aef151, 47ee773): vergence-dm's
  elevate.rs (the actions, their checks and words; `request` for
  programs), proto's Act/Answering/Decline and Ask/Withdrawn, vdm's elevate
  socket (0666; only an administrator in wheel with a console session may
  ask; one request per person; 120 s; a decline makes the next wait 10 s;
  the check in a forked worker with PAM service vdm-elevate and the lock's
  pause; the server's Answering accepted only from a process on a console
  named vergence-server), the server's elevate.rs (the prompt, drawn and
  answered as the lock is), the Login page's Turn On…, Turn Off… and the
  login screen's switch. Tests 676/676/669.
- **Seen on kepler** (Dave, 15:55-15:57): Turn On… approved, Cancel
  declined, a wrong password refused with the pause, Turn Off… approved;
  each logged by vdm.
- **Step 4: an independent review of the code** (2026-11-16; given the
  code and this plan): 11 findings, none critical, all fixed:
  1. *A program's own cursor could be drawn over the prompt* (covering its
     words while the password went into the real field): the prompt takes
     the keyboard and the pointer from every program when it comes up, the
     desktop's cursor only while it (or the log out dialog) shows, the
     keyboard given back when it closes.
  2. *The "who" line was the program's to choose* (its process name): who
     asks is read before anything the program sends, from its program
     file (Linux /proc/<pid>/exe, FreeBSD kern.proc.pathname); "System
     Settings" only for root's installed vergence-settings, anything else
     "A program (its path)". The words of the action are what the person
     decides on; the name is a hint (this plan said otherwise before).
  3. *The asker need not be at the machine* (an ssh login or cron job of
     the person's could raise the prompt): it must run below one of the
     person's registered servers (their pids kept at registration) or the
     worker vdm runs their session in.
  4. *Keys typed elsewhere could land in a prompt that just came up*, and a
     program could raise one every 10 s: the prompt takes nothing for
     750 ms; after a decline or a prompt left to run out the next request
     waits 10 s, then 20, 40 ... up to 5 minutes, a yes clearing it.
  5. *Answering and declining were weakly tied to the prompt* (a
     sequential id, a process name): a prompt's id is 128 random bits only
     vdm and the asked server know; the answer must come from a process on
     a console running root's installed vergence-server; the Ask goes to
     every server of the person's. The energy sleep command is started
     with no terminal (it could otherwise have registered as a server).
  6. *The check's worker kept vdm's connections*: it closes them.
  7. *A request outlived its program and its session*: it is withdrawn when
     the asking program goes away or the person's last server does.
  8. *The control socket could type into the prompt*: it refuses input
     while the prompt shows.
  9. *A missing /etc/pam.d/vdm-elevate* would have fallen back to PAM's
     "other": requests are refused while it is missing; uninstall removes it.
  10. *A program's name could forge log lines*: the name shown and logged
      is cleaned.
  11. Smaller: Super+L locks while the prompt shows; the client waits up
      to 260 s (120 s at the prompt and a check after it).
  Found sound: the parsers, the actions and their checks, the files'
  places (root's 0700 state directory), the password being the asker's own
  and answered only by the same account, ids spent after a yes, the
  pause, the admin check, the lock in front of the prompt, the server's
  dialing and the Secret.

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
