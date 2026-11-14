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
