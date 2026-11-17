# Menus milestone 1: one look, and where the session ends

*Planned and built 2026-11-16 night, after File Viewer's refinement (files
milestone 2) moved the desktop's icons to the bus v20 right-click menu and
"Exit Vergence…" went missing with the server's old desktop menu. Dave:
"that's not really the place for it. But where then?" A mock-up (a canvas
of three boards) settled it.*

## Decisions (Dave, 2026-11-16)

1. **The session ends from the launcher**, as from a start menu ("it's
   somewhat unavoidable with all the choices we've made that we will end up
   where many other desktops have"): a footer under the grid with the
   person's name, Lock and a power button; the power button opens the
   session dialog (Log Out on the right, Sleep, Restart and Shut Down apart
   on the left, as it already was since VDM step 6).
2. The power key and **Ctrl+Alt+Delete** open the same dialog; typing "log
   out" (or "shut down") in the launcher finds it too. "Exit Vergence…" is
   called **Log Out…** now.
3. **The desktop's right-click menu holds only what is about the desktop**:
   New Folder, Paste, Arrange By ▸, Open Desktop in File Viewer, Open in
   Terminal, Properties, then the server's Change wallpaper… and Display
   settings… (and desktop.toml's `[[menu.item]]` apps). No Settings…, no
   Log Out.
4. **One menu look for every right-click menu**, the mock-up's "black
   rounded design", light where the look is light: the server's (the
   desktop's, the dock's, a title bar's, a tab's, an app's own and its app
   button's) and the toolkit's in a window (a field's, a choice's list).

## Steps

1. The desktop's own lines in its v20 menu. **Done** (7ffe544): the
   desktop's menu marks their place (`window::MENU_DESKTOP_LINES`); the
   server puts its lines there (ids from `window::MENU_SERVER_IDS`) and
   carries them out itself (menu.rs `desktop_own_lines`, `menu_run`);
   trimmed to Change wallpaper… and Display settings… by decision 3.
2. The launcher's footer. **Done**: `Foot::Lock` (lock_request) and
   `Foot::Power` (session_dialog_open), lit under the pointer; the name is
   the first part of the full name in the password file, else the login.
   The grid gives up `FOOT_H` (46 points).
3. The keys. **Done**: the seat's `Key::Delete` and `Key::Power`
   (XF86PowerOff) give `KeyOut::SessionDialog` whatever has the keyboard,
   not while locked, not at the login screen. The system's own handling
   of the power key is held off while a session is registered with vdm
   (vergence-dm `powerkey`, Dave 2026-11-16 "do the power key"): logind's
   handle-power-key inhibitor (a `systemd-inhibit` or `elogind-inhibit`
   child); on Void's acpid, install.sh's guard in /etc/acpi/handler.sh
   that leaves the button alone while /run/vdm/power-key is there; on
   FreeBSD `hw.acpi.power_button_state` NONE (the old state kept in
   STATE_DIR/power-button-state, given back by a vdm that starts after one
   died holding it), and devd's button event sent to the servers as
   `FromVdm::PowerKey`, since the server never sees it as a key. At the
   login screen and the console the system's way stands.
4. The look. **Done**: vergence-look's `MENU_RADIUS` (10), `MENU_ROW_RADIUS`
   (6), `MENU_PAD` (6), `MENU_ROW` (28), `MENU_SEP` (9, its rule
   `MENU_SEP_INSET` from the sides), `MENU_TEXT`, and the dark menu ground
   near black (42, 44, 51). The server's old menu (menu.rs: the dock's, the
   launcher's right click, a title bar's layouts, a tab's) is one picture
   now, as the app menu is, solid, no frost; both draw through menu.rs
   `menu_ground` and `menu_separator`. The toolkit's field menu and choice
   list take the same measures and the server's colors (light or dark as the
   window), highlight in the menus' hover color.
5. The session dialog as the mock-up (Dave: "make ours match your mockup
   exactly": the power buttons were only 16 points from the others, no
   apart at all). **Done**: a 600 by 150 panel, 24 inside, corners 14; the
   heading at 18 points and "Your apps will be asked to close first." under
   it; buttons 34 high, each as wide as its word (14 either side, Log Out 16
   and semibold), 8 apart, Sleep, Restart and Shut Down from the left, Lock,
   Cancel and Log Out from the right, all the room between them.
6. The launcher as a start menu (Dave, 2026-11-16: the grid in the middle
   of the screen "feels disconnected"; of two mock-ups, B, a text menu,
   "pinned right to the button as would be the norm"). The dock stands on
   the bottom edge by default now (c05c284). **Done**: launcher.rs is the
   app menu's panel (appmenu.rs `app_menu_launcher`, no window of its own)
   with given lines and a search field (vergence-shell `Model::searchable`:
   typing finds an application in any submenu, each once): Vergence's own
   applications, the categories that have one as submenus (Other last), Lock
   Screen, Log Out…, each carried out by menu.rs (`Entry::Shelf`, `Lock`,
   `LogOut`). Above the dock's button on a dock at the bottom, held by its
   bottom as it grows; below it at the top; beside it at a side. The grid's
   footer (step 2) went with it: Lock Screen and Log Out… are its last
   lines. *Lost for now*: dragging an application from the launcher to the
   dock, its right click (Pin to the dock), and finding the named actions
   by typing (Sound settings, Mute...). Dave (2026-11-16): left out until
   real use shows they are missed.
