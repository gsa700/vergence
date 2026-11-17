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
   not while locked, not at the login screen. *Not yet*: logind (Fedora,
   elogind on Void) and FreeBSD's ACPI act on the power key themselves (a
   shutdown or suspend by default); the server needs logind's
   handle-power-key inhibitor, and on FreeBSD `hw.acpi.power_button_state`
   set to NONE, before the key reaches the dialog alone.
4. The look. **Done**: vergence-look's `MENU_RADIUS` (10), `MENU_ROW_RADIUS`
   (6), `MENU_PAD` (6), `MENU_ROW` (28), `MENU_SEP` (9, its rule
   `MENU_SEP_INSET` from the sides), `MENU_TEXT`, and the dark menu ground
   near black (42, 44, 51). The server's old menu (menu.rs: the dock's, the
   launcher's right click, a title bar's layouts, a tab's) is one picture
   now, as the app menu is, solid, no frost; both draw through menu.rs
   `menu_ground` and `menu_separator`. The toolkit's field menu and choice
   list take the same measures and the server's colors (light or dark as the
   window), highlight in the menus' hover color.
