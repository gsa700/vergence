# Session milestone 1: back as it was, or fresh

Status: **agreed 2026-11-21** (Dave: "yes, add it to the plan, build it
where you think it makes sense"). Built before files milestone 3, because
that milestone's first step keeps FileView's tabs across restarts and
should keep them by these rules from the start. Small: four steps and the
checks.

This settles the question left open on 2026-11-21 at 02:46 ("i am still
not sure if the window position should be remembered. i found myself
thinking that is Reader as well"). Dave's answers, 2026-11-21 afternoon:

- "when restarting or the computer power failed or whatever, then bring it
  back JUST like it was, windows documents etc. scrolled to where they
  were in the document. This senario suggests the user is actively reading
  the document. But if I close it by choice and reopen it sometime later,
  I think it should open at the first page."
- "Settings is one where I would go the other way: during a session
  remember everything, so if I am trying to get a setting just right but
  have to close settings to do so, it should re-open on the same page and
  location as I closed it. Maybe have it reset if the computer restarts or
  a period of time has gone by?"
- "keep window handling as is, we might also offer this on a session tab
  in settings?"

## The rules

1. **Window places stay as they are.** Window memory (persistence tokens;
   per app, role and set of screens) is not changed by this milestone.
2. **Ended from outside, everything comes back as it was.** A logout, a
   restart, a power cut or a crash ends a program without the user closing
   its windows. At the next start, every window comes back with its
   documents and tabs, the one in front, and each document's place: the
   page and the scroll place within it, and the zoom. This applies to
   every app, Settings too. The server already starts the apps again
   (session.rs, `[session] restore`), and Reader and Edit already reopen
   their windows (keep.rs). What is new here is the place inside the
   document.
3. **Closed by the user, a document starts fresh.** A document or folder
   closed on purpose (its tab, its window, or Quit) opens at the top next
   time: a PDF at page 1, a folder at its first row. Its zoom stays as it
   was left, as a choice of how to read it rather than where. *Claude's call:
   say if the zoom should start fresh too.*
4. **Settings keeps its place for a while.** Settings closed by the user
   opens again on the same page, scrolled to the same place, for 30
   minutes by default. A new session (a restart, or a logout and login)
   always starts it at its front page. *Claude's call: a logout counts as a
   restart here. Say if only a real restart should.*
5. **Unsaved work is never dropped.** Edit keeps unsaved text across
   restarts whatever the switches below say: they decide what is
   reopened, never what is lost. Edit's kept text is still offered back.

## Settings > Session

A new page of its own in the System group. The Session group now on the
Dock page moves here.

- **When Vergence starts:** "Reopen the apps that were open" or "Start with
  no apps open" (today's `[session] restore`, moved, not changed).
- **Return to where I was in documents** (switch, on; under the one above,
  dimmed when it is off): rule 2's places. Off: the windows and documents
  come back, at their top. `[session] places`.
- **Settings remembers its page for:** Off, 10 minutes, 30 minutes, 1 hour,
  Until the next session. `[session] settings-keep` in minutes (0 is off,
  -1 is until the next session; default 30).
- A line pointing to **Windows > Window places**, where window places are
  forgotten (places.rs). It is not repeated here.

When "Start with no apps open" is chosen, the apps also forget the windows
they kept (Reader's and FileView's kept windows are not reopened when the
app is next started by hand), except Edit's unsaved work (rule 5).

## Steps

1. **Groundwork.** The server gives each session an id
   (`VERGENCE_SESSION`, random, in the environment of everything it
   starts), so an app can tell a new session from the same one. The toolkit
   can read and set a scroll's place by the scroll's name
   (`Window::scroll_place`/`set_scroll_place`, in reference units, kept to
   the content's size). A small shared module (vergence-types `kept`) holds
   the owner-only state file pattern that Reader's and Edit's keep.rs both
   write by hand, and reads `[session]`.
2. **Reader.** The places kept in reader-session grow from the page to the
   page plus the place within it, and the zoom, per tab. A start from the
   kept session goes there. An open by the user (Ctrl+O, a file handed
   over, Recent) starts at page 1 with the document's last zoom.
   reader-places stops giving the page and keeps only the zoom. `[session]
   places = false` reopens at the top. Tests: kept and parsed, a fresh
   open at page 1, a kept start at its page.
3. **Settings.** Kept like Reader's windows when ended from outside (page,
   the app page if one is open, the scroll place). Closed by the user: the
   same, with the time and `VERGENCE_SESSION`, used at the next open if it
   is within `settings-keep` and the same session. Tests for each case,
   the clock given.
4. **Settings > Session.** The page above; the Session group leaves the
   Dock page; search words (session, restore, reopen, restart, remember).
5. **Checks.** Tests on all four; a display-suite scenario (the Session
   page); agent-check through it; on kepler, a session ended hard (the
   server killed) with Reader at page 40 of a document and Settings open on
   Mouse, both back as they were after the next login; Reader's tab closed
   and opened again at page 1; Settings closed and opened within the time
   (same page) and after a new login (front page). Then Dave's hands-on pass.

## Not in it

Edit's caret on a file closed and opened again (it opens at the top
already); Close-Up (pictures have no place inside them); the terminal's
scrollback across restarts; FileView, which follows these rules in files
milestone 3 (its kept tabs, step 1; its folders' scroll places, step 2).
