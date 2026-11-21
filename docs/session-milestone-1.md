# Session milestone 1: back as it was, or fresh

Status: **all steps in 2026-11-21**, a real logout passed on techbench the
same afternoon; Dave's longer hands-on pass still open (see Progress). Agreed 2026-11-21 (Dave: "yes,
add it to the plan, build it where you think it makes sense"). Built before files milestone 3, because
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

## Progress

All built 2026-11-21, the same afternoon as the plan. Tests pass on Hubble,
kepler, techbench and apollo; display-suite scenario 15 (tools/display-suite/15-session.sh) passes.

- **Step 1.** The server sets `VERGENCE_SESSION` (16 random hex digits,
  new for every server start) for everything it starts. The toolkit's
  named scrolls: `Program::scroll_to` (places to go to, taken at the next
  layout where the scroll with that `Element::name` appears) and
  `Program::scrolled` (told once for each change); `Ui::set_scroll_place`
  and `Ui::scroll_places` underneath. The shared module is
  `vergence_widgets::session` rather than a vergence-types `kept`
  (vergence-types is about file types, and every app has the toolkit):
  `[session]` keys, `id()`, `still_kept`, `ending`, `write_private`.
- **Found while building: an exit looked like the user closing every
  window.** Exit Vergence, Restart and Shut Down ask each window to close as
  its close button does (exit.rs), so Reader and Edit dropped the windows
  they kept, and nothing came back after a logout. **Bus v33**:
  `WinEvent::SessionEnding { window }`, sent by the server just before an
  exit's Close; `App::session_ending` / `Program::session_ending` in the
  toolkit; Reader, Edit and Settings keep what they have when the close
  follows within 15 seconds (`session::ending`; an exit that stopped over a
  question leaves later closes the user's). On the native road only.
- **Step 2, Reader.** reader-session keeps each tab's place ("at <page>
  <within> <zoom>" after its "tab" line; a file from before still reads),
  written with the page and zoom 0.7 s after scrolling stops
  (`Layout::place`, `go_to_place`). A start from the kept session goes
  back there; an open by the user starts at page 1, at the document's last
  zoom (reader-places now gives only the zoom). `[session] places = false`
  reopens at the top; `restore = false` forgets the kept windows.
- **Step 3, Settings** (crates/vergence-settings/src/keep.rs,
  ~/.local/state/vergence/settings-page): the page by its sidebar name (or
  the app whose page is open) and the scroll, written on a page change, and
  at most once a second while scrolling, with a last write a second after
  it stops; at a close by the user with the time and the session id.
  A `--page`/`--app` start is never overridden.
- **Step 4, Settings > Session** (System group; the Session group left the
  Dock page): When Vergence starts; Return to where I was in documents
  (shown only while apps are reopened); Remember the page after closing
  (Never, 10 minutes, 30 minutes, 1 hour, Until the next session); a button
  to the Windows page for window places. It uses the Apps icon for now.
- **Step 5, checks.** Scenario 15: Reader killed at a page of the
  whitepaper comes back at it; its tab closed (Ctrl+W) and the file opened
  again starts at page 1; Settings killed on the Windows page scrolled to
  its end comes back on it, scrolled; the Session page's rows. By hand in a
  headless session on Hubble: Settings closed with the close button
  reopens on its page within the time, at the front page for another session or
  after the time. **A real logout, 2026-11-21 ~14:49 on techbench** (bus v33
  installed on Hubble and techbench): Dave left Reader partway into two
  PDFs and Settings on a page, chose Exit Vergence (the server asked 5
  windows to close; all closed in 0.3 s), logged in again, and "both came
  back where they were" (reader-session kept pages 38 and 12; settings-page
  had no "closed" line). **Still open:** Dave's longer hands-on pass.
