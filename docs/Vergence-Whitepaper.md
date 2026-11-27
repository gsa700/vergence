---
title: "Vergence"
subtitle: "A display architecture for right-sized, right-placed, accessible applications on Linux and FreeBSD"
author: "David Erickson (AB0R), with Claude"
date: "November 24, 2026 — Draft 0.27"
---

# Vergence

**A display architecture for right-sized, right-placed, accessible applications on Linux and FreeBSD**

David Erickson (AB0R), with Claude · Draft 0.27 · November 24, 2026

---

## Abstract

*Vergence: the movement of both eyes converging on one point so that you see one clear image. Here, every monitor and every toolkit converge on one answer.*

On today's Unix-like desktops, an application's size on screen depends on which monitor it happens to be on, which toolkit it was written with, and which environment variables were set when it started. Where it appears depends on whether the window system lets it choose, and on whether anything remembers where you last put it. Usually nothing does.

This paper proposes a path to **app consistency**: every application the right size, in the right place and readable, on any monitor setup, for every user, including users who depend on accessibility tools. It started with a daemon for ordinary X11 desktops that recognized monitor setups by their EDIDs, applied consistent scaling across toolkits and remembered window layouts per setup. It continues with a new display server resting on one authority and three contracts: geometry in visual size, placement owned by the server but negotiated with apps, and accessibility built into the protocol. Optional vector "display-list" surfaces revive the best idea of NeXTSTEP's Display PostScript.

As of November 2026 the phase 1 daemon has done its job and been retired, and the phase 2 server is the author's daily desktop on Void Linux and FreeBSD, with releases packaged for Void since version 0.1.0 (§9). Parts of phases 3 and 4 are built too: display lists drawn by the server, the accessibility tree with a magnifier and an agent interface on it, and a remote desktop over SSH.

---

## 1. The problem

Three symptoms, one cause.

**Size is inconsistent.** A window that looks right on a 1080p monitor is microscopic on a 4K or 6K panel, or the reverse. Mixed setups are worst: with a 4K, a 1080p and a 6K monitor side by side, no single scale setting suits all three, and most toolkits on X11 only accept a single setting.

**Placement is forgotten.** Unplug a monitor, dock a laptop, or wake from sleep, and windows pile onto one screen. Applications reopen wherever the window manager guesses. Windows 10/11 remembers layouts per monitor configuration; Linux desktops largely do not.

**Accessibility is an add-on.** Screen readers, magnifiers, voice control and automation reach applications through a separate side channel (AT-SPI over D-Bus), not through the window system itself. When Wayland tightened security, much of that tooling broke.

The common cause: **no single component knows the truth about each display and each window, and gives every application the same answer.**

---

## 2. Why today's systems fall short

### 2.1 X11

The X11 core protocol counts **pixels**. It has one DPI value for the whole screen (`Xft.dpi`, XSETTINGS) and no notion of per-monitor density. Toolkits have each invented their own scaling switches: GTK's whole-number `GDK_SCALE`, Qt's `QT_SCREEN_SCALE_FACTORS`, Avalonia's `AVALONIA_SCREEN_SCALE_FACTORS`, Java's `sun.java2d.uiScale`. They disagree about units, granularity and when they are read.

X11 also lets any application position its windows anywhere, but nothing remembers those positions. And it has **no security between clients**: any app can read every keystroke, capture any window, inject input and draw over other windows.

### 2.2 Wayland

Wayland fixed the plumbing: logical coordinates, per-output scale, fractional scaling, and isolation between clients. But:

- **Applications cannot position their own windows**, and most compositors don't remember positions either.
- The user still **guesses a scale percentage** per monitor; nothing derives it from the panel's physical size.
- XWayland applications are often blurry on scaled outputs.
- Its **deny-by-default security** broke screen sharing, global hotkeys, automation and accessibility tools for years, while permissions were slowly added back.

### 2.3 Windows and macOS

**Windows** solved scaling end to end with per-monitor DPI awareness: each app is told its monitor's DPI and is notified when a window moves to a different monitor. Apps that don't support this are bitmap-stretched by the system, so they're blurry but the right size. Windows also remembers window placement per monitor configuration. It is the benchmark for placement.

**macOS** renders at 2× and scales down per screen. Its smoothness on Retina displays comes from a device-independent drawing model inherited from NeXTSTEP.

### 2.4 NeXTSTEP: the road not taken

NeXTSTEP's Window Server (1988) ran **Display PostScript**. Applications didn't draw pixels; they sent PostScript drawing commands in device-independent units (points, 1/72 inch), and the server turned them into pixels for whatever screen they were on. One authority owned drawing, windows, compositing (buffered windows with alpha) and events. AppKit's *frame autosave* let each application remember its own window frames.

| Problem | NeXTSTEP | X11 today | Wayland |
|---|---|---|---|
| Scaling | Solved at the root: device-independent drawing | Pixels; toolkits improvise | Per-output scale, but user-guessed; pixels still the currency |
| Authority | One server owns everything | Server, window manager and compositor split | One compositor owns everything |
| Window placement memory | App cooperates (frame autosave) | Nobody | Mostly nobody |
| Descendant | macOS (Quartz/Display PDF, backing scale factor) | — | — |

X11 had a Display PostScript extension (DPS/X) on Sun, DEC and SGI workstations, and Sun's NeWS went further with a programmable PostScript window server. Both died, largely to licensing costs and X's momentum. **Wayland copied NeXT's single-authority architecture but not its drawing model**, which is why it fixed the plumbing but not the experience.

---

## 3. Design goals and principles

**Goal.** App consistency: every application the right size, in the right place and readable, on any monitor setup, for every user.

**Design principle.** *One component knows each display's true resolution and each window's state, and gives every toolkit and every window the same answer.*

**Further principles:**

1. **Derive, don't guess.** Scale comes from each panel's physical size (from its EDID) and viewing distance, not from a percentage the user tunes by eye.
2. **Remember per setup.** Layouts are keyed to the set of connected monitors, so every setup (docked, traveling, presenting) keeps its own arrangement.
3. **Every phase stands alone.** Each phase delivers value even if nobody else ever adopts it.
4. **Compatibility at the edges.** Existing X11, Wayland and D-Bus applications keep working; new capabilities are optional upgrades.
5. **Accessibility and security are part of the design**, not afterthoughts, and neither is allowed to break the other.
6. **Two platforms, one code base:** Void Linux first and FreeBSD second, every change built and tested on both. A core that runs on FreeBSD and on a Linux without systemd cannot depend on Linux-only interfaces (systemd, logind, Linux-specific kernel interfaces); where the platforms must differ, the code says so.
7. **Our own parts.** The core is written by the project. A part from another project is used only when it is truly necessary, and only by explicit decision (§8).
8. **Vergence does display things; the system does system things.** What only a display server can do is Vergence's: drawing, input, windows, the lock and login screens (§7.5), and remote *graphical* access, since only the server has the frames and the input, and only it can show who is watching. What belongs to the operating system stays the system's, run and configured with its own tools: SSH, the network, the firewall, accounts, the time zone, how passwords are checked (PAM). Vergence may show a system service's state and, at most, drive the system's own tools behind an elevation prompt; it never runs its own copy of one or keeps its own settings for it. So Wi-Fi goes through wpa_supplicant, the time zone through the system's tool, passwords through PAM by way of vdm, sound through the system's PipeWire, and Vergence Remote through SSH.

---

## 4. Phase 1: the X11 consistency daemon

Phase 1 was a daemon, vergd, that ran **alongside any EWMH-compliant window manager** (XFCE's xfwm4, Openbox and others), with no new window manager and no changes to applications. It was built through four milestones on real FreeBSD hardware, frozen when X11's walls (§5) proved to cost more than they taught, and retired on November 10, 2026, once nothing used it. Its profile, EDID and settings code lives on in the server. It is described here for what it showed.

### 4.1 Monitor-setup profiles

- Read every connected monitor's **EDID** through RandR; hash the set of EDIDs into a **profile key**.
- Each profile stores **resolution, refresh rate, arrangement, edge alignment and scaling**.
- On hot-plug, dock/undock, or wake from sleep, recognize the setup and **apply its profile automatically**. Unknown setups get a sensible default derived from the EDIDs' physical sizes.

### 4.2 One scaling answer for every toolkit

The daemon set every scaling input together, from the profile: XSETTINGS (`Gdk/WindowScalingFactor`, `Xft/DPI`) and `Xft.dpi`, the cursor size, and `GDK_SCALE`, `QT_SCREEN_SCALE_FACTORS`, `AVALONIA_SCREEN_SCALE_FACTORS` and Java's scale property. Because most toolkits read environment variables only at launch, it wrote them into the X session environment and the D-Bus activation environment, and it published its state, including the per-output scale map, in one documented root-window property.

### 4.3 Two scaling modes per profile

**Native (the default).** Each monitor at its native resolution, with per-monitor factors calculated from each panel's EDID for the toolkits that accept them (Qt, Avalonia). Toolkits that take only one global value (GTK, Electron, Java) cannot be right on every monitor on X11.

**Uniform (the macOS approach; opt-in).** Render the whole desktop at the highest scale needed and let `xrandr --scale` shrink it onto lower-density monitors. Everything is the same size, at the cost of GPU work and softness.

Uniform was the default until the first test on monitors of very different density (103 ppi beside 315 ppi): it was visibly less sharp, made an 11,861 × 4,038 desktop for two monitors, and broke the desktop's wallpaper at render scale 4. Native pixels at a calculated scale is also what the phase 2 server does for every application.

### 4.4 Window layout memory

For each window, store monitor, position, size and maximized/fullscreen state, keyed by `WM_CLASS` plus an optional title pattern, **per profile**. Restore when a window opens and when the setup changes, through EWMH requests, accounting for GTK's client-side decorations (`_GTK_FRAME_EXTENTS`) so windows don't drift a few pixels each restore. This works for almost every application, whatever its toolkit, because it acts on X windows from outside. Its weak spot is several windows of one application (several Firefox windows share one class, and their titles change with the tab): placing a window is easy, knowing *which* window it is needs a stable identity, which only phase 2 provides.

### 4.5 What each toolkit can use (X11)

| Toolkit | What it reads | Per-monitor scale? | Picks up changes while running? |
|---|---|---|---|
| **Avalonia** | RandR physical size or `AVALONIA_SCREEN_SCALE_FACTORS` | **Yes** | Per monitor yes; profile changes need a restart |
| **Qt 5/6** | `QT_SCREEN_SCALE_FACTORS` (fractional), `Xft.dpi` | **Yes** | Mostly at launch |
| **GTK 3/4** (Firefox, Thunderbird) | XSETTINGS: whole-number window scale + fractional text DPI | No (global) | **Yes**, live via XSETTINGS |
| **Chromium / Electron** | `Xft.dpi`, GTK scale, `--force-device-scale-factor` | No | At launch |
| **Java (Swing/JavaFX)** | `GDK_SCALE`, `sun.java2d.uiScale` | No | At launch |
| **Old Xlib / Motif / Tk** | `Xft.dpi` for fonts at best | No | No |

### 4.6 Worked example

Three monitors in a row, physical sizes assumed:

| | M1 | M2 | M3 |
|---|---|---|---|
| Panel (assumed) | 27″ 4K | 24″ 1080p | 32″ 6K |
| Native resolution | 3840×2160 | 1920×1080 | 6016×3384 |
| User scaling | 150% | 100% | 200% |
| Logical size | 2560×1440 | 1920×1080 | 3008×1692 |
| Effective density | ~109 ppi | ~92 ppi | ~109 ppi |

**Uniform, rendering at 2×,** makes a desktop of 14,976 × 3,384 pixels, just under the common 16,384-pixel limit of one X screen (§5), with about 40% more pixels drawn than native. M1 is slightly soft (0.75 is not a whole-number scale), M2's text is softer than true 1080p (drawn at 2× and halved, losing hinting), and old Xlib programs are small everywhere. **Native** gives Qt and Avalonia 1.5 / 1.0 / 2.0, sharp and right-sized; GTK, Electron and Java must take one value and are wrong somewhere. Either way, the user's percentages leave M2 about **18% larger** physically (92 vs 109 ppi); deriving the scale from the EDID's physical size corrects that, which Windows does not.

### 4.7 Lessons from FreeBSD

The daemon was built on real hardware from the start: a virtual machine's synthetic EDIDs and hot-plug behavior would only have been worked around, not learned from. The first machine (FreeBSD 15.1, Intel UHD 630) showed two things a Linux-first build would have missed: Xorg on FreeBSD does not re-probe its outputs when a monitor is plugged in, although the kernel reports it to devd, so the daemon listened to devd and asked X to re-probe; and a freshly started Xorg does not publish the RandR EDID property until the first re-probe.

### 4.8 Testing on real hardware, automated

The kernel's DRM layer gives every connector, on Linux and on FreeBSD, debugfs controls that make it look plugged in or unplugged (`force`) and report any EDID (`edid_override`). With them the daemon ran against monitors from the public linuxhw EDID database without a cable being touched: all 174,759 EDIDs parsed without fault, and 2,000 ran as fake monitors in under five minutes, broken ones included (no physical size, no serial number, identical twins). The same idea, screens that exist only in memory, became the phase 2 server's display test suite (§9).

---

## 5. Native per-monitor rendering, and the X11 walls

The ideal is to **separate the logical space** (where windows live, where the pointer moves, where layouts are saved) **from each monitor's native resolution**, so every monitor always shows native pixels. Windows and Wayland both work this way.

On X11 this works in tiers:

1. **Toolkits that support per-monitor scaling** (Qt, Avalonia) are native everywhere with the daemon alone. Only a window straddling two monitors is resampled on one side, as on Windows and macOS.
2. **Other applications** can be drawn at one scale and have a compositing window manager scale **just their windows** to suit each monitor. This is Windows' approach.
3. **The wall is input.** If the compositor shows a window at 0.75 size, X still routes clicks by the window's real geometry, so clicks land in the wrong place. X11 has no per-window input transformation. It was prototyped (Sun's Looking Glass, Compiz experiments) but never merged.

A second wall is **the size of the desktop**. X keeps the whole desktop as **one picture**: a single framebuffer in which every monitor shows its own rectangle. That picture must fit in one buffer the GPU can draw and scan out (16,384 × 16,384 on Intel UHD 630; often 8,192 on older GPUs), and the X protocol's 16-bit coordinates cap it at 32,767 whatever the hardware. The Uniform mode multiplies the desktop by its render scale, so it reaches the limit quickly: on the phase 1 test machine, a 13.3″ 4K panel (332 ppi, needing 4×) beside two ordinary monitors would have made a desktop 19,651 pixels wide, and the daemon refused it.

Removing the input wall on X11 would need a compositing window manager **plus a new X server extension**; the desktop-size wall is part of X's design. The phase 2 server removes both a different way: **it owns input routing**, and **it keeps one buffer per monitor**, sized to that panel, so the only limit is what one panel needs.

### 5.1 Worked example: a twelve-monitor wall on three GPUs

Nothing in the design limits the number of monitors. A monitor setup is a list of EDIDs of any length, and window places are stored per setup, so twelve monitors are simply a larger profile, and every subset that appears (one panel unplugged, one card's panels switched off) becomes a profile of its own. Take a workstation with three NVIDIA RTX 3060 cards, each driving four displays.

**Telling twelve monitors apart.** A monitor whose EDID carries a serial number is recognized whichever port it is plugged into. Many low-cost panels report serial 0, so twelve of one model have identical EDIDs; they are then told apart by connector, and swapping two cables swaps their layouts. No server can do better: identity can only come from the panel or the port.

**On X11** the wall cannot be one dependable desktop. Twelve 4K panels in a 4 × 3 grid fit X's 32,767 cap, twelve in a row do not; and joining three GPUs into one desktop means either Xinerama, which turns off RandR's per-output interface on the NVIDIA driver, or one GPU rendering everything for the others to scan out (PRIME output sinks), copying every frame across PCIe.

**On the Vergence server** every GPU is opened through DRM/KMS with one buffer per monitor, so there is no single picture and no size cap, and because the server owns input, three cards make one desktop. What remains is hardware: four displays per card, GPU memory for twelve buffers, and a copy between cards when a window drawn on one GPU is shown on another's panel. The server already shows screens of a second GPU this way (§8.3). Display lists (§6.6) help: the server can draw such a window on each card instead of copying pixels.

---

## 6. The long-term architecture: one authority, three contracts

### 6.1 Root problems

1. **Wrong unit.** X counts pixels; Wayland counts pixels times a guessed scale. Neither knows how big things look to the user.
2. **Wrong placement authority.** X lets apps place windows but nobody remembers; Wayland forbids apps from placing and mostly doesn't remember either.
3. **Accessibility bolted on**, through a side channel that broke under Wayland's security model.
4. **Pixels are the only currency**, so magnification is blurry, straddling windows are wrong on one side, and scaling means resampling.

### 6.2 One authority

A single display server owns all outputs and all input, like NeXT's Window Server. It knows each panel from its EDID: physical size, subpixel layout, color gamut, refresh capabilities. It composes **each monitor in its own buffer**, at that panel's native resolution, so no single picture of the whole desktop exists and X's desktop-size limit (§5) does not apply.

### 6.3 Contract 1: geometry in visual size

- The protocol unit is the **CSS reference pixel**: the visual angle of one pixel at 96 dpi seen from arm's length. Browsers have used this definition for decades. It's the right unit because **viewing distance matters**: a TV across the room, a laptop and a 32″ monitor need different densities for the same apparent size.
- Each monitor's scale is **calculated** from physical size and viewing distance: scale = (ppi ÷ 96) × (viewing distance ÷ 71 cm). Viewing distance is guessed, not asked: a built-in laptop panel is assumed at laptop distance (50 cm), everything else at desk distance.
- **The default is the sharpest scale near the calculated one**: the nearest whole number when it is within 25%, and never below 1. Whole numbers are sharp in every toolkit and need no resampling; most people's one ordinary monitor then shows its native pixels at 1×. A 103 ppi monitor (calculated 1.07) gets 1×, a 14″ 4K panel (3.28) gets 3×, a 27″ 4K (1.70) gets 2×. Only where no whole number is close (a calculated 1.5) does the exact value stand.
- **The user can choose a screen's own scale**, on a slider marked at the whole numbers and the default, and sets **one** size preference ("how big I like things") that multiplies every screen. A switch, "windows the same size on every screen", gives every screen its exact calculated scale instead.
- Applications that support it render **natively per monitor**: told the exact scale, they draw at the final pixel size, so any scale is shown 1:1 without resampling. At a fractional scale the project's windows keep to its pixel grid (at 133⅓%, steps of 3 points), so their edges fall on whole pixels; text on a screen set below its calculated scale is drawn a little larger, by an adjustable share of what the scale took away.
- **Text size is a separate preference from interface size**, a real accessibility need.

### 6.4 Contract 2: placement, cooperative but owned by the server

- Every window has a **stable identity**: application ID, window role and a persistence token.
- Applications **may request** a position, relative to a monitor or a parent window. The **server decides and remembers**, per monitor set and per resolution and scale of it: the same monitors at another scale keep places of their own, and going back finds the old ones. A size seen for the first time starts from the nearest one, each window kept in proportion to its monitor.
- **The token is the application's to keep.** The server issues it when the window first opens; an application that wants its windows back hands each one's token in again on the next run, and the window returns exactly where it was. A window the user closes on purpose is forgotten; a session's end forgets nothing. Places unseen for 90 days are pruned.
- **The server draws every window's bar** on the native protocol, with the window controls on the side the user chose and an application's **tabs** described by the application and drawn, hit-tested and dragged by the server. A tab dragged out becomes a window where it was dropped; dropped on another window of the same application, it joins it. One bar for every application, so they look and behave alike and the application's name on it cannot be faked (§7.3).
- **User rules override both** (for example, "the logbook always opens on the left monitor, top half").
- Unplugging, sleeping and waking never pile windows onto one screen.

### 6.5 Contract 3: meaning, not just pixels

- Each window publishes its **accessibility tree as part of the window protocol**: roles, names, values, states, bounds in reference units, focus and caret position. A secret field's text never leaves the program.
- The server can therefore offer, natively: a **magnifier that follows focus and the text caret** (built; with display lists, §6.6, text at 4× is re-drawn text), screen readers by design, **voice control and automation** under explicit permission (§7), and consistent, server-drawn focus highlighting. A screen reader needs a voice, which is a borrowing question of its own and not yet built.

**An agent is a client of this contract too** (built in meaning milestone 1, November 20, 2026). A software agent acting for the user, whichever one the user chooses, needs what a screen reader and a voice-control tool need: to know what is on the screen, and to act on it. Today's agents work from screenshots and pointer coordinates, because that is all a desktop gives them. Here an agent is given the same tree:

- **It reads meaning, not pixels:** roles, names, values and bounds ("the button named Save, enabled"). It **acts by name**: press this button, type into this field.
- **Permission is per agent and per application**, asked in a dialog the server draws, for once, a time or a session, under the rule every permission follows (§7.2): granted by the user, shown by a tile on the dock while the agent is at work, revocable, and logged. A password field is no more open to an agent than to any other tool.
- **It never takes the user's pointer or keyboard:** the server tells the window which widget to press or fill, so the user can watch it, shown by a ring where it acts, or carry on beside it.
- **A terminal is a special case**, since what it holds is whatever its programs print. An agent reads or types in a tab only by a grant for that tab, can ask for a terminal window of its own, is refused at password prompts and in tabs running as root, and Escape pressed twice in the tab ends its grant.
- **Any agent can be plugged in.** The interface is the server's own, on the session bus (§6.8); an adapter, `vergence-agent-mcp`, speaks the Model Context Protocol at the edge, as D-Bus is spoken at the edge.

The agent layer is frozen since November 22, 2026: maintained, but grown only where daily use asks. The tree itself keeps growing, for accessibility and testing. The control socket the server has for scripted tests is not this interface: it goes around the security model and exists only when a developer turns it on.

### 6.6 Optional display-list surfaces

An application may submit either a **bitmap** (always supported) or a **retained vector scene**: a display list of drawing commands in **the server's own minimal, validated format** (fills, strokes, lines, glyph runs, pictures, clips, a terminal's cells, in reference units). Existing serializations such as Skia's SkPicture were not designed to be read from untrusted processes, so they are not used on the wire. For display-list surfaces the server can render the window **at any density, at any time**:

- **A window straddling two monitors is sharp on both halves.** No other current desktop does this.
- **Magnification is truly sharp:** text at 4× is re-rendered text, not enlarged pixels.
- **The overview and the dock's window pictures** are drawn at their own size.

Built in display lists milestone 1 (November 20, 2026): every window of the project's own toolkit, vergence-ui (§6.10), is sent as a scene and drawn by the server with the toolkit's own renderer, so there is one renderer for both. Each scene's cost is estimated before it is drawn, and a window over its budget goes back to sending pixels. Wayland and X11 programs send bitmaps, as they always will.

**Over the network**, display lists would carry drawing commands instead of video, the network transparency X promised. Vergence Remote (§9) starts the simpler way: the host draws its screen again at the viewer's size and scale and sends the changed pixels, compressed; display lists over the wire come later. Display PostScript died because every application had to speak it; here it is an optional upgrade over bitmaps.

### 6.7 Other display problems addressed

- **Color:** per-monitor color management from EDID colorimetry, so the same color looks the same on every monitor; HDR where the panel supports it; color-blindness filters and night light applied per output.
- **Timing:** each monitor on its own refresh timing (mixed 60/144 Hz, variable refresh rate); tear-free by default, with a low-latency mode for games.
- **Text rendering per panel:** subpixel layout per panel (RGB, BGR, OLED), and hinting decided by density: hinted at low density, unhinted at high density.
- **Pointer:** a cursor that is the same physical size on every monitor, and pointer speed consistent in physical distance across mixed-density monitors.
- **Accessibility settings in one place:** reduced motion, high contrast, reduced transparency, large cursor, sticky keys, all honored by the server.

### 6.8 The session bus: our own core, D-Bus at the edge

A desktop carries more than windows: notifications, settings that apply live, launching and activating applications, accessibility, media keys. On Linux and FreeBSD that traffic goes over the **D-Bus session bus**, a separate daemon through which any program can call any other. It has the same flaws as the window systems in §6.1:

- **It is a side channel.** It knows nothing about windows, monitors or which application the user is looking at. Accessibility (AT-SPI) travels this way, which is why it is an add-on (§1).
- **It trusts every caller.** Any program on the bus can call any service, and broadcast signals go to whoever asks for them.
- **Services start themselves.** D-Bus activation launches a service the first time anyone calls its name. In the first phase 2 session on real hardware, Firefox found no session bus, started a private one, and a notification call activated Xfce's notification daemon, which appeared as an ordinary window in the middle of the desktop.

Vergence uses the same pattern for the bus as for windows: **the core is its own, and compatibility lives at the edge.**

**Core: the server is the bus.** Session services are part of the native protocol, on the same connection as windows, with the same identity and permissions (§7), defined in the shared protocol crate (one exact version shared by the server and every app). Its services: **notifications** drawn by the server; **settings**, one source of truth (fonts, theme, cursor, text and interface size) pushed to every subscriber at once, so applications follow a change live; **launch and activation**, with activation tokens, so focus moves only on user action; the **Contract 3 tree**; **live tiles** (§6.9); and later media keys and global hotkeys registered by name, and screen capture under permission.

The rules: **the server always knows who is calling** (socket credentials now, a sandbox identity where one exists, §7.4); **there is no eavesdropping**, and a signal reaches only subscribers entitled to it; **nothing starts on demand** except services the user has installed and declared.

**Edge: a D-Bus front end, written by the project.** Existing applications expect a D-Bus session bus, so the server provides one at the usual address (`DBUS_SESSION_BUS_ADDRESS`). It speaks the D-Bus wire protocol, which is documented and small: authentication over a Unix socket, the message encoding, and the bus's own methods. It does not run the reference D-Bus daemon. Calls to well-known interfaces are **translated into core services**:

| D-Bus interface | Core service |
|---|---|
| `org.freedesktop.Notifications` | Notifications |
| `org.freedesktop.portal.Settings` (color scheme, accent, fonts) | Settings |
| The portal's file chooser and OpenURI | The desktop's Open and Save dialog, opening files and links (§6.10) |
| AT-SPI (`org.a11y.Bus` and the accessibility bus) | Contract 3, through an AT-SPI bridge (later, §8.2) |
| Others as measured: screensaver inhibit, application activation | As needed |

Applications that talk to themselves over the bus keep working: a name an application owns is routed to it, so Firefox opening a link in the window already running still works. A name nobody owns and the edge doesn't translate gets a clean "service unknown" reply, never an automatic start. The edge keeps no state beyond which application owns which name, and permissions are decided in the core.

**Why not run the reference D-Bus daemon at the edge?** It would work, but it keeps the side channel's flaws (no window identity, open broadcasts, activation) and still leaves every service to be written. The subset applications actually use is small. **Out of scope:** the D-Bus **system** bus (power, removable media, networking) belongs to the operating system and stays as it is.

### 6.9 The shell: the dock

The contracts above are where Vergence is new. The shell, the part of the desktop the user sees around the windows, is deliberately not a second experiment: **a familiar shell on a new foundation**, minimal, but showing what is installed and what is running. One rule runs through it: **every surface has an everyday layer and a deeper one, and nothing deeper appears until it is asked for.**

**What it borrows, and from where.** Window Maker (1997), the free descendant of NeXTSTEP's desktop, had three ideas worth keeping: one icon for each running application, small live tiles ("dockapps") instead of a tray of unrelated icons, and a menu on the bare desktop. It aged badly because its tiles were a fixed 64 pixels, dockapps were ordinary windows drawn by whatever program supplied them, and nothing showed what was installed. From current desktops Vergence takes the macOS habits of a floating dock and a launcher that searches as you type.

**The dock** holds everything a top bar and a dock would hold between them, drawn by the server with the toolkit's drawing code (§6.10), sized in reference units so it is the same physical size on every monitor. It is a rounded panel as tall as what it holds, floating a small gap in from the edge; it starts on the right edge of the main monitor, and moved by Alt+drag or "Move dock" in its menu it settles along any edge of any screen, or floats where it was put. Whether windows keep clear of it, and whether it hides until the pointer reaches the edge, are settings. If the main monitor is unplugged, it moves with the primary role to another screen. From top to bottom:

- **The apps button** opens the **launcher**, also opened by tapping Super: a frosted panel in the middle of the pointer's screen. Its search field is ready at once and finds an application by name, by kind ("browser" finds Firefox), keywords and description, then **named desktop actions** (mute, screens off, show the desktop, each settings page, each switch on a tile's card): one list of what the desktop can be asked to do by name, the same list voice and agents will use under permission, so there is one command surface for hands, voice and agents. Under the field, category tabs filled from the categories every application's desktop entry already declares.
- **Applications**: pinned and running, one icon each, with a mark beside those that are running; icons are dragged to reorder, pin and remove. Resting on an icon opens a card of its windows as small live pictures, grouped by screen; resting on a picture brings that window forward for a moment (a **peek**). A minimized window goes to its application's icon.
- **Live tiles**: clock, sound, network, battery, and anything else that publishes one. A tile is not a small window: its owner **publishes a value and a set of actions on the session bus, and the server draws it** in the desktop's style, so tiles are sharp at any scale, look alike whoever supplies them, are readable by a screen reader, and cannot draw anything misleading. At rest a tile shows one value; resting on it opens its **card** (detail, a short chart, the everyday controls); a click goes to where the item lives, usually its page in the settings app. **Anyone may publish a tile** (`vtile` from a script): a home lab's UPS through NUT, rack temperatures, a NAS. Tiles not chosen for the dock wait in a **drawer**, and **a tile in trouble comes out by itself** while the problem lasts, so a drawer of fifty cannot hide the one that matters.
- **The Dockworker** is the dock's own tile for whoever is acting on the desktop besides the user: it shows when an agent or a remote viewer is at work, and ends either with one click.

**What the dock knows that others don't.** Because the server owns placement and remembers it per monitor setup (Contract 2): when a monitor is unplugged its windows are shown as **away** and go back to their places when it returns; an application started from the dock opens where it last was on this setup; and **session restore** starts again the applications that were running, so after a kernel update and a restart the desktop is as it was left. The list survives a crash; the project's own tools are told they are being restored (the terminal reopens its tabs); only applications known by a desktop entry are started, never a remembered command line; and a session that ends twice soon after being restored starts clean.

**Turned down, with the reasons.** A top bar in the manner of macOS: a second strip on another edge, and a global menu far from its window on a desk of several monitors. Workspaces: monitors and placement memory do that job, and window memory stays keyed to the monitor setup alone. Automatic tiling, which would move windows the user had placed. Everything in the settings app, as phones do: too slow for the time or a mute. **Open:** other programs' tray icons (StatusNotifierItem over D-Bus), which the edge could draw as plain tiles.

**Windows.**

- Windows **float**, and the server remembers where they were put. **Snapping** is on by default: dragged to a screen's left or right edge a window takes that half, to a corner a quarter, to the top edge the whole screen; a tint shows where it will go, and it can be set to snap only with Ctrl held, or never. The maximize button and the title bar's right-click menu offer the same layouts as pictures.
- **Keyboard**: Super with the arrows snaps, maximizes, restores or minimizes the focused window. Alt+Tab steps through windows, most recently used first, with a frosted list while Alt is held; Alt+\` does the same among one application's windows. A new window takes the keyboard only when it was asked for; otherwise it opens behind and its icon asks for attention.
- **Menus stay in each window.** There is no global menu bar.
- **Kinds of window.** A program says what each window is (Wayland and X11 programs by the hints they already give), and the server shapes it: a **dialog** has a slim bar, opens over its parent and goes with it; a **modal** one dims its parent; a **palette** floats above its own application's windows only; a window with a largest size has no maximize button; a **sidebar window** (the settings app) runs its content up under the frosted bar.
- **The desktop** shows the wallpaper and the Desktop folder's icons, with Home, the drives and the Trash (drawn by FileView, kept running by the server); a click on bare ground slides the windows aside (the overview).

**The settings app** is laid out by the same layers: a sidebar in groups (Connections, Look, Desktop, Screens, Input, Sound, Accessibility, Privacy, System) with a search over the pages and their settings, each page opening on its everyday choices with an Advanced part below. Among its pages: each screen's resolution and refresh rate, with a "Keep this resolution?" dialog that goes back by itself after 15 seconds; Wi-Fi through the project's own client of wpa_supplicant's control socket, by default on only while no network cable is connected; energy on battery and on power; notifications per app; default applications; forgetting window places; the lock and login screens; and Remote access, off by default. The settings app is always light, because small gray text on dark is hard to read.

### 6.10 The toolkit: vergence-ui, the tools' own

The project's own tools are drawn with a **toolkit written by the project**, **vergence-ui**: that crate and the ones under it, vergence-scene (the display list, §6.6), vergence-render (the GPU renderer and the CPU reference renderer), vergence-text (fonts, fallback, glyphs), vergence-widgets (widgets, layout and their meaning) and vergence-wl (the project's own Wayland client code). It was decided on November 9, 2026, after two days of building the first tools on a borrowed toolkit (iced): its software renderer rebuilt a window-sized mask for every run of text, its GPU renderer lost small drawings, clicks went missing under load, and it had no route to Contract 3. Each fault had a workaround; the lesson, the same one the author's music applications taught, is that the workarounds cost more than the part. The terminal and the settings app moved on November 10, and iced was removed.

It is not a general-purpose toolkit. It is what this desktop's tools need, and it carries the design's priorities down to the level where a button is drawn:

- **The scene is the currency.** A tool describes its window as a retained scene in the format of §6.6, not as pixels, and in a Vergence session the server draws it.
- **GPU first, with a CPU reference.** Scenes are rendered on the GPU at the exact scale of the monitor they are on. The same scene drawn in software is the **reference renderer**, for tests and for machines without a GPU; the two must agree, which is how the GPU renderer is tested.
- **One renderer, shared with the server.** The code that draws a tool's scene draws the server's dock, title bars and notifications too: one implementation of text, one of shapes, one place to fix a drawing fault.
- **Meaning is built in.** Every widget publishes its role, name, value and bounds as it is laid out. A tool cannot be built on this toolkit without an accessibility tree, and the same tree is what named actions, voice and agents address.
- **Isolation.** Reading a font, decoding a picture or an archive and parsing what a program prints are done on **untrusted input**, off the thread that owns the window, and where the platform allows in a process that can do nothing else (Capsicum on FreeBSD; no new privileges and limits on Linux). Input is handled in order at the place it happened, and never waits for drawing.
- **Security.** A tool gets nothing a third-party application would not get (§7): no global input, no reading other windows, no placing itself. The toolkit is also the proof that the protocol is enough.
- **Size and placement come from the server.** The toolkit has no scale setting and no window-position code of its own. Elsewhere, on another Wayland desktop, it talks Wayland behind the same interface and draws its own bar.

**An application's own settings live in the application.** Every window whose bar the server draws has one **app button**: the application's icon and a chevron, at the end opposite the window's controls, opened by a click or Alt+Space. One rule decides what goes where, in every application: **the app button is the application, a right click is the thing under the pointer, the title bar is the window.** An application does not send a menu: it sends **commands with roles** (Copy, New tab, Settings …), and a role's name, place and keys are the desktop's, drawn from one table, so no program can call Copy something else or give its keys to another command. A terminal, whose Ctrl+letter keys belong to its programs, gets Shift added to them, by the same rule in every terminal. The menu always ends with Settings… (Ctrl+,), Keyboard shortcuts, About and Quit. **Settings…** opens a window the toolkit draws for every tool alike: the application's own part, then **On this desktop**, rows that inherit the desktop's value and follow it live until the application is given its own. The desktop's Style (App default, Dark, Light) is the master switch for light and dark.

**Text, files and spelling are the toolkit's**, so every tool has them alike. A **text area** edits text of any length (only the lines in view are laid out, so a file of 200,000 lines types as quickly as a note), with line numbers, wrapping, undo in sensible steps, find and replace, completion in gray as TextEdit does it, and **syntax colors** for languages built in or added by definition files. **Spelling** is checked by the project's own checker against a word list made from SCOWL, in every text field, in prose only where the text is code. The **Open and Save dialog** is one dialog for every tool, and since the files milestone the file chooser other programs ask the desktop for. What a file is comes from the project's own table of about 140 types, and which application opens it from the shared mimeapps.list files with the desktop's own choices first. **Pictures are decoded apart**, in a helper under limits on time, memory and files; **archives** too, by libarchive in a confined helper that writes nothing itself. **Fonts** are the project's own default everywhere (Inter and JetBrains Mono, shipped with it), so the desktop looks the same on every system.

**The tools on it** (November 2026): **Terminal**; **Edit**, plain text and code in tabs; **FileView**, the file manager, with tabs, thumbnails shared with other desktops, the Space preview, copies with progress and undo, the shared Trash, network mounts, removable drives with Eject (udisks2 on Linux, the base automounter on FreeBSD) and archives; **Close-Up**, the viewer for pictures; **Reader**, PDFs on the project's own engine; **Screenshot**; **Calculator**, with the project's own exact decimal arithmetic (0.1 + 0.2 is 0.3); **AlbumWall**, the author's music player rebuilt for Vergence (§8.1); **Hardware**; **Force Quit**; and **Settings**. **Sound** in the Space preview plays through Hymie, the music engine of the author's AlbumWall, ported to Rust. On a touch screen, holding a finger still is a right click in every tool.

**Text is the hard part** of any toolkit: shaping for scripts that need it, right-to-left text, input methods. Plain left-to-right text with kerning and per-character font fallback is built; the rest is named in §10 rather than assumed.

---

## 7. Security model

### 7.1 What is being protected

On X11 any client can log every keystroke, capture any window or the whole screen, inject input (XTest), move, cover or kill other windows (including drawing fake password dialogs), and read the clipboard at will. Wayland's response was to deny all of this and add permissions back over time, which broke legitimate tools for years. **X allows everything; Wayland denied first and patched later. Both are wrong.**

### 7.2 Principle

**Deny by default, but every legitimate need is a first-class permission:**

- **granted** explicitly by the user;
- shown by a **visible indicator** while in use, like a camera light;
- **revocable** at any time;
- recorded in an **audit log**.

Nothing is impossible; nothing is silent.

### 7.3 Risks and mitigations

| Area | Risk | Mitigation |
|---|---|---|
| Placement requests (Contract 2) | Spoofing (fake dialogs over real ones), focus stealing, hidden windows | Positions are requests; the server forbids covering another app's window or always-on-top without permission. Focus moves only on user action (activation tokens). The owning application's name in the title bar is **drawn by the server**, so it cannot be faked. |
| Accessibility and automation (Contract 3) | A screen reader is technically a full screen reader and keylogger | Permissions for specific tools, with an indicator. **Password fields are protected**: contents never leave the app; tools see only "secure field focused". Audit log. |
| Agents (Contract 3, §6.5) | An agent that can read the tree and act is as able as the user, and can be misled by what it reads on the screen | Grants per agent and per application (per tab in a terminal), asked in a server-drawn dialog, with the Dockworker's tile and the log. It acts through meaning, never through the user's pointer or keyboard. Refused at password fields, password prompts and root tabs. |
| Display-list surfaces | The server interprets untrusted drawing commands | Strict validated format; each scene's cost estimated, a window over its budget going back to pixels; drawn by the server in its own GPU context. Untrusted *files* (pictures, fonts, archives, PDFs) are parsed in confined helpers: **Capsicum on FreeBSD**, no new privileges and limits on Linux. |
| Legacy X11 apps | X clients can spy on each other within one X server | Off by default. They cannot see native apps; untrusted X apps can each run in their own rootless X server. |
| Screen capture | Silent screen recording | Permission plus indicator, scoped to a window or a monitor. |
| Clipboard | Background clipboard sniffing | Read only on the user's paste or by the focused app; clipboard managers need a permission. |
| Input injection | Fake input | Permission plus indicator (remote desktop, voice control). |
| Global hotkeys | Apps keylogging to catch shortcuts | Hotkeys **registered with the server by name and approved by the user**. Push-to-talk keys for radio and voice tools work properly. |
| Session bus (§6.8) | Programs calling each other's services, sniffing broadcast signals, or starting services silently | The server knows every caller; no eavesdropping; no on-demand activation beyond declared services; the D-Bus edge exposes only translated interfaces and names an application owns. |
| Remote display | Network eavesdropping and hijack; being watched unawares | Off by default, and no socket at all while off. Only through SSH (or WireGuard), never a TCP port of its own; the Dockworker's tile shows every viewer, and the person at the desk can always end it. |

### 7.4 An honest limit

On a typical desktop, any program running as the user can already read the user's files and attach a debugger to their other programs. **A display server cannot create isolation that the operating system doesn't provide.** Its job is to **not be the weak link**. Real application-to-application isolation needs sandboxing (Flatpak on Linux, jails and Capsicum on FreeBSD), which also gives the server a **trustworthy application identity** to attach permissions to. Without a sandbox, identity is only the executable path, which is weaker. The design supports both and is explicit about the difference.

### 7.5 The lock screen and the login screen

A lock screen is only worth having if it is a real lock, so VDM, the project's display manager, is built around one small root program, `vdm`, and nothing else that holds privilege:

- **One password checker.** `vdm` alone talks to PAM, on both systems; neither the user's server nor the login screen can check a password themselves. It learns who is asking from the kernel (the peer of the Unix socket), never from the request, and checks only that account. Each check runs in a child process of its own with a time limit, so a PAM module that hangs (a security key waiting for a touch) holds only that child. After a wrong answer the account waits before it may try again, 1, 2, 4 seconds and on up to five minutes, kept across restarts.
- **The password never becomes an ordinary string.** The lock and the login dialog are drawn by the server itself, and the typed text lives from the first key in locked, wiped memory kept out of core dumps; there is no paste into it. A program that is not the server cannot draw over the lock, and nothing is shown until the lock is on every screen.
- **The login screen starts nothing.** It is the same server run as an account that owns nothing (`_vdm`), with a fresh home each time, in a mode that refuses to start any program and leaves out the session bus, the D-Bus edge, X11, the dock and the launcher. Root and system accounts cannot log in at it.
- **Sessions on their own consoles.** Each login runs on a virtual terminal of its own, through the user's login shell, from an environment built from nothing; when it ends, whatever it left running is ended too, before the login screen comes back.
- **Locking before sleep.** The system's sleep hooks ask `vdm` to lock every session and wait until each shows the lock, so the machine never wakes showing the desktop.
- **Automatic login only where it cannot give away a secret**: off unless turned on, and allowed only where exactly one person can log in and no storage is encrypted (LUKS, GELI, encrypted ZFS), checked again at every boot.

What it does not do, said plainly: switching to another virtual terminal stays possible, and a text console where the user is logged in is not covered by the lock; and §7.4 holds. The full threat model, an independent review of the design and a review of the code are in the project's VDM plan.

---

## 8. Implementation strategy

**No new drivers.** The server builds on the shared lower layers that **both Linux and FreeBSD** provide: DRM/KMS for display, the kernel's input devices (evdev), GBM, EGL and Vulkan for rendering (§8.3).

**Our own parts.** The project writes its own core. A part from another project (a library, a program, a protocol extension added only so borrowed clients work) comes in only when it is truly necessary, and only by explicit decision of the project lead, with the reason and the cost of writing our own on record. The operating system's own interfaces and services (PAM, PipeWire, wpa_supplicant, udisks2) and the compatibility edges' protocols are not borrowing; implementations of them are. Parts borrowed while bringing the server up stay until each is decided on; docs/borrowed-review.md keeps the list to review:

| Part | Used for | Status |
|---|---|---|
| Smithay | The server's Wayland front end, DRM/KMS and input back ends | Moving to a support role (decided November 8, 2026). Input dispatch, focus and grabs are the project's own since November 12 (vergence-seat), and the server draws native windows with the project's own renderer; next the window space, then the DRM/KMS and input back ends; the Wayland front end may stay as an edge |
| drm-rs | Smithay's DRM crate | Carried with a patch for FreeBSD's device nodes, meant to go upstream |
| libinput, libseat/seatd, xkbcommon | Input devices, seat and device access, keymaps | Through Smithay; xkbcommon also directly, for the server's own keyboard state |
| libarchive | Reading archives (in a confined helper only) and writing .zip, for FileView | BSD 2-clause, linked from the system, not shipped; through the project's own FFI (November 23, 2026) |
| Xwayland | X11 applications (the X11 edge) | Off by default; an X server of our own is not planned |
| ab_glyph, resvg | Glyph outlines, SVG wallpapers and icons | Small; candidates for our own |
| image, png, zune-jpeg | Decoding pictures (in the decoding helper only), writing thumbnails | MIT/Apache; BMP is the project's own |
| minimp3, ALAC, FAAD2 | Hymie's MP3, ALAC and AAC decoders, ported to Rust | CC0 and Apache-2.0; FAAD2 (GPL) only in a helper program of its own |
| Inter, JetBrains Mono, Liberation; SCOWL | The desktop's fonts (Liberation for PDFs' standard fonts); the spelling word list | Data under the SIL Open Font License and SCOWL's permissive terms; read by the project's own code |

**Language: Rust throughout.** Everything the project ships is written in Rust: the server, which needs steady frame timing that garbage-collected runtimes can't promise; the toolkit and every tool; the protocol, shared by both. One language means one toolchain, and the profile, EDID, configuration and protocol code shared by server and tools without bindings. Rust builds natively on FreeBSD; .NET there is a port outside Microsoft's platforms, which is one reason the author's .NET applications are not the desktop's base (§8.1).

**Compatibility at the edges.**

- **Wayland applications** connect through a standard Wayland front end.
- **X11 applications** run through rootless Xwayland, off by default. Because the server owns input routing, legacy windows can be **scaled per monitor with correct input mapping**, which removes the input wall from §5 without any X extension.
- **D-Bus clients** reach the project's own D-Bus front end (§6.8).
- **Sound**: the session starts the system's PipeWire on Void, and the desktop speaks to it with its own client of the PulseAudio protocol; on FreeBSD, OSS directly.
- The server speaks Wayland, X11 and D-Bus at the edges, but placement, memory, units, accessibility and security follow this design.

**Platforms and releases.** **Void Linux (glibc) is first and FreeBSD second**, settled on November 22, 2026, for hardware and application support (graphics, Wi-Fi, laptop power saving, and applications built only for Linux). Development started on FreeBSD, which taught the project much of what §4.7 and §9 list, and moved to Void on November 12, because the work ahead (GPUs, several cards, explicit sync, sleep and wake) is where FreeBSD's ported DRM drivers lag and its tools are thin. Every change is built and tested on two Void machines, a laptop with Intel UHD graphics and an NVIDIA GTX 1650 and a desktop with Intel UHD 770 graphics and a 27″ 4K monitor, and on a FreeBSD 15.1 desktop with Intel UHD 630 graphics and a 14″ 4K touch monitor. (Fedora was a third platform until November 21, to keep the logind side honest.) Releases are tagged and packaged for Void (0.1.0 on November 22; 0.3.0 the latest); Vergence installs on a stock system and leaves its disk layout, encryption and backups to that system. An installer of its own on a Void base, and a FreeBSD port, come after.

### 8.1 The author's .NET/Avalonia applications

The author's existing applications (AlbumWall, Deadwax, Marginal and others) are .NET/Avalonia, and they run on Vergence as any Wayland application does. They stay what they are on the Mac, Windows and a Raspberry Pi kiosk, and their FreeBSD port is not pursued. For Vergence itself the direction changed on November 21, 2026: rather than give Avalonia a native backend, **AlbumWall is rebuilt in Rust on vergence-ui and Hymie** ("AlbumWall for Vergence", generic name Music), looking and working the same. Its first milestone, a folder library with everything seen and played, is in 0.4.0; the music server, lyrics and the media keys are its second. A portable build of the toolkit for macOS and Windows would later let such apps go back to those platforms.

### 8.2 Major third-party applications: Firefox and Thunderbird

Firefox and Thunderbird are GTK applications on Linux and FreeBSD with mature native Wayland support, and on Vergence they are among the best-behaved programs:

- **Scaling.** Native Wayland is their best mode: Firefox supports per-monitor fractional scaling and is sharp on every monitor.
- **Placement.** They don't request positions, but the server owns placement and remembers windows by app ID. Telling several windows of one application apart needs a persistence token, which depends on Mozilla adopting the Wayland session-restore protocol (§10).
- **Accessibility.** Both have rich accessibility trees, exposed through AT-SPI. Bringing them into Contract 3 needs an **AT-SPI bridge** at the D-Bus edge; Mozilla is not expected to write a native backend.
- **Surfaces.** They remain bitmap surfaces (GPU-rendered by WebRender): fully supported, without the sharp-at-any-density benefits of display lists.
- **Security.** Clipboard on paste, focus on user action and permissioned capture match how they already behave on Wayland, so the model costs them nothing.

### 8.3 GPUs and drivers

Machines will have Intel, AMD or NVIDIA graphics, on open-source or proprietary drivers, some with two GPUs at once. "No new drivers" still holds: the server uses the interfaces the drivers already provide (KMS for display, GBM for buffers, EGL and Vulkan for rendering, sync objects for timing). What differs is which of them a driver implements, and how well.

| GPU | FreeBSD | Linux |
|---|---|---|
| Intel | drm-kmod (a port of Linux's DRM drivers) + Mesa | in-kernel DRM + Mesa |
| AMD | drm-kmod (amdgpu) + Mesa | in-kernel DRM (amdgpu) + Mesa |
| NVIDIA, open source | **none**: FreeBSD has no nouveau | nouveau + NVK (Mesa) |
| NVIDIA, proprietary | NVIDIA's driver + its DRM module for KMS | NVIDIA's driver; KMS only with `modeset=1` |

Everything on Mesa shares one path: GBM buffers, EGL rendering, and implicit synchronization, where the kernel makes sure a buffer is finished before anyone reads it. The server runs on this path today, Intel first and NVIDIA through nouveau as a second GPU. NVIDIA's proprietary driver is a different stack: KMS must be switched on (`modeset=1`), GBM comes through NVIDIA's own back end (on Linux since driver 495; on FreeBSD unknown, §10), and it does no implicit sync, so without **explicit sync** (DRM sync objects, and a fence handed over with each buffer, supported from driver 555) Xwayland and GL programs flicker or show half-drawn frames.

**Design decisions:**

1. **Explicit sync belongs in the native protocol**: every buffer a client submits carries an acquire point and gets a release point, with implicit sync only as a fallback at the edges. It is the right design on every GPU, since the server then knows exactly when each buffer is ready. It is not built yet; NVIDIA's own driver waits for it.
2. **Each monitor's buffer belongs to a GPU.** When a window is drawn on another GPU, its picture is copied across, and the server can tell the application which GPU sits under its window. This covers hybrid laptops and multi-card walls (§5.1).
3. **Capabilities are probed, never assumed by vendor.** Formats, modifiers, sync support and GBM are queried from each driver at start-up and logged, and the server picks its path from the answers. A driver that improves gets used fully without a code change.
4. **The server survives a GPU reset.** When the kernel resets a hung GPU, the server's GL context can go on drawing only black or white while every call reports success (seen on the UHD 630), and Smithay's context is not robust, so it cannot ask. A frame that blocks for seconds is therefore taken as a reset, and the renderer is rebuilt on a new context without a restart. A reset that does not stall a frame still goes unnoticed (seen once on FreeBSD); a robust context of the project's own will close that gap.

**A screen on a second GPU, as built** (November 14, 2026). The development laptop has no MUX switch: its panel is wired to the Intel GPU and its HDMI and USB-C outputs to the NVIDIA GTX 1650 (nouveau). Every GPU on the seat is probed at start and every GPU with connectors is opened. The Intel GPU draws every screen; a screen on the NVIDIA GPU is drawn into a linear buffer in the Intel GPU's memory, which the NVIDIA GPU takes in as a texture, copying **only the changed rectangles** into its scan-out buffer once the Intel GPU's fence has signaled, without the server ever waiting. Three findings shaped it: Mesa runs nouveau's OpenGL through Zink, which needs the NVK Vulkan driver (without it the "GPU copy" ran on the CPU at 40 to 77 ms a 4K frame, so the server names each GPU's renderer at start); damage tracking by buffer age must count every frame, drawn or not; and windows are asked for a frame only when one went to the screen, so each screen's refresh paces its windows. With no screen on it, the NVIDIA GPU sleeps and the laptop draws about 9.4 W idle instead of 15. Still to come: a copy by the CPU for GPUs that share no buffer format, and a screen drawn by its own GPU for heavy loads. AMD and NVIDIA's own driver wait for hardware to test on.

---

## 9. Roadmap

| Phase | Deliverable | Value on its own | Where it stands |
|---|---|---|---|
| **1** | X11 consistency daemon: EDID profiles, scaling, toolkit settings, layout memory | Fixes daily annoyances on any X11 desktop | Built, then retired (November 10, 2026) |
| **2** | Display server on real hardware: native per-monitor rendering, placement memory, reference-unit scaling, the session bus and its D-Bus edge, the project's own toolkit and tools, the shell, the login and lock screens | A usable personal desktop that beats X11 and Wayland on consistency | The daily desktop; released for Void |
| **3** | Accessibility contract and display-list surfaces | Sharp magnification, sharp straddling windows, accessibility by design | Display lists, the tree, the magnifier and the agent interface built; screen reader, voice and the AT-SPI bridge to come |
| **4** | Remote display | A remote desktop as good as sitting at the machine | Vergence Remote in progress: pixels first, display lists later |

**Status, November 26, 2026.** Vergence is the daily desktop on two Void machines and a FreeBSD one, and every change is built and tested on all three before it is pushed. Hot-plug comes from the operating system's device events; connectors are never polled. Built:

- **The server:** DRM/KMS, GPU rendering with a CPU fallback, every screen at its own calculated scale, screens on a second GPU, and **headless screens**: screens that exist only in memory, beside real monitors or with none at all, from fake EDIDs of any size.
- **The protocol:** the session bus with its D-Bus edge (notifications, the settings portal, the file chooser), and the native window protocol: GPU pictures or scenes, input, the clipboard (both ways with Wayland and X11 programs), placement requests the server decides, and the meaning tree.
- **The seat:** every pointer, keyboard and touch event decided by the project's own dispatcher, the first step of Smithay's replacement.
- **Windows:** the server-drawn bar with tabs, window memory per monitor setup and scale, snapping, Alt+Tab, dialogs, palettes and sidebar windows, full screen.
- **The look:** frosted glass under bars, the dock, menus and the launcher (and, in 0.4.0, behind a see-through terminal), light and dark versions of everything with contrast checked in both, rounded corners, the project's own icons, pointers and fonts.
- **The shell:** the dock with live tiles, the drawer and the Dockworker, the launcher, named desktop actions, the overview, session restore.
- **VDM:** the login and lock screens (§7.5).
- **Meaning:** widgets acted on by meaning, the magnifier, the agent interface with permission, the terminal on it.
- **The tools** of §6.10, with sound through PipeWire, archives, drives and PDFs.
- **Releases:** packages for Void, 0.1.0 to 0.3.0 between November 22 and 23; 0.4.0 on November 26, with AlbumWall for Vergence, syntax colors in Edit and the frosted terminal. Vergence Remote follows in 0.5.0.

**Display testing.** Since November 18, three layers check what reaches the screen, all running on headless screens: geometry tests at every clean scale with no GPU; a test-pattern window with a one-pixel line on each edge, stepped through the scales and checked pixel by pixel; and a suite of scenarios (hot-plug, a window rescued and given back its place, a scale changed under a window, two scales side by side, the GPU against the CPU reference, the magnifier, syntax colors, the frosted terminal, AlbumWall's wall playing and keeping its place) that runs on the development machines before anything that draws is pushed. Its first runs found a picture drawn one pixel taller than it was shown at 120% (rounding in single precision).

Still open within phase 2: drag and drop with Wayland and X11 programs; docks on more than one screen; the drawer's filter field and a calendar in the clock's card; the rest of Smithay's replacement; explicit sync and NVIDIA's own driver (§8.3); AMD; checks by hand left in a few milestones.

**What comes next**, in order:

1. **Vergence Remote**: an application to use a Vergence machine from another as if sitting at it, a KVM for use rather than hardware, over SSH: the host's screen drawn at the viewer's size, input, the clipboard, files, the login screen, the person at the desk shown who is watching. The Vergence viewer first, then the Mac through a portable build of the toolkit.
2. **AlbumWall for Vergence, milestone 2** (§8.1): the music server (Navidrome), lyrics, the media keys and a system Now Playing tile.
3. **A web component**: WebKit (WPE) as a component any app can embed, then mail and a browser on it.
4. **The Vergence system**: an installer and image on a Void base, then a FreeBSD port.

**What FreeBSD taught phase 2.** As in phase 1 (§4.7), the real machine showed things a Linux-first build would have missed:

- There is no logind, so seat and device access go through seatd, and the project's login screen runs on seatd as well as on logind.
- FreeBSD's PAM (OpenPAM) agrees with Linux's on the calls and differs in the numbers of several results; the project declares both, and a test asks each system's PAM to name them.
- Every query of a DRM connector's state took about 327 ms, and polling connectors froze the whole server every few seconds. Hot-plug is detected from devd's DRM events instead.
- There are no abstract Unix sockets, so Xwayland is reached only through its socket file.
- Removable drives go through the base system's automounter, which keeps a drive's name after it is pulled out and mounts FAT drives for root unless told otherwise; the project's map and checks handle both.
- drm-kmod's GPU hangs under load (a page fault in its buffer mapping, a context reset during tests) are kept as evidence, with a report tool, before anything is restarted.

**Where the real costs are.** Writing the code is not the bottleneck. The bottlenecks are **real-hardware testing** (GPUs, unusual EDIDs, docks, sleep and wake; since November 18 the display's part is automated, and what stays manual is real docks and KVM switches), **the user's time and judgment** (what "the right size" means to their eyes), and, for display lists beyond the project's own tools, **toolkit adoption**, which only a working demonstration can start.

---

## 10. Open questions and items to verify

Numbered as in earlier drafts, which other documents cite; answered items are kept in one line.

1. *Retired with phase 1:* live-update behavior of Qt 6 and Chromium scaling on X11.
2. *Retired with phase 1:* Avalonia's X11 per-screen scaling.
3. Reliability of EDID physical-size data, and the fallback when it is missing. In the linuxhw database 0.3% report no size at all, 0.4% an implausible one, and 6.6% give a millimeter size more than 2 cm away from their own centimeter size.
4. *Answered:* viewing distance is guessed from the connector (built-in panels at laptop distance), and users change a screen's scale rather than the distance.
5. Status of Wayland session-restore and placement protocols, and whether the server should implement them for compatibility.
6. *Answered (display lists milestone 1):* the display-list format and its validation, with text as glyph runs. Open: sending scenes over the network (§6.6).
7. FreeBSD specifics: DRM/KMS coverage beyond Intel; drm-kmod's GPU hangs under load; a suspend-and-wake loop for testing (no rtcwake).
8. The toolkit's text: shaping for scripts that need it, right-to-left text and input methods, and whether shaping is written by the project or borrowed; which GPU interface the renderer moves to (OpenGL ES through EGL today, or Vulkan).
9. *Dropped:* a native Avalonia backend (§8.1).
10. Firefox and Thunderbird: whether Mozilla implements the Wayland session-restore protocol; the scope of an AT-SPI bridge into Contract 3.
11. Many monitors on several GPUs (§5.1): DRM/KMS with more than one card, NVIDIA's included; how many monitors report a real EDID serial number.
12. GPUs and drivers (§8.3): whether NVIDIA's FreeBSD driver provides GBM, KMS and sync objects well enough; which nouveau/NVK generations are usable; AMD on real hardware, on both systems.
13. Session bus: which D-Bus interfaces everyday applications call. *First measured November 8, 2026:* the settings portal (now provided), GNOME's idle monitor, gvfs, screensaver inhibit, Ubuntu's menu registrar and the accessibility bus; none of the applications failed without them.
14. The agent interface: *built and frozen* (§6.5). Open: an agent's own pointer and focus as a second seat, if ever needed; how an agent misled by text on the screen is kept from harm beyond grants and refusals; voice control on the same interface.
15. Session restore: *built* (§6.9). Open: how much of the Wayland and X11 session-management protocols to speak at the edge for other toolkits.
16. *Settled November 22, 2026:* Void Linux is the reference platform, FreeBSD second (§8).

---

## Appendix A: Glossary

- **EDID:** data a monitor reports about itself: manufacturer, model, serial, physical size, supported modes, color characteristics.
- **RandR:** the X11 extension for querying and configuring outputs (monitors).
- **EWMH:** Extended Window Manager Hints; the standard by which clients and tools talk to X11 window managers.
- **XSETTINGS:** an X11 protocol for publishing desktop settings (fonts, scale) that toolkits read live.
- **CSS reference pixel:** the visual angle of one pixel at 96 dpi seen from about arm's length (roughly 0.0213°).
- **Compositor:** the component that combines window images into the final screen image.
- **Agent:** a program that acts for the user by reading what applications show and operating them; here a permitted client of Contract 3 (§6.5).
- **Display list:** a recorded sequence of drawing commands that can be replayed at any resolution.
- **KMS/DRM:** the kernel interfaces for display mode-setting and GPU access, on Linux and FreeBSD.
- **GBM:** Generic Buffer Management, the interface through which a display server allocates GPU buffers it can show on a monitor.
- **Mesa:** the open-source implementation of OpenGL, Vulkan and GBM used by Intel, AMD and nouveau drivers.
- **Explicit sync:** passing a fence with each buffer that says when the GPU has finished with it, instead of leaving the kernel to work that out (implicit sync).
- **Toolkit:** the code an application draws its buttons, lists and text with. Vergence's own, vergence-ui, is described in §6.10.
- **Smithay:** a Rust library for building Wayland compositors.
- **Capsicum:** FreeBSD's capability-based sandboxing framework.
- **Dock:** Vergence's floating panel at a screen edge, holding the apps button, application icons and live tiles (§6.9).
- **Dockworker:** the dock's tile that shows an agent or remote viewer at work, with a way to end it.
- **Hymie:** the music engine (decoders, tags, playback) of the author's AlbumWall, ported to Rust for Vergence.
- **Window Maker:** a window manager for X11 (1997) modeled on NeXTSTEP's desktop, with a dock of application icons and small live "dockapps".
- **D-Bus:** the message bus Linux and FreeBSD desktops use between programs; the *session* bus serves one user's login, the *system* bus the whole machine. *Activation* starts a service when its name is first called.
- **AT-SPI:** the accessibility interface on Linux and FreeBSD desktops, carried over D-Bus.

## Appendix B: One sentence

*NeXT's single authority and device independence, Windows' placement memory, Wayland's per-monitor rendering, and the web's reference pixel, with accessibility and security part of the protocol rather than add-ons.*
