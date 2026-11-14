# Next: the settings app grouped, and Wi-Fi (notes for the plan)

Agreed with Dave on 2026-11-14, after windows milestone 1 ("finish windows
first"). To be drafted as a settings milestone plan for him to accept.

Dave: "we are building it as we go so it's becoming disorganized and is
missing some needed pages like wifi bluetooth etc. wifi would be good for
hubble and would make it portable so we could try adding the 43" 4k 144 hz
tv for testing"; "I noticed in both gnome and macos that pages are grouped
by type ... what do you think?"; Advanced options still wanted, "what form
that takes we will have to see".

## The sidebar in groups (proposed, Dave agreed to grouping)

| Group | Pages (new marked *) |
|---|---|
| Connections | Wi-Fi*, Network* (wired, addresses), Bluetooth* (later) |
| Look | Appearance, Theme and fonts, Wallpaper* (moved from the screen pages) |
| Desktop | Windows, Dock, Notifications* (later) |
| Screens | the overview, each screen |
| Input | Keyboard, Mouse and touchpad, Touchscreen* |
| Sound | Sound |
| Accessibility | one page*: High contrast, Reduce motion, Reduce transparency, text size (scattered today) |
| System | Energy, Date and time*, Users* (later), About |

Advanced: a collapsed "Advanced" part at the end of each page, every other
option and live status (Wi-Fi: signal, channel, addresses). Its form to be
seen once a few pages have one (whitepaper §6.9, layers, 2026-11-13).

## Wi-Fi

- Hubble (Mobilenode): Intel AX210 Wi-Fi 6E, wlp4s0, driver loaded, not
  connected; wpa_supplicant and wpa_cli installed, not running as a service;
  Bluetooth radio hci0 present. kepler (M920q) has no Wi-Fi card. Techbench:
  NetworkManager and BlueZ, wired.
- **Decided (Dave, 2026-11-14): "wpa_supplicant is fine"**: our own client of
  wpa_supplicant's control socket, one back end for Void and FreeBSD;
  NetworkManager (Fedora) later, as PipeWire for sound. The control socket's
  access (group, or a small root helper as sound-default is) to be settled
  in the plan.
- **Bluetooth: "skip BlueZ for now"**: no BlueZ on Hubble; the page waits.

## The iPhone: by cable and by its hotspot (Dave, 2026-11-14)

"make sure it will take a direct cable connection from my iphone as well as
wifi to it's hotspot". By Wi-Fi it is one more network in the list. By cable
the phone's USB tethering appears as a wired-style interface (Linux `ipheth`,
FreeBSD `ipheth(4)`): the Network page lists it as a connection (the phone's
name, connected or not, its address) and asks for its address by DHCP as it
does for wired. On Linux the phone usually wants usbmuxd running (Apple's USB
pairing daemon) before it hands out a connection: borrowing a daemon, so a
question for Dave when the plan is drafted. To check on each machine with
Dave's phone (Control already uses it this way, over USB).

## The settings window's shape (Dave, 2026-11-14, from macOS's System Settings)

Dave's screenshot, docs/reference/macos-system-settings-2026-11-14.png ("shows
what should be the default layout"; "the titlebar on the right side is using
the translucent effect as we do on terminal so we should too, that is a good
place for it"). Earlier the same morning: settings needs no maximize and no
tabs; "model it like macos does without the hardcoded size limit, or with a
limited size allowance to preserve the layout".

- Portrait: about 830 x 1100 points. Opens about 840 wide and as tall as the
  screen allows, up to about 1100; width 760 to 1100, height free, no
  maximize; window memory after that.
- No bar across the whole window: the sidebar runs to the top edge, the
  window buttons on it, search under them; then (later) an account card;
  the grouped pages, each with a small icon (question for Dave).
- The page's own bar over the right side: back and forward, then the page's
  title; translucent, the page scrolling up under it (our frost, as the
  terminal's content under its bar). Built as a "sidebar" window style in the
  window styles milestone: the server's frosted bar over the whole top, the
  sidebar drawn up under it, the title placed over the page; back and
  forward need the bar to carry a program's own buttons (protocol).
- Pages: section headings over rounded groups of rows, as ours already are.
