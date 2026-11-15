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

**Tried 2026-11-14 on Hubble, parked** (Dave: "3, park the cable for now"):
without usbmuxd the phone stays in USB configuration 1; switched to 4 by
hand, ipheth makes its interface but it never gets a link, and the phone
asks "Trust" again on every reconnect, because the computer's half of the
pairing (keys, a stored pairing record, a session held open) is missing.
Later: Void's usbmuxd with libimobiledevice (borrowing: ask), or our own
USB mux and pairing (likely a TLS library). The hotspot by Wi-Fi works.

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

## Answers, 2026-11-14 (Dave: "yes to the rest")

- Order: window styles next (size limits, the plain bar, dialogs, palettes,
  the sidebar style), then this settings milestone.
- Page icons in the sidebar: yes, a set of our own (about 16).
- An account card at the top of the sidebar: yes, once there is a Users page.
- Applications' own settings (decided 2026-11-10, whitepaper §6.10 since
  Draft 0.13): after window styles, the toolkit's preferences window with
  inherit-or-override rows, the terminal first.
- Later (Dave: "we might need a login manager at some point depending on how
  much work that is"): a lock screen first (none today: an unattended session
  is open to anyone at it), then a login manager on the same password code
  (PAM on all three systems; part runs as root, so a narrow design and a
  security review). A milestone or two.

## From Dave, 2026-11-14 afternoon (for a later settings milestone)

- **Ethernet like Wi-Fi**: the Network page's cable with its link speed and
  duplex (Linux /sys, FreeBSD the interface's media), live traffic in and out
  with a chart, as the Wi-Fi page has.
- **Resolution and refresh rate** on each screen's page, from the modes the
  monitor offers (the server takes the preferred mode today); and beside
  Scale, the resolution it comes to ("Looks like 1280 x 720, native 3840 x
  2160").
- **A UPS page** of its own (local UPS by USB only), and **low-battery
  actions** were offered and deferred ("not yet").
- **Forget an app's window places** (Dave, 2026-11-14: "a reset if things get
  messy"): in the app's dock menu ("Forget window places") and on the Windows
  page (apps with saved places, each with Forget, and Forget all), clearing
  its entries in window memory for every setup.
- **Confirmations** (Dave, 2026-11-14): a dialog before forgetting an app's
  window places, and before closing a terminal window with several tabs
  open ("Close 3 tabs?"), on window styles milestone 1's dialogs.
- **Energy on battery and on power** (Dave, 2026-11-14): separate settings
  (screens off, sleep) while on battery and while on mains power, and later
  while on a UPS's battery; vergence-power already tells which.
- **Same physical size on every screen** (Dave, 2026-11-14: "Make windows on
  all monitors the same physical size", applying the design theory across
  whatever screens are installed or memorized): a switch on the Screens
  overview that uses each screen's exact calculated scale (Contract 1's
  reference unit; the 14" at 3.28, not snapped to 300%) for every screen,
  connected or remembered, so a window keeps its size in millimeters moving
  between them; off (today): whole-number scales where close, for sharpness.
- **Scroll bars under the title bar** (Dave, 2026-11-14 night: "next time we
  work on Settings, we need to fix the scrollbars - they go under the
  titlebar"): the settings app's scroll bars (sidebar and page) run up
  under the frosted bar; they should start below it (the window draws under
  its bar, `Theme::top`: the scroll area's bar must begin at that inset).
