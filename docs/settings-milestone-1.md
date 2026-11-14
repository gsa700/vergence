# Settings milestone 1: the sidebar in groups, Accessibility, Wi-Fi and networks

Drafted 2026-11-14 from docs/next-settings.md (agreed with Dave the same
morning: the sidebar grouped by type as GNOME and macOS do, an Accessibility
page, Wi-Fi by wpa_supplicant ("wpa_supplicant is fine"), no Bluetooth yet
("skip BlueZ for now"), the iPhone by cable and by its hotspot, page icons
of our own, an Advanced part per page) and his screenshot of macOS's System
Settings (docs/reference/macos-system-settings-2026-11-14.png). Comes after
window styles milestone 1, which made the settings app a sidebar window.
**Not accepted yet: questions at the end.**

## Where things stand (read 2026-11-14, 45b2a4c)

- **The settings app** (crates/vergence-settings) is a sidebar window
  (window styles milestone 1, step 5): portrait, its sidebar and page under
  the frosted bar, back and forward and the page's name in the bar. Its
  sidebar is one flat list of eleven pages (Appearance, Theme and fonts,
  Windows, Dock, Keyboard, Mouse and touchpad, Sound, Energy, Screens (or
  each screen), About), text only, no search.
- **Accessibility settings are scattered**: High contrast, Reduce motion and
  Reduce transparency sit in Appearance's last group; text size in Theme and
  fonts.
- **Networks**: nothing in Vergence. Hubble (Void, the AX210 Wi-Fi 6E card
  as wlp4s0, the driver loaded): wpa_supplicant installed and configured as
  Void ships it (control socket in /run/wpa_supplicant for the `wheel`
  group, `update_config=1`), its service not enabled; dhcpcd runs on every
  interface (so an address comes once Wi-Fi joins); Dave's account is in
  `wheel`. kepler has no Wi-Fi card (wired em0); Techbench runs
  NetworkManager (wired). No usbmuxd anywhere.

## Design

### The sidebar in groups

A heading over each group, the pages under it with a small icon each (our
own, drawn as scenes: Question 3), and a search field at the top:

| Group | Pages |
|---|---|
| Connections | Wi-Fi, Network |
| Look | Appearance, Theme and fonts, Wallpaper |
| Desktop | Windows, Dock |
| Screens | Screens (the overview, or the one screen), each screen when there are several |
| Input | Keyboard, Mouse and touchpad |
| Sound | Sound |
| Accessibility | Accessibility |
| System | Energy, About |

- **Wallpaper** becomes a page of its own: one place for every screen's
  picture (the screen pages keep their scale and arrangement).
- **Search** (Question 4): typing filters the sidebar to the pages whose
  name or settings match ("frost" shows Appearance), Enter opens the first.
- Bluetooth, Notifications, Date and time, Users and the account card wait
  (docs/next-settings.md).

### Accessibility

One page: High contrast, Reduce motion, Reduce transparency (moved from
Appearance), Text size (moved from Theme and fonts; a copy stays there), and
the window keys' and snapping's notes pointing to Windows. Nothing new is
set: the same desktop.toml keys, so nothing a program reads changes.

### Advanced

A collapsed "Advanced" part at the end of a page, opened with a click
(remembered for the session), holding every other option and live status.
Wi-Fi's is the first: signal in dBm, channel and band, the access point's
address, the addresses the machine has. The Sound page's raw devices move
into one when this lands.

### Wi-Fi

- **Our own client of wpa_supplicant's control socket** (Dave:
  "wpa_supplicant is fine"): a crate, `vergence-net`, speaking its text
  protocol over a Unix datagram socket (STATUS, SCAN, SCAN_RESULTS,
  LIST_NETWORKS, ADD_NETWORK, SET_NETWORK, ENABLE_NETWORK, SELECT_NETWORK,
  REMOVE_NETWORK, SAVE_CONFIG, and ATTACH for its events: scans done,
  connected, disconnected, wrong password). The same on Void and FreeBSD;
  NetworkManager (Fedora) later, as PipeWire is for sound.
- **The page**: a switch for Wi-Fi (the interface up or down), the network in
  use with its signal, then the networks in range (strongest first, a lock
  for those with a password, the bars of their signal), rescanned every few
  seconds while the page is open. Choosing one asks for its password in the
  page (a field that never shows what is typed unless asked) and joins;
  wrong passwords say so. Known networks can be forgotten. The phone's
  hotspot is one of the networks.
- **No root**: the socket is the `wheel` group's (Void's and FreeBSD's
  default); the page says, with the command, when wpa_supplicant is not
  running or the account cannot reach it. Enabling the service once is a
  root step for install.sh or by hand (Question 1).
- **Addresses** come from dhcpcd, as for wired; the page shows them.

### Network

The machine's other connections: each wired interface and a phone's USB
tethering (Linux `ipheth`, FreeBSD `ipheth(4)`), by the names people use
("Ethernet", "iPhone (USB)"), connected or not, their addresses; read only
for now (DHCP is dhcpcd's). An iPhone by cable usually needs to be trusted
by the computer through usbmuxd before it hands out a connection (Question
2).

## Steps

1. **The sidebar in groups**: headings, the Accessibility page, the Wallpaper
   page, the search field; the page icons.
2. **The Advanced part**, as a widget of the settings app.
3. **vergence-net**: the wpa_supplicant client (commands, replies, events,
   scan results parsed, its security: lengths and characters checked), the
   interfaces and their addresses (getifaddrs, on both systems); tests
   against a fake wpa_supplicant socket.
4. **The Wi-Fi page**, on Hubble with wpa_supplicant enabled.
5. **The Network page**, with an iPhone by cable on Hubble (Dave's phone).
6. **Docs**: whitepaper §6.9's settings, README, CLAUDE.md, this plan's notes.

## Acceptance

1. Tests green on Hubble, kepler and Techbench.
2. The sidebar shows its groups with headings and icons; "frost" in the
   search leaves Appearance; every old page is still reached, and every
   setting moved to Accessibility or Wallpaper does what it did.
3. On Hubble: Wi-Fi lists the networks in range with their signal; joining
   one with its password connects and gets an address (dhcpcd); a wrong
   password says so; forgetting it removes it; turning Wi-Fi off and on
   works; the phone's hotspot joins the same way; Advanced shows the signal,
   channel and addresses.
4. On Hubble: the iPhone by cable appears on the Network page with an
   address (or the page says what it lacks).
5. On kepler (no Wi-Fi) the Wi-Fi page says there is no Wi-Fi device; on
   Techbench (NetworkManager) it says Wi-Fi is NetworkManager's for now.

## Security (whitepaper §7.3)

- The control socket is wpa_supplicant's own, its group's: the settings app
  gets no more than any program run by a member of `wheel` already has.
- Passwords go from the field straight to wpa_supplicant (SET_NETWORK psk),
  quoted and checked (length 8 to 63, printable), never logged or kept by
  the settings app; wpa_supplicant keeps them in its own file
  (`update_config`), root's to read.
- Everything read back (network names, scan results) is untrusted text:
  length limited, control characters dropped, drawn as plain text.

## Not in this milestone

- Bluetooth (no BlueZ: Dave, 2026-11-14), NetworkManager, VPN, static
  addresses, Notifications, Date and time, Users, the account card.
- A Wi-Fi tile on the dock (live tiles).

## Questions for Dave

1. **Enabling wpa_supplicant on Hubble.** (a) `sudo tools/install.sh` enables
   it where it is installed and not running (Void: the runit service; FreeBSD:
   `wlans_*` and the service in rc.conf, when a Wi-Fi card is there); (b) by
   hand, the page saying how. Proposed: **(a)**, printed by install.sh as it
   does it.
2. **usbmuxd for the iPhone by cable** (a daemon of other people's, so the rule
   says ask). (a) Install it on Hubble (Void's package) so the phone can be
   trusted and tether; (b) not now: the Network page shows the phone if it
   tethers without it, and says so if not. Proposed: **(b)** first, to see
   whether it is needed at all.
3. **The page icons.** (a) A small rounded square in a color per group with a
   white mark, as macOS's; (b) a plain line mark in the text's color.
   Proposed: **(a)**: it is what makes macOS's list quick to scan.
4. **Search.** (a) Now, over page names and the names of their settings;
   (b) later, with the launcher finding settings. Proposed: **(a)**: small,
   and the launcher can reuse it.
