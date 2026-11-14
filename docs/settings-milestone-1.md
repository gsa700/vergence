# Settings milestone 1: the sidebar in groups, Accessibility, Wi-Fi and networks

Drafted 2026-11-14 from docs/next-settings.md (agreed with Dave the same
morning: the sidebar grouped by type as GNOME and macOS do, an Accessibility
page, Wi-Fi by wpa_supplicant ("wpa_supplicant is fine"), no Bluetooth yet
("skip BlueZ for now"), the iPhone by cable and by its hotspot, page icons
of our own, an Advanced part per page) and his screenshot of macOS's System
Settings (docs/reference/macos-system-settings-2026-11-14.png). Comes after
window styles milestone 1, which made the settings app a sidebar window.
**All steps in 2026-11-14; acceptance below. Accepted 2026-11-14: Dave agreed to the Wi-Fi switch below and answered
the proposals ("yes to your proposals on the rest"); VPN parked ("lets just
park that for now").**

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
| Input | Keyboard, Mouse, Touchpad and Touchscreen (the last two once the machine has had one) |
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
- **When the radio is on** (Dave, 2026-11-14: "what if we enable wpa on all,
  but only turn it on when there is no hardline detected? Or, we ask?";
  agreed): a setting **Wi-Fi: Off / On / When no cable is connected**, the
  last the default. The server watches the wired links (a carrier on an
  Ethernet port, or a phone's USB tethering, by the device events it already
  takes for monitors), debounced by a few seconds, and brings the Wi-Fi
  interface up or down by the setting; down, the radio neither scans nor
  sends. The first time Wi-Fi comes on for want of a cable with no known
  network, a notification says so ("No network cable: Wi-Fi is on. Choose a
  network...") and opens the page; never again after. install.sh enables
  wpa_supplicant wherever there is a Wi-Fi card (not where there is none: a
  service with no card to manage fails and is restarted over and over).
- **The page**: the Wi-Fi setting above, the network in
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

## Progress

- **Step 1 in, but the Wallpaper page** (2026-11-14): the sidebar in groups
  (Look, Desktop, Screens, Input, Sound, Accessibility, System; Connections
  comes with steps 4 and 5), a heading over each, each page with its icon (a
  20-point rounded square in its group's color with a white mark drawn as a
  scene: half circle, A, window, dock, screen, keys, mouse, speaker, person,
  bolt, i), the search field at the top (pages by name, then by the words of
  their settings, a word's start; Enter opens the first; `search_match`
  tested); the Accessibility page (High contrast, Reduce transparency, Size,
  Reduce motion, a way to Theme and fonts), Appearance keeping a row that
  says their state and opens it. The sidebar is 210 points (the bar's back
  and forward follow). On Hubble (shots): the groups and icons; "frost" left
  Appearance, "contrast" left Accessibility and Enter opened it. The
  Wallpaper page (every screen's picture in one place) is its own piece,
  next. 441 tests on Hubble, kepler and Techbench.
- **Step 1 done: the Wallpaper page** (2026-11-14): `Page::Wallpaper(screen)`
  in Look (a picture icon), the sidebar entry opening the primary screen's;
  with several screens a row of them first, each with the picture it shows
  (the primary marked), choosing whose wallpaper the page sets; then Same as
  the primary screen, Make primary, Fit, Background color, the pictures and
  Your pictures, as the screen pages had them. The screen pages keep scale,
  Shown at, Size of controls and the arrangement, and a Wallpaper row saying
  the picture with a button to the page. Without any screen the Screens
  group is left out (it said "Wallpaper" before). **The bar over the page is
  empty** (Dave, 2026-11-14: "drop the heading and the forward back arrows
  above the right pane"): no back and forward (the history went with them),
  and the server draws no title in a sidebar window's bar (decor.rs); the
  window is still named after its page for Alt+Tab and the dock. On Hubble
  (shots): the page on its one screen; a nested server with two screens: the
  row, choosing "right", giving it the panda (its own entry in desktop.toml,
  the desktop and its tile changed). 442 tests on all three.
- **No heading card on the pages** (Dave, 2026-11-14: "we clicked
  wallpaper, we don't need a big banner at the top"): each page opens with
  its one-line description in small dim text, then its groups; the page's
  name stays as the scroll's name for Contract 3. Adding pictures, a
  sub-page with no sidebar entry, keeps its name in bold over its line.
- **Step 2 in: the Advanced part** (2026-11-14): `advanced(key, rows)` in the
  settings app, a chevron and "Advanced" in small dim text at a page's end;
  a click opens its card, and it stays open for the session (per page, a
  set of keys; not saved). Sound's: the sound system and each output as the
  system names it ("HDA Intel PCH: HDMI 0 (hw:0,3)"), moved out of the cards,
  except where two outputs share a name (the system's name stays to tell them
  apart). A screen's: how the scale was found (moved from Shown at), the
  monitor's make and model (when it has one), its connector, size and place,
  its identity. On Hubble (shots): opened on Screens, kept open after Windows
  and back; Sound's HDMI outputs told apart. Wi-Fi's comes with step 4.
- **Step 2b in: input pages by kind** (2026-11-14; Dave: "maybe have
  seperate entries for: keyboard, mouse, trackpad, touchscreen", then
  "Mixed"; the word Touchpad left to Claude). Bus protocol **v7**: topic
  `inputs`, `ServerMsg::Inputs { devices (name, kind), seen }`, kinds
  Keyboard, Mouse, Touchpad, Touchscreen (others kept as a number). The
  server's inputs.rs keeps the devices present (a keyboard only with letters:
  no power buttons) and the kinds the machine has had, in
  ~/.local/state/vergence/inputs.toml (0600); a touchscreen counts when
  present, a touchpad only once it scrolls with fingers (a Logitech receiver
  claims a touchpad: kepler, and Hubble's too). The settings app: Keyboard
  and Mouse always, Touchpad and Touchscreen once had; each with an Advanced
  part naming the devices present. Touchscreen: which screen each panel is
  on (desktop.toml `[[touch.panel]]`, a choice with several screens, the
  server rereads it), what a tap and a hold do, a way to Screens for the
  size of controls. Checked: Hubble shows Keyboard and Mouse (its touchpad
  waits for a two-finger scroll, Dave's to do), Mouse's Advanced lists the
  mice; kepler shows Touchscreen, not Touchpad, its page names the SW4101C
  on HDMI-A-1. 444 tests on all three; Hubble and kepler installed with v7,
  Techbench's installed session needs `sudo tools/install.sh`.
- **Step 3 in: vergence-net** (2026-11-14), our own code with no
  dependency. `wpa`: the control-socket client (sockets found in
  /run/wpa_supplicant or /var/run/wpa_supplicant, not the global or
  Wi-Fi Direct ones; a socket of its own in the runtime directory, removed
  after); PING, STATUS, SIGNAL_POLL, SCAN (busy counts as started),
  SCAN_RESULTS, LIST_NETWORKS, join (ADD_NETWORK, the name as hex, the
  password checked and quoted or key_mgmt NONE, ENABLE and SELECT; a
  refused step removes the half-made network), select, enable all, forget
  (REMOVE and SAVE_CONFIG), save, disconnect, reconnect; `Events` on a
  socket of its own after ATTACH (scan results, connected, disconnected,
  wrong password, network not found, terminating). Untrusted text: names
  decoded from wpa_supplicant's escapes, at most 32 bytes, control characters
  dropped (the bytes kept to join with); passwords 8 to 63 printable ASCII
  or 64 hex digits, so none can carry a second command. Bars from dBm,
  band and channel from the frequency, one entry per network, strongest
  first. `ifaces`: getifaddrs on both systems (the socket-address headers,
  the hardware address from AF_PACKET or AF_LINK, the carrier from
  IFF_LOWER_UP or if_data's link state), addresses with prefixes, a kind by
  name (wl* Wi-Fi). Tests: parsing, what is sent, and a fake wpa_supplicant
  socket (commands in order, an event slipped before a reply passed over,
  events after ATTACH). The example `vnet [scan]`: on Hubble with
  wpa_supplicant started by hand for the test (stopped after) it read the
  state and scanned Dave's two networks with signal, security, band and
  channel; on Hubble and kepler the interfaces, carriers and addresses are
  right. 459 tests on all three.
- **Step 4 in: the Wi-Fi page** (2026-11-14). install.sh enables
  wpa_supplicant where there is a Wi-Fi card and no NetworkManager (Void:
  the runit service, the config given ctrl_interface_group=wheel and
  update_config=1 if it lacks them; FreeBSD: wlans_ and WPA DHCP in rc.conf,
  /etc/wpa_supplicant.conf if missing; untried, kepler has no card) and says
  so; on Hubble it enabled it, the socket directory root:wheel 0750. The
  server's network.rs: a thread looks at the links every 2 s
  (vergence-net's `is_cable`: wired, up, carrier, not virtual), a change
  holding 4 s, and tells wpa_supplicant DISCONNECT or RECONNECT by
  desktop.toml `[network] wifi` (off, on, auto; default auto), only when
  what is wanted changes or wpa_supplicant comes back; only in the session
  on the real screens (--drm), never in a nested test server. The first
  time Wi-Fi comes on for want of a cable with no known network: one
  notification, opening the settings app with `--page wifi`
  (~/.local/state/vergence/network.toml keeps that it was shown). The
  settings app's Wi-Fi page (wifi.rs; Connections group, a waves icon): the
  setting, the network in use with bars, the networks in range with lock
  and bars and Join, rescanned every 10 s while open and Wi-Fi is wanted,
  the password asked in the page (a secret field that takes the keyboard by
  itself: new `autofocus()` in vergence-widgets, tested; Show the password),
  joined, and on WRONG_KEY the try forgotten and the password asked again;
  known networks with Forget; Advanced with the device, state, signal,
  channel and band, access point, addresses and hardware address. It says
  when there is no Wi-Fi device, when Wi-Fi is NetworkManager's, when
  wpa_supplicant is not running (with what to do) and when the account is
  not in wheel. On Hubble (shots): off with the cable in ("Off: a network
  cable is connected"); On listed Dave's networks with bars; in a nested
  server, a wrong password for dNET1 was refused by the access point, said
  "Wrong password for dNET1." and asked again, nothing left in
  wpa_supplicant's list; back to the default, Wi-Fi off again. kepler: "This
  computer has no Wi-Fi device". Techbench has no Wi-Fi card. Still to do
  by Dave: join with the real password, forget it, pull the cable (Wi-Fi on
  by itself, and the notification once), the phone's hotspot. 462 tests on
  all three.
  Dave, same afternoon: joined dNET1, set back to When no cable is
  connected, pulled the cable: Wi-Fi came on by itself and joined (Hubble
  got 10.0.1.14, the reservation for Mobilenode's Wi-Fi), the note shown
  once.
- **Step 5 in, waiting on the phone: the Network page** (2026-11-14).
  vergence-net knows an iPhone's USB tethering (`Kind::Phone`: Linux by the
  interface's driver in /sys, `ipheth`; FreeBSD a `ue` interface with
  `dev.ipheth`), and it counts as a cable, so Wi-Fi turns off while the
  phone shares its connection. The page (Connections, three linked boxes):
  each cable port ("Ethernet", or "Ethernet (name)" with several),
  connected with its addresses or "Cable not connected"; "iPhone (USB)",
  or how to make it show (trust, Personal Hotspot); Wi-Fi in a line with
  its page a button away; Advanced with every interface as the system has
  it. Read only. On Hubble (shots, on Wi-Fi, cable out): Ethernet "Cable
  not connected", Wi-Fi "Connected to dNET1.". The phone by cable is Dave's
  to plug in (usbmuxd not installed, to see whether it tethers without).
  462 tests on all three. Dave plugged the cable back in: Wi-Fi off by
  itself, the page "Ethernet: Connected" and "Wi-Fi: Off.". The iPhone:
  seen (Apple 05ac:12a8), left in USB configuration 1; switched to 4 by
  hand, ipheth made `enp0s20f0u4c4i2` (the page: "iPhone (USB): Not
  sharing its connection"), never a link; the phone asked "Trust" on each
  reconnect and kept nothing, since the computer's half of the pairing is
  usbmuxd's. **Parked** (Dave, 2026-11-14: "3, park the cable for now");
  docs/next-settings.md has the options.
- **Step 6 in: docs** (2026-11-14): whitepaper Draft 0.15 (§6.9 "The settings
  app as built", §9), README, CLAUDE.md, docs/next-settings.md, this plan.

## Where acceptance stands (2026-11-14)

1. Tests: 462 on Hubble, kepler and Techbench. **Met.**
2. Groups, headings, icons, "frost" finds Appearance, every old page
   reached, moved settings work. **Met** (Hubble shots).
3. Wi-Fi on Hubble: networks with signal (met), join with the password and
   an address (met, Dave), wrong password said (met), turning off and on
   (met, and by the cable both ways, Dave), Advanced (met). Forgetting and
   the phone's hotspot: **met** (Dave, 2026-11-14 ~13:40: forgot dNET1,
   joined the iPhone's hotspot, pinged 1.1.1.1 with a T-Mobile address,
   rejoined dNET1 with its password, switched between the two and back, on
   dNET1 at the end with the hotspot kept as a known network).
   Then, at Dave's word ("on windows ... I have to click it manually"):
   **Join automatically** per known network (off: kept disabled in
   wpa_supplicant while not in use, marked by its id_str
   `vergence-manual` in wpa_supplicant's own file; choosing it still
   joins; the server re-disables it once Wi-Fi settles elsewhere or turns
   off, never mid-join), and ("it could notify: Networks Available") a
   note when Wi-Fi is on, has joined nothing for 20 s and networks are in
   range, the hotspot named first, once per such spell. Hubble:
   OMG!MyPhone set to join only when chosen (saved disabled=1). 464 tests.
   The network in use up front (Dave: "some of the connection statistics
   right up front"): band and channel, Signal (dBm and a word), Speed,
   Address; then ("make the signal indicator green? or maybe add a little
   chart"): its bars in green, amber or red by strength, a chart of the
   last two minutes' signal (sampled every 2 s while the page is open), and
   Traffic in and out from the interface's byte counters (vergence-net:
   Linux's 32-bit rtnl_link_stats, wrap handled; FreeBSD's if_data). Dave:
   "later when we build out the dock info icons we can grab the same for
   that one": the network tile's card (live tiles, whitepaper §6.9) is to
   reuse wifi.rs's signal_chart, strength_color and rate_text. 465 tests.
   **6 GHz** (Dave: "is it using the 6ghz at all?"): no; dNET1's strongest
   access point was its 6 GHz one (channel 37, -38 dBm), WPA3 only, and the
   page had joined dNET1 as WPA2 alone, with no country set (6 GHz "no IR").
   Now (Dave: "yes go ahead, country US"): vergence-net reads each network's
   handshakes across its access points (`Keys`), joins with WPA2+WPA3
   (`key_mgmt WPA-PSK SAE`, `ieee80211w 1`) or WPA3 alone (`SAE`, `2`);
   the server sets `sae_pwe 2` and desktop.toml `[network] country` in
   wpa_supplicant (saved in its file) and gives known networks WPA3 when
   they take it, rejoining the one in use. Hubble: country = "US"; dNET1
   upgraded, now on 6 GHz channel 37 at 160 MHz, 1921.5 Mb/s in (was 573.5
   on 5 GHz at 40 MHz), the page "In use. 6 GHz, channel 37.". 466 tests.
4. The iPhone by cable: the page says what it lacks (met); a connection:
   **parked** (needs pairing).
5. kepler says there is no Wi-Fi device (met); Techbench has no Wi-Fi card,
   so the NetworkManager message is untried on hardware.

## Steps

1. **The sidebar in groups**: headings, the Accessibility page, the Wallpaper
   page, the search field; the page icons.
2. **The Advanced part**, as a widget of the settings app. 2b. **Input pages
   by kind** (added 2026-11-14), with bus protocol v7's input devices.
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

- Bluetooth (no BlueZ: Dave, 2026-11-14), NetworkManager, VPN (parked: Dave,
  2026-11-14), static
  addresses, Notifications, Date and time, Users, the account card.
- A Wi-Fi tile on the dock (live tiles).

## Questions for Dave (answered 2026-11-14)

Answers: 1 as revised above (install.sh enables wpa_supplicant where there is
a card; Wi-Fi follows the Off / On / When no cable setting, default the
last); 2 (b) no usbmuxd at first, to see whether the phone tethers without it;
3 (a) colored rounded squares with a white mark; 4 (a) search now. VPN: a
page some day, parked.

The questions as asked:

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
