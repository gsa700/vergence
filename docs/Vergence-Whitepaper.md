---
title: "Vergence"
subtitle: "A display architecture for right-sized, right-placed, accessible applications on Linux and FreeBSD"
author: "David Erickson (AB0R), with Claude"
date: "10 November 2026 — Draft 0.9"
---

# Vergence

**A display architecture for right-sized, right-placed, accessible applications on Linux and FreeBSD**

David Erickson (AB0R), with Claude · Draft 0.9 · 10 November 2026

---

## Abstract

*Vergence: the movement of both eyes converging on one point so that you see one clear image. Here, every monitor and every toolkit converge on one answer. (The project was called App Consistency until Draft 0.4.)*

On today's Unix-like desktops, an application's size on screen depends on which monitor it happens to be on, which toolkit it was written with, and which environment variables were set when it started. Where it appears depends on whether the window system lets it choose, and on whether anything remembers where you last put it. Usually nothing does.

This paper proposes a path to **app consistency**: every application the right size, in the right place and readable, on any monitor setup, for every user, including users who depend on accessibility tools. It starts with something buildable in days on an ordinary X11 desktop: a daemon that recognises monitor setups by their EDIDs, applies consistent scaling across toolkits, and remembers window layouts per setup. It ends with a long-term design for a new display server. That server rests on one authority and three contracts: geometry in visual size, placement owned by the server but negotiated with apps, and accessibility built into the protocol. Optional vector "display-list" surfaces revive the best idea of NeXTSTEP's Display PostScript.

The design targets **Linux and FreeBSD** together: development on a Linux without systemd, with FreeBSD and Fedora tested at every milestone.

As of November 2026 the phase 1 daemon is built and frozen, and the phase 2 server runs as the author's daily session on real monitors, on FreeBSD and on Linux (§9).

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
2. **Remember per setup.** Layouts are keyed to the set of connected monitors, so every setup (docked, travelling, presenting) keeps its own arrangement.
3. **Every phase stands alone.** Each phase delivers value even if nobody else ever adopts it.
4. **Compatibility at the edges.** Existing X11, Wayland and D-Bus applications keep working; new capabilities are optional upgrades.
5. **Accessibility and security are part of the design**, not afterthoughts, and neither is allowed to break the other.
6. **Three platforms, one code base:** a Linux without systemd for development, FreeBSD and Fedora at every milestone. A core that runs on FreeBSD and on a Linux without systemd cannot depend on Linux-only interfaces (systemd, logind, Linux-specific kernel interfaces); Fedora keeps the logind side honest.
7. **Our own parts.** The core is written by the project. A part from another project is used only when it is truly necessary, and only by explicit decision (§8).

---

## 4. Phase 1: the X11 consistency daemon

A daemon that runs **alongside any EWMH-compliant window manager** (XFCE's xfwm4, Openbox and others). It requires no new window manager and no changes to applications.

### 4.1 Monitor-setup profiles

- Read every connected monitor's **EDID** through RandR; hash the set of EDIDs into a **profile key**.
- Each profile stores **resolution, refresh rate, arrangement, edge alignment and scaling**.
- On RandR hot-plug, dock/undock, or wake from sleep, recognise the setup and **apply its profile automatically**. Unknown setups get a sensible default derived from the EDIDs' physical sizes.

### 4.2 One scaling answer for every toolkit

The daemon sets every scaling input together, from the profile:

- XSETTINGS (`Gdk/WindowScalingFactor`, `Xft/DPI`) and `Xft.dpi`
- cursor size
- `GDK_SCALE`, `QT_SCREEN_SCALE_FACTORS`, `AVALONIA_SCREEN_SCALE_FACTORS`, Java's scale property

Because most toolkits read environment variables only at launch, the daemon writes them into the **X session environment and the D-Bus activation environment**, so newly started applications inherit them. It also publishes its state, including the per-output scale map, in **one documented X root-window property**. Applications that opt in can follow changes live; our .NET/Avalonia applications do so through a small helper library (§8.1).

### 4.3 Two scaling modes per profile

**Mode B, "Native" (sharpest; the default).** No GPU scaling; each monitor at its native resolution; per-monitor factors, calculated from each panel's EDID, go to toolkits that support them (Qt, Avalonia). Those apps are perfect. Toolkits that support only one global value (GTK, Electron, Java) cannot be right on every monitor on X11. That is a known gap, to be closed by toolkit work and then by the phase 2 server, not a reason to prefer Mode A.

**Mode A, "Uniform" (the macOS approach; opt-in fallback).** Render the whole desktop at the highest scale needed and use `xrandr --scale` to scale lower-density monitors down. Every toolkit uses the same scale, so everything is the same size everywhere. The costs are GPU work and some softness on scaled-down monitors.

Mode A was the default until the first test on real monitors of very different density (103 ppi beside 315 ppi). There it was visibly less sharp, made an 11,861 × 4,038 desktop for two monitors, and broke the Xfce desktop's wallpaper at render scale 4. Mode B, native pixels at a calculated scale, is also what the phase 2 server does for every application, so design and testing effort goes there.

### 4.4 Window layout memory

For each window, store monitor, position, size and maximised/fullscreen state, keyed by `WM_CLASS` plus an optional title pattern, **per profile**. Restore when a window opens and when the setup changes, using EWMH requests (`_NET_MOVERESIZE_WINDOW`, `_NET_WM_STATE`). Handle GTK client-side decorations by accounting for `_GTK_FRAME_EXTENTS`, so windows don't drift a few pixels each restore.

### 4.5 What each toolkit can use (X11)

| Toolkit | What it reads | Per-monitor scale? | Picks up changes while running? |
|---|---|---|---|
| **Avalonia** | RandR physical size or `AVALONIA_SCREEN_SCALE_FACTORS` per output | **Yes** | Per monitor yes; profile changes need a restart unless the app reads the daemon directly |
| **Qt 5/6** | `QT_SCREEN_SCALE_FACTORS` (fractional), `Xft.dpi` | **Yes** | Mostly at launch |
| **GTK 3/4** | XSETTINGS: whole-number window scale + fractional text DPI | No (global) | **Yes**, live via XSETTINGS |
| **Chromium / Electron** | `Xft.dpi`, GTK scale, `--force-device-scale-factor` | No | At launch |
| **Java (Swing/JavaFX)** | `GDK_SCALE`, `sun.java2d.uiScale` | No | At launch |
| **Old Xlib / Motif / Tk** | `Xft.dpi` for fonts at best | No | No |

*Behaviour as understood at the time of writing; to be verified during implementation (see §10).* Window layout memory works for almost every application regardless of toolkit, because it acts on X windows from outside.

Firefox and Thunderbird use GTK on Linux and FreeBSD, so on X11 they behave like the GTK row: one global scale, read at launch. Major third-party applications are covered in §8.2.

### 4.6 Worked example and acceptance test

Three monitors in a row. Physical sizes are **assumed** for the calculation.

| | M1 | M2 | M3 |
|---|---|---|---|
| Panel (assumed) | 27″ 4K | 24″ 1080p | 32″ 6K |
| Native resolution | 3840×2160 | 1920×1080 | 6016×3384 |
| User scaling | 150% | 100% | 200% |
| Logical size | 2560×1440 | 1920×1080 | 3008×1692 |
| Effective density | ~109 ppi | ~92 ppi | ~109 ppi |

**Mode A (Uniform, render at 2×):**

| | M1 | M2 | M3 |
|---|---|---|---|
| Rendered at | 5120×2880 | 3840×2160 | 6016×3384 |
| `xrandr --scale` | 1.333×1.333 | 2×2 | 1×1 |

- **Virtual screen:** 14,976 × 3,384 pixels, just under the common 16,384-pixel limit on one X screen (§5). A fourth monitor would exceed it.
- **GPU load:** about 43 Mpx rendered versus 31 Mpx native, roughly 40% more.
- `xrandr --scale` takes the rendered size divided by the panel's native size, so a value above 1 shrinks the picture onto the panel: M1 shows its 5120×2880 render on 3840×2160 (shown at 0.75 size), M2 shows 3840×2160 on 1920×1080 (shown at half size).
- **M1** is slightly soft, because 0.75 is not a whole-number scale.
- **M2** text is softer than true 1080p, because it's drawn at 2× and halved, which loses font hinting.
- **Old Xlib/Tk applications** draw at 1× and are small everywhere, but at least consistently small.

**Mode B (Native):** Qt and Avalonia receive 1.5 / 1.0 / 2.0 and are sharp and right-sized on each monitor. GTK, Electron and Java must take one value. With 2 they are double size on M2 and oversized on M1; with 1 they are tiny on M1 and M3.

**Equal physical size.** The user's percentages leave M2 about **18% larger** in physical terms (92 vs 109 ppi). The daemon corrects this from EDID physical dimensions, so a window is the same physical size on all three. Windows does not do this.

**Edge alignment.** Logical heights differ (1440 / 1080 / 1692), so the profile records top or centre alignment, and the pointer and dragged windows cross edges sensibly.

**Pass criteria:**

1. The same window is the same physical size on M1, M2 and M3 in Mode A, across GTK, Qt, Avalonia and Electron.
2. Layouts restore correctly after unplugging and replugging M3, and the two-monitor setup keeps its own layout.
3. The virtual screen stays within GPU limits; the daemon refuses a mode that would exceed them and explains why.
4. Firefox with two or three windows open, plus Thunderbird, return to the right monitor, position and size after a restart and after an unplug/replug, with no drift from client-side decoration shadows.

### 4.7 First milestone

Read EDIDs, hash each setup into a profile key, and log RandR hot-plug events. Small, safe, and the foundation for everything else. It is built and tested on real hardware from the start, on a FreeBSD workstation with real monitors, with Debian alongside: a virtual machine's synthetic EDIDs and hot-plug behaviour would only be worked around, not learned from.

The first real machine (FreeBSD 15.1, Intel UHD 630) turned up two things a Linux-first build would have missed. Xorg on FreeBSD does not re-probe its outputs when a monitor is plugged in, although the kernel reports the plug to devd; so on FreeBSD the daemon listens to devd and asks X to re-probe, and X then sends its usual RandR events. And a freshly started Xorg reads every EDID but does not publish the RandR EDID property until the first re-probe, so the daemon re-probes once when a connected output has no EDID.

### 4.8 Testing on real hardware, automated

Real-hardware testing is the slow part of phase 1 (§9), so most of it is scripted. The test harness is a separate tool; it may need root, the daemon never does.

1. **Plug and unplug without touching a cable.** The kernel's DRM layer gives every connector two debugfs controls, on Linux and on FreeBSD (drm-kmod, through lindebugfs): `force` makes it look plugged in or unplugged, and `edid_override` makes it report any EDID. Forcing sends no hot-plug event, so the harness makes X re-probe after each change. The daemon can then be run against thousands of real monitors' EDIDs from the public linuxhw EDID database, including broken ones (zero or wrong physical size, missing serial numbers, identical twins). First results: all 174,759 EDIDs in the database parse without fault, and 2,000 of them run as fake monitors in under five minutes.
2. **Sleep and wake, overnight.** The machine suspends and wakes in a loop (`rtcwake` on Linux; FreeBSD has no equivalent yet, so the method there is still open); after every cycle the profile and the layout must be unchanged.
3. **Same physical size, measured.** Small test programs in GTK, Qt, Avalonia, Electron and Java each draw a box of known logical size; the harness screenshots every monitor and measures each box in millimetres using the EDID's physical size. The user's judgment of "the right size" is needed once, as a calibration, not on every run.
4. **Layout memory, scripted.** Windows (several Firefox windows and Thunderbird included) are placed, a monitor is unplugged and replugged through (1), the applications restart, and every window's position and size is compared with what was saved, with a check for drift of a few pixels per restore.
5. **Record and replay.** Every RandR event and EDID seen on real monitors, docks and KVM switches is recorded once; the recordings become tests that run anywhere, with no monitors attached.

What stays manual: the one-time size calibration, and occasional real monitors, docks and KVM switches, which do things no script predicts. The details are in the milestone 1 plan.

---

## 5. Native per-monitor rendering, and the X11 walls

The ideal is to **separate the logical space** (where windows live, where the pointer moves, where layouts are saved) **from each monitor's native resolution**, so every monitor always shows native pixels. Windows and Wayland both work this way.

On X11 this works in tiers:

1. **Toolkits that support per-monitor scaling** (Qt, Avalonia) are native everywhere with the daemon alone (Mode B). Only a window straddling two monitors is resampled on one side, as on Windows and macOS.
2. **Other applications** can be drawn at one scale and have a compositing window manager scale **just their windows** to suit each monitor. This is Windows' approach.
3. **The wall is input.** If the compositor shows a window at 0.75 size, X still routes clicks by the window's real geometry, so clicks land in the wrong place. X11 has no per-window input transformation. It was prototyped (Sun's Looking Glass input redirection, Compiz experiments) but never merged.

A second wall is **the size of the desktop**. X keeps the whole desktop as **one picture**: a single framebuffer, the "screen", in which every monitor shows its own rectangle. That picture must fit in one buffer the GPU can draw and scan out, so its size has a hardware limit (16,384 × 16,384 on Intel UHD 630; often 8,192 on older or smaller GPUs), and the X protocol's 16-bit coordinates cap it at 32,767 whatever the hardware. Mode A multiplies the desktop by its render scale, so it reaches the limit quickly: on the phase 1 test machine, a 13.3″ 4K panel (332 ppi, needing 4×) beside two ordinary monitors would have made a desktop 19,651 pixels wide, and the daemon refused it (§4.6, pass criterion 3).

On X11, removing the input wall would need a compositing window manager **plus a new X server extension**; the actively developed XLibre fork of Xorg is the most plausible home for one. The desktop-size wall is part of X's design and has no such fix. The long-term architecture below removes both a different way: **its server owns input routing**, and **it keeps one buffer per monitor**, sized to that panel, so the only limit is what one panel needs.

### 5.1 Worked example: a twelve-monitor wall on three GPUs

Nothing in the design limits the number of monitors. A monitor setup is a list of EDIDs of any length, and window places are stored per setup, so twelve monitors are simply a larger profile. Every subset that appears (one panel unplugged, one card's four panels switched off) becomes a profile of its own, with its own remembered layout. The limits come from elsewhere. Take a workstation with three NVIDIA RTX 3060 cards: each drives up to four displays, twelve in all.

**Telling twelve monitors apart.** A profile can only put a window back on "the third panel from the left" if it recognises that panel. A monitor whose EDID carries a serial number is recognised whichever port it is plugged into. Many low-cost panels report serial 0, so twelve of the same model have identical EDIDs; the daemon then tells them apart by connector, as it already does for identical twins. Layouts survive restarts and hot-plugs, but swapping two cables swaps their layouts. The new server can do no better: identity can only come from the panel or from the port.

**Phase 1 (X11).** Three walls apply.

- **Desktop size.** The whole desktop is one picture (§5). NVIDIA's buffer limit is well above Intel's 16,384, so the X protocol's cap of 32,767 is the one that applies. Twelve 1080p panels in a 4 × 3 grid make a desktop of 7,680 × 3,240, and twelve 4K panels in the same grid 15,360 × 6,480: both fit. Twelve 4K panels in one row (46,080 wide) do not, and Mode A multiplies the desktop by its render scale, so a wall that mixes densities reaches the cap sooner. The daemon refuses a setup that does not fit and says why, as it does today.
- **One desktop across three GPUs.** X11 can join GPUs into one desktop in two ways, and neither suits the daemon. *Xinerama* joins separate X screens, but on the NVIDIA driver it turns off RandR's per-output interface, which is how the daemon sees monitors and hot-plugs. Alternatively one GPU renders the whole desktop and the others only scan out their part of it (*PRIME output sinks*); that copies every frame across PCIe and depends on driver support. Leaving each GPU as its own X screen works, but windows cannot move between X screens, which defeats the purpose.
- **Driver.** On FreeBSD an RTX 3060 needs NVIDIA's proprietary driver (drm-kmod covers Intel and AMD). How it behaves with several cards, and whether it can act as an output sink, has to be checked on hardware (§10).

So in phase 1 the dependable form is **one GPU with up to four panels**; a wall across several GPUs is an experiment.

**Phase 2 (the new server).** The server opens every GPU through DRM/KMS and keeps one buffer per monitor, so there is no single picture and no desktop-size cap, and because it owns input routing, three cards make one desktop. What remains is hardware: four displays per card, GPU memory for twelve buffers, and a copy between cards whenever a window drawn on one GPU is shown on a panel of another (a window straddling two cards is drawn once and copied). Display-list surfaces (§6.6) help here: the server can render such a window on each card instead of copying pixels. Whether NVIDIA's FreeBSD driver provides the DRM/KMS interface the server needs is also on the list to verify; AMD and Intel cards provide it through drm-kmod.

---

## 6. The long-term architecture: one authority, three contracts

### 6.1 Root problems

1. **Wrong unit.** X counts pixels; Wayland counts pixels times a guessed scale. Neither knows how big things look to the user.
2. **Wrong placement authority.** X lets apps place windows but nobody remembers; Wayland forbids apps from placing and mostly doesn't remember either.
3. **Accessibility bolted on**, through a side channel that broke under Wayland's security model.
4. **Pixels are the only currency**, so magnification is blurry, straddling windows are wrong on one side, and scaling means resampling.

### 6.2 One authority

A single display server owns all outputs and all input, like NeXT's Window Server. It knows each panel from its EDID: physical size, subpixel layout, colour gamut, refresh capabilities. It composes **each monitor in its own buffer**, at that panel's native resolution, so no single picture of the whole desktop exists and X's desktop-size limit (§5) does not apply.

### 6.3 Contract 1: geometry in visual size

- The protocol unit is the **CSS reference pixel**: the visual angle of one pixel at 96 dpi seen from arm's length. Browsers have used this definition for decades. It's the right unit because **viewing distance matters**: a TV across the room, a laptop and a 32″ monitor need different densities for the same apparent size.
- Each monitor's scale is **calculated** from physical size and viewing distance: scale = (ppi ÷ 96) × (viewing distance ÷ 71 cm). Viewing distance is guessed, not asked: a built-in laptop panel is assumed at laptop distance (50 cm), everything else at desk distance.
- **The default is the sharpest scale near the calculated one**: the nearest whole number when it is within 25%, and never below 1. Whole numbers are sharp in every toolkit, even those that only scale by whole numbers, and need no resampling; most people have one ordinary monitor, which then shows its native pixels at 1×. A 103 ppi monitor (calculated 1.07) gets 1×, a 14″ 4K panel (3.28) gets 3×, a 27″ 4K (1.70) gets 2×. Only where no whole number is close (a calculated 1.5) does the exact value stand.
- **The user can choose a screen's own scale**, on a slider marked at the whole numbers and the default, and sets **one** size preference ("how big I like things") that multiplies every screen.
- Applications that support it render **natively per monitor**: told the exact scale, they draw at the final pixel size, so any scale, whole or fractional, is shown 1:1 without resampling.
- **Text size is a separate preference from interface size**, a real accessibility need.

### 6.4 Contract 2: placement, cooperative but owned by the server

- Every window has a **stable identity**: application ID, window role and a persistence token.
- Applications **may request** a position, relative to a monitor or a parent window. The **server decides and remembers**, per monitor set.
- **User rules override both** (for example, "the logbook always opens on the left monitor, top half").
- Unplugging, sleeping and waking never pile windows onto one screen.

### 6.5 Contract 3: meaning, not just pixels

- Each window publishes its **accessibility tree as part of the window protocol**: roles, names, bounds in reference units, focus and caret position.
- The server therefore supports, natively: a **magnifier that follows focus and the text caret**, screen readers by design, **voice control and automation** under explicit permission (§7), and consistent, server-drawn focus highlighting.

**An agent is a client of this contract too.** (A design direction, set down on 10 November 2026; none of it is built.) A software agent acting for the user, whichever one the user chooses, needs what a screen reader and a voice-control tool need: to know what is on the screen, and to act on it. Today's agents work from screenshots and pointer coordinates, because that is all a desktop gives them. Here an agent is given the same tree:

- **It reads meaning, not pixels:** roles, names, values and bounds ("the button named Save, enabled"), and where a window is a display list (§6.6), its text as text. It **acts by name**: press this button, type into this field.
- **Permission is per agent and per application**, under the rule every permission follows (§7.2): granted by the user, shown by an indicator while the agent is at work, revocable, and logged. A password field is no more open to an agent than to any other tool.
- **It has a seat of its own:** its own pointer and its own keyboard focus. An agent at work does not move the user's pointer or type into the user's window, and the user can watch it or carry on beside it.
- **Any agent can be plugged in.** The interface is the server's own, on the session bus (§6.8). An adapter at the edge speaks whatever protocol agents use (the Model Context Protocol is one today), as D-Bus is spoken at the edge.

The control socket the server has for scripted tests (§9) is not this interface: it goes around the security model, and exists only when a developer turns it on. When the agent interface exists, the test tool becomes one of its clients. The work belongs to phase 3, with the rest of this contract.

### 6.6 Optional display-list surfaces

An application may submit either a **bitmap** (always supported) or a **retained vector scene**: a display list of drawing commands in **the server's own minimal, validated format**. Existing serialisations such as Skia's SkPicture were not designed to be read from untrusted processes, so they are not used on the wire; toolkits convert their scenes into this format instead. For display-list surfaces the server can render the window **at any density, at any time**:

- **A window straddling two monitors is sharp on both halves.** No current desktop does this.
- **Magnification is truly sharp:** text at 4× is re-rendered text, not enlarged pixels.
- **Remote display sends drawing commands instead of video**, the network transparency X promised, done properly.

The reference implementation is in Rust, and the server's own tools are its first clients, through the project's toolkit (§6.10), whose scenes are in this format. Its renderer is the project's own unless a borrowed one, such as Vello, is decided to be necessary (§8). **Avalonia is the first outside toolkit to adopt it** (§8.1): it already draws a retained scene, which converts naturally. Display PostScript died because every application had to speak it; here it is an optional upgrade over bitmaps.

### 6.7 Other display problems addressed

- **Colour:** per-monitor colour management from EDID colorimetry, so the same colour looks the same on every monitor; HDR where the panel supports it; colour-blindness filters and night light applied per output.
- **Timing:** each monitor on its own refresh timing (mixed 60/144 Hz, variable refresh rate); tear-free by default, with a low-latency mode for games.
- **Text rendering per panel:** subpixel layout per panel (RGB, BGR, OLED), and hinting decided by density: hinted at low density, unhinted at high density.
- **Pointer:** a cursor that is the same physical size on every monitor, and pointer speed consistent in physical distance across mixed-density monitors.
- **Accessibility settings in one place:** reduced motion, high contrast, large cursor, sticky keys, all honoured by the server.

### 6.8 The session bus: our own core, D-Bus at the edge

A desktop carries more than windows: notifications, settings that apply live, launching and activating applications, accessibility, media keys. On Linux and FreeBSD that traffic goes over the **D-Bus session bus**, a separate daemon through which any program can call any other. It has the same flaws as the window systems in §6.1:

- **It is a side channel.** It knows nothing about windows, monitors or which application the user is looking at. Accessibility (AT-SPI) travels this way, which is why it is an add-on (§1).
- **It trusts every caller.** Any program on the bus can call any service, and broadcast signals go to whoever asks for them.
- **Services start themselves.** D-Bus activation launches a service the first time anyone calls its name. In the first phase 2 session on real hardware, Firefox found no session bus, started a private one, and a notification call activated Xfce's notification daemon, which appeared as an ordinary window in the middle of the desktop.

Vergence uses the same pattern for the bus as for windows: **the core is its own, and compatibility lives at the edge.**

**Core: the server is the bus.** Session services are part of the native protocol, on the same connection as windows, with the same identity and the same permissions (§7). The message types live in the shared protocol crate, beside the window protocol. The first services:

- **Notifications**, drawn by the server: placed under Contract 2, the same size on every monitor (Contract 1), readable by accessibility tools (Contract 3), and quiet when the user asks.
- **Settings**: one source of truth (fonts, theme, cursor, text and interface size). A change is pushed to every subscriber at once, so applications and the server's own title bars follow it live, instead of reading environment variables at launch.
- **Launch and activation**, with activation tokens, so focus moves only on user action (§7.3).
- **Accessibility**: the Contract 3 tree travels over the core bus, not a separate bus.
- Later: media keys and global hotkeys registered by name (§7.3), and screen capture requests under permission.

The rules: **the server always knows who is calling** (socket credentials now, a sandbox identity where one exists, §7.4); **there is no eavesdropping**, and a signal reaches only subscribers entitled to it; **nothing starts on demand** except services the user has installed and declared.

**Edge: a D-Bus front end, written by the project.** Existing applications expect a D-Bus session bus, so the server provides one at the usual address (`DBUS_SESSION_BUS_ADDRESS`). It speaks the D-Bus wire protocol, which is documented and small: authentication over a Unix socket, the message encoding, and the bus's own methods (registering a name, subscribing to signals, looking up a name's owner). It does not run the reference D-Bus daemon. Calls to well-known interfaces are **translated into core services**:

| D-Bus interface | Core service |
|---|---|
| `org.freedesktop.Notifications` | Notifications |
| `org.freedesktop.portal.Settings` (colour scheme, accent, fonts) | Settings |
| AT-SPI (`org.a11y.Bus` and the accessibility bus) | Contract 3, through the AT-SPI bridge (§8.2) |
| Others as measured: screensaver inhibit, application activation | As needed |

Applications that talk to themselves over the bus keep working: a name an application owns is routed to it, so single-instance behaviour such as Firefox opening a link in the window already running still works. A name nobody owns and the edge doesn't translate gets a clean "service unknown" reply, never an automatic start. The edge only translates and routes: apart from which application owns which name, it keeps no state, and permissions are decided in the core.

**Why not run the reference D-Bus daemon at the edge?** It would work, but it keeps the side channel's flaws (no window identity, open broadcasts, activation) and still leaves every service to be written. The subset applications actually use is small, and writing it ties every call to the server's identity and permissions.

**Out of scope:** the D-Bus **system** bus (power, removable media, networking) belongs to the operating system and stays as it is. Nothing here depends on systemd or logind, which FreeBSD doesn't have.

### 6.9 The shell: one shelf

The contracts above are where Vergence is new. The shell, the part of the desktop the user sees around the windows, is deliberately not a second experiment: **a familiar shell on a new foundation.** It should be minimal, and it must still show what is installed and what is running. This section is the design decided on 9 November 2026; none of it is built yet.

**What it borrows, and from where.** Window Maker (1997), the free descendant of NeXTSTEP's desktop, had three ideas worth keeping: one icon for each running application, small live tiles ("dockapps") instead of a tray of unrelated icons, and a menu on the bare desktop. It aged badly because its tiles were a fixed 64 pixels, dockapps were ordinary windows drawn by whatever program supplied them, and nothing showed what was installed. From current desktops Vergence takes the compact application list of macOS, a popup rather than a view that takes over the whole screen.

**One shelf.** A single strip holds everything a top bar and a dock would hold between them, drawn by the server with the same drawing code as the project's toolkit (§6.10). It stands on the **right edge** of the main monitor (the primary screen in the settings app) and is always visible; hiding it until the pointer reaches the edge is an option. Any other monitor can have a shelf of its own, switched on in that screen's settings. Its width is set in reference units (Contract 1), so it is the same physical size on every monitor. From top to bottom it has three parts:

*Revised on 10 November 2026, when the first of it was on a screen:* the shelf is a **dock**, a rounded panel as tall as what it holds, floating a small gap in from the edge so that the desktop shows around it, and **it can be dragged**. Dropped near an edge of any screen it settles along that edge (upright at the sides, level at the top and bottom) and windows keep clear of it; dropped anywhere else it floats where it was put, over the windows. Which it does, where, the gap, and whether windows keep clear are settings. The right edge of the main monitor is where it starts.

- **The apps button**, in the top corner, the easiest target on the screen. It opens a compact popup beside the shelf: a grid of every installed application, with a search field. Typing filters the grid. The same field will later run **named desktop actions** ("move this window to the left monitor", "screens off"), the list that voice control and automation use under permission (§6.5, §7), so there is one command surface for hands, voice and agents.
- **Applications**: pinned and running, one icon each, with a mark beside those that are running. An application with several windows lists them in a small popup. The shelf on the main monitor shows the windows of every monitor, grouped by screen, except those of a monitor that has its own shelf. A minimised window goes to its application's icon; there are no separate icons for minimised windows.
- **Live tiles**: clock, volume, network, battery. A tile is not a small window. Its owner **publishes a value and a set of actions on the session bus (§6.8), and the server draws it** in the desktop's style. Tiles are therefore sharp at any scale, look alike whoever supplies them, are readable by a screen reader (Contract 3), and cannot draw anything misleading in the shelf. Detail (a calendar, a volume slider, a list of networks) opens in a popup beside the tile. Controls have real steps the user can set.

A vertical shelf has no room for text, so names appear when the pointer rests on an icon and in the popups. If the main monitor is unplugged, the shelf moves with the primary role to another screen.

**What the shelf knows that others don't.** Because the server owns placement and remembers it per monitor setup (Contract 2):

- **Away windows.** When a monitor is unplugged, its windows are not piled onto another screen. They are shown on the shelf as away and go back to their places when the monitor returns; the user can call one over at any time.
- **Remembered launch.** Starting an application from the shelf opens it where it last was on this monitor setup.
- **Session restore** (decided 10 November 2026; not built). The server keeps a list of the applications that are running, written whenever it changes, so that it survives a crash as well as an orderly end. When a session starts, each is started again and its windows go back to their places: a kernel is updated, the machine is restarted, and the desktop is as it was left. What was open *inside* an application is the application's to bring back, as a browser brings back its tabs; the project's own tools are told they are being restored, so the terminal can reopen its tabs where they were (the programs that ran in them it cannot). Only applications the system knows by a desktop entry are started again, never a remembered command line. It can be switched off, a session can be started clean, and one that has ended twice soon after being restored starts clean by itself.

**Windows.**

- Windows **float**, and the server remembers where they were put. There is no automatic tiling, which would move windows the user had placed, and there are **no workspaces**: monitors and placement memory do that job, and window memory stays keyed to the monitor setup alone.
- **Snapping**: dragging a window to an edge gives it half the screen, to a corner a quarter, with the shelf counting as the edge. It can be switched off.
- **Keyboard**: Alt+Tab steps through windows, most recently used first, in a small list in the middle of the screen. One key opens the apps popup with the search field ready, so anything can be started or found by typing.
- **Menus stay in each window.** There is no global menu bar.
- **Dialogs** stay attached to the window they belong to and move with it.
- **The desktop** is wallpaper only, with no icons on it; a right-click on it opens a short menu of applications.

### 6.10 The toolkit: the tools' own

The project's own tools (the terminal, the settings app, later a file manager and whatever else the desktop itself needs) are drawn with a **toolkit written by the project**. This was decided on 9 November 2026, after two days of building them on a borrowed one (§8): its software renderer rebuilt a window-sized mask for every run of text, its GPU renderer lost small drawings when multisampling was on, clicks went missing under load because input was judged at the pointer's latest position, and it has no route to Contract 3. Each fault had a workaround; the lesson, the same one the author's music applications taught, is that the workarounds cost more than the part. With two tools to move, the cheapest moment is now.

It is not a general-purpose toolkit. It is what this desktop's tools need, and it carries the design's priorities down to the level where a button is drawn:

- **The scene is the currency.** A tool describes its window as a retained scene, a display list in the format of §6.6, not as pixels. Everything else follows from that one choice.
- **GPU first.** Scenes are rendered on the GPU, in parallel, at the exact scale of the monitor they are on (Contract 1). The CPU draws only where a GPU buys nothing, a dialog that changes once, and as the **reference renderer**: the same scene drawn in software, for tests and for machines without a GPU. The two must agree, which is how the GPU renderer is tested.
- **One renderer, shared with the server.** The code that draws a tool's scene is the code the server uses for its own shelf (§6.9), title bars and notifications, and for display-list surfaces from any application (§6.6). There is one implementation of text, one of shapes, one glyph cache design, and one place to fix a drawing fault.
- **Remote display falls out of it.** A tool that produces a scene can send that scene instead of video (phase 4). The toolkit sends what changed, in drawing commands, with text as text.
- **Meaning is built in.** Every widget publishes its role, name, value and bounds as it is laid out (Contract 3). A tool cannot be built on this toolkit without an accessibility tree, and the same tree is what named desktop actions (§6.9), voice and automation address.
- **Isolation.** Reading a font, decoding a picture and parsing what a program prints are done on **untrusted input**. The toolkit keeps such work off the thread that owns the window and its input, and where the platform allows (Capsicum on FreeBSD, the equivalents on Linux) in a process that can do nothing else. A tool's interface stays responsive whatever its content is doing: input is handled in order at the place it happened, and never waits for drawing.
- **Security.** A tool gets nothing a third-party application would not get (§7): no global input, no reading other windows, no placing itself. It reaches the desktop through the same protocol and the same permissions, so the toolkit is also the proof that the protocol is enough.
- **Size and placement come from the server.** The toolkit has no scale setting and no window-position code of its own. It lays out in reference units and draws at whatever density it is told.

**What it stands on.** For now the toolkit talks to the server as a Wayland client, in code written by the project as the D-Bus edge was (§6.8), with the native window protocol (§6.3 to §6.5) taking its place behind the same interface when that exists. Font files are read and keyboards mapped with the parts already in use (§8) until each is decided on. **Text is the hard part** of any toolkit: shaping for scripts that need it, right-to-left text, fonts that fall back per character, input methods. Plain text comes first; the rest is named in §10 rather than assumed.

**The order.** The drawing and text core comes first, with the **terminal** as its first user: a terminal is a grid of cells, needs the fewest widgets, and is where the borrowed toolkit hurt most. The **shelf** (§6.9) is then built in the server on the same drawing code. Widgets follow (label, button, switch, slider, list, text field, scrolling), and the **settings app** moves when they can carry it. The borrowed toolkit is removed when the second tool has moved.

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
| Accessibility and automation (Contract 3) | A screen reader is technically a full screen reader and keylogger | Permissions for specific tools, with an indicator. **Password fields are protected**: contents never leave the app; tools see only "secure field focused". Automation can be scoped per application. Audit log. |
| Agents (Contract 3, §6.5) | An agent that can read the tree and act is as able as the user, and can be misled by what it reads on the screen | Grants per agent and per application, with the indicator and the audit log. A seat of its own, so its input is never taken for the user's and can be stopped without stopping the user. Password fields protected as for any tool. Actions that spend, send or delete can be made to wait for the user's own confirmation. |
| Display-list surfaces | The server interprets untrusted drawing commands | Strict validated format; per-frame complexity budgets; rasterisation in a **separate sandboxed process**: seccomp/Landlock on Linux, **Capsicum on FreeBSD**. |
| Legacy X11 apps | X clients can spy on each other within one X server | They cannot see native apps. Untrusted X apps can each run in their own isolated rootless X server. |
| Screen capture | Silent screen recording | Permission plus indicator, scoped to a window or a monitor. |
| Clipboard | Background clipboard sniffing | Read only on the user's paste or by the focused app; clipboard managers need a permission. |
| Input injection | Fake input | Permission plus indicator (remote desktop, voice control). |
| Global hotkeys | Apps keylogging to catch shortcuts | Hotkeys **registered with the server by name and approved by the user**. Push-to-talk keys for radio and voice tools work properly. |
| Session bus (§6.8) | Programs calling each other's services, sniffing broadcast signals, or starting services silently | The server knows every caller; no eavesdropping, and signals reach only entitled subscribers; no on-demand activation beyond declared services; the D-Bus edge exposes only translated interfaces and names an application owns. |
| Remote display | Network eavesdropping and hijack | Always authenticated and encrypted (SSH or WireGuard); never unauthenticated TCP. |
| Phase 1 daemon (X11) | Creating a new hole | No network socket, no unauthenticated control interface; the published property holds only monitor scales; saved layouts stored with owner-only permissions (0600). |

### 7.4 An honest limit

On a typical desktop, any program running as the user can already read the user's files and attach a debugger to their other programs. **A display server cannot create isolation that the operating system doesn't provide.** Its job is to **not be the weak link**. Real application-to-application isolation needs sandboxing (Flatpak on Linux, jails and Capsicum on FreeBSD), which also gives the server a **trustworthy application identity** to attach permissions to. Without a sandbox, identity is only the executable path, which is weaker. The design supports both and is explicit about the difference.

---

## 8. Implementation strategy

**No new drivers.** The server builds on the shared lower layers that **both Linux and FreeBSD** provide: DRM/KMS for display, the kernel's input devices (evdev), GBM, EGL and Vulkan for rendering (§8.3).

**Our own parts.** The project writes its own core. A part from another project (a library, a program, a protocol extension added only so borrowed clients work) comes in only when it is truly necessary, and only by explicit decision of the project lead, with the reason and the cost of writing our own on record. The operating system's own interfaces (above) and the compatibility edges' protocols are not borrowing; implementations of them are. This replaces an earlier "borrow first, replace later" approach. Parts borrowed while bringing the server up stay until each is decided on:

| Part | Used for | Status |
|---|---|---|
| Smithay | The server's Wayland front end, DRM/KMS and input back ends, renderer | Borrowed for phase 2 bring-up; moving to a support role (decided 8 November 2026): the renderer is replaced with the project's own first, then the DRM/KMS and input back ends; the Wayland front end may stay as an edge |
| libinput, libseat/seatd, xkbcommon | Input devices, seat and device access, keymaps | Borrowed through Smithay |
| Xwayland | X11 applications (the X11 edge) | Borrowed; an X server of our own is a large project |
| iced | The settings app (and, until 10 November 2026, the terminal) | Borrowed (MIT); chosen over Slint on 8 November 2026. **Being replaced** by the project's own toolkit (§6.10, decided 9 November 2026): the terminal moved first, on 10 November; the settings app follows, and iced is removed with it. Until then it carries two patches and workarounds (its software renderer's clip mask, multisampling off) |
| ab_glyph, resvg | Title-bar text, SVG wallpapers | Borrowed; small, candidates for our own |
| Adwaita cursor theme | The pointer | Borrowed artwork, read at run time |

**Language: Rust throughout.** Everything the project itself ships is written in Rust:

- **Phase 1 daemon:** Rust over XCB/RandR (the `x11rb` crate). Its profile, EDID and rules logic lives in a shared crate that moves unchanged into the server.
- **Server core:** Rust. A compositor needs steady frame timing, which rules out garbage-collected runtimes.
- **The toolkit and the tools** (terminal, settings, the shelf's drawing): Rust, on the project's own toolkit (§6.10). The terminal is on it; the settings app uses iced until it is moved.
- **Display-list reference implementation:** Rust (§6.6).

Why one language:

1. **FreeBSD from day one.** Rust builds natively on FreeBSD today. .NET works on FreeBSD (AlbumWall, Deadwax and Marginal all run there as Native AOT builds made with the FreeBSD port's own ILCompiler), but only through the FreeBSD ports' .NET 9, a version behind what our apps target and outside Microsoft's official platforms. System components of the desktop itself should not depend on that.
2. **System components must be small and start instantly.** Settings panels and tray indicators are part of the desktop, not applications, and are packaged with it.
3. **One codebase.** Profile and EDID logic, the configuration format and the protocol types are shared between daemon, server and tools, with no second toolchain or cross-language bindings.

**Compatibility at the edges.**

- **Wayland applications** connect through a standard Wayland front end.
- **X11 applications** run through rootless Xwayland. Because the server owns input routing, legacy windows can be **scaled per monitor with correct input mapping**, which removes the input wall from §5 without any X extension.
- **D-Bus clients** reach a D-Bus front end written by the project, which translates notifications, settings and accessibility into the server's own session bus (§6.8).
- The server speaks Wayland, X11 and D-Bus at the edges, but placement, memory, units, accessibility and security follow this design.

**Platforms.** Development happens on a Linux without systemd: Void Linux on a small desktop with Intel UHD 630 graphics, the machine that ran FreeBSD 15 as the primary platform until November 2026. FreeBSD taught the project most of what §9 lists and stays a real-hardware platform on a second machine of the same kind; the order was changed because the work ahead, GPUs and drivers, several cards, explicit sync, a suspend-and-wake soak, is where FreeBSD's ported DRM drivers lag and its tools are thin (no nouveau, no rtcwake, and a GPU hang whose cause is still open, §9). The second Linux is Fedora on a workstation with Intel UHD 770 graphics; there the server meets logind, udev and the in-kernel DRM drivers instead of seatd and devd, and the session starts from the display manager. A FreeBSD virtual machine builds and tests every change in between. Where the platforms must differ, as with hot-plug events (§4.7) or seat management, the code and the documents say so.

### 8.1 Running our .NET/Avalonia applications

The author's existing applications (AlbumWall, Deadwax, Marginal, the W2 and LP-100A monitors and others) are .NET/Avalonia. They are first-class citizens on this desktop, in three steps:

1. **From day one, unchanged.** Avalonia runs on X11, so the applications run through Xwayland with per-window scaling and correct input. With the phase 1 daemon they already get correct per-monitor scaling, because Avalonia supports per-monitor scale factors on X11.
2. **Live updates with the daemon.** A small .NET helper library reads the daemon's published state (the root-window property, §4.2), so the applications follow monitor and profile changes live without restarting.
3. **Native backend: Avalonia as the first adopting toolkit.** Avalonia's platform layer is pluggable, so an Avalonia backend can speak the server's native protocol directly: placement requests with stable window identity (Contract 2), the accessibility tree from Avalonia's automation peers (Contract 3), and display-list surfaces converted from Avalonia's scene into the server's validated format (§6.6). This is a contained, highly visible piece of work, and it makes Avalonia the proof that the protocol works for a real cross-platform toolkit with real users, not just for the server's own Rust tools.

On FreeBSD, our .NET applications already run: AlbumWall, Deadwax and Marginal all run as Native AOT builds on a FreeBSD 15.1 test machine (November 2026), made by the FreeBSD port's own ILCompiler, with no Linux compatibility libraries. Shipping them waits for the FreeBSD ports to reach .NET 10, so all platforms run the same .NET. The desktop itself does not wait.

### 8.2 Major third-party applications: Firefox and Thunderbird

Firefox and Thunderbird are GTK applications on Linux and FreeBSD with mature native Wayland support. Among everyday applications they are expected to behave among the best.

**Phase 1 (X11 daemon).**

- **Layout memory works.** The daemon acts on their windows from outside, by `WM_CLASS` (`firefox`, `thunderbird`). Both draw their own title bar by default (tabs in the title bar), so `_GTK_FRAME_EXTENTS` handling (§4.4) is required, or windows drift by the width of the invisible shadow.
- **Several Firefox windows are the hard case.** They share one class, and titles change with the active tab. Restoring a window to the right *place* is easy; deciding *which* window belongs in which place is fuzzy, because Firefox's own session restore reopens windows in its own order. Heuristics (window order, size, title at save time) get close; exact matching needs stable identity (phase 2).
- **Scaling.** In Mode A they get the same scale as everything else and are consistent everywhere. In Mode B they take one global value (as GTK does) and are the wrong size on some monitor. Firefox's `layout.css.devPixelsPerPx` is also global and doesn't help.

**Phase 2 onward (the new server).**

- **Native Wayland clients.** This is their best mode: Firefox supports per-monitor fractional scaling on Wayland and is sharp on every monitor, unlike applications that fall back to Xwayland.
- **Placement.** They don't request positions, but the server owns placement and remembers them by app ID. Telling several windows of one application apart needs a persistence token, which depends on Mozilla adopting the Wayland session-restore protocol (§10).
- **Accessibility.** Both have rich accessibility trees, exposed through AT-SPI. Bringing them into Contract 3 needs an **AT-SPI bridge** in the server; Mozilla is not expected to write a native backend, at least early on.
- **Surfaces.** They remain bitmap surfaces (GPU-rendered by Firefox's WebRender): fully supported, without the sharp-at-any-density benefits of display lists.
- **Security.** Clipboard on paste, focus on user action and permissioned capture match how they already behave on Wayland, so the model costs them nothing.

**FreeBSD.** Both are well-maintained ports with X11 and Wayland builds; nothing here is Linux-specific.

*Expectations as understood at the time of writing; to be verified (see §10).*

### 8.3 GPUs and drivers

The development machine has Intel integrated graphics. Other machines will have AMD or NVIDIA graphics, some on open-source drivers and some on proprietary ones, and some will have two GPUs at once. "No new drivers" still holds: the server uses the interfaces the operating system's drivers already provide (KMS for display, GBM for buffers, EGL and Vulkan for rendering, sync objects for timing). What differs between drivers is which of those interfaces they implement, and how well.

**The driver landscape:**

| GPU | FreeBSD | Linux |
|---|---|---|
| Intel | drm-kmod (a port of Linux's DRM drivers) + Mesa | in-kernel DRM + Mesa |
| AMD | drm-kmod (amdgpu) + Mesa | in-kernel DRM (amdgpu) + Mesa |
| NVIDIA, open source | **none**: FreeBSD has no nouveau | nouveau + NVK (Mesa); slower, newer cards first |
| NVIDIA, proprietary | NVIDIA's driver + its DRM module for KMS | NVIDIA's driver; KMS only when its DRM module is loaded with `modeset=1` |

Everything on Mesa shares one path: GBM buffers, EGL rendering, and implicit synchronisation, where the kernel makes sure a buffer is finished before anyone reads it. The phase 2 server runs on this path today, so Intel and AMD on either system should need little beyond testing. NVIDIA's proprietary driver is a different stack:

- **Display needs KMS switched on.** Without `modeset=1` the driver drives no monitor through KMS, and the server can't start. On a Debian 13 machine with an RTX 3060 on driver 550, used here for reference, the DRM module is loaded but nothing switches KMS on, which is the default for an install that only computes.
- **Buffers through GBM.** NVIDIA supports GBM on Linux since driver 495, through its own GBM back end (Debian packages it as `libnvidia-egl-gbm1`). Whether NVIDIA's FreeBSD driver offers GBM well enough for a display server is not yet known (§10). If it doesn't, the server needs a second way to get scan-out buffers: EGL streams or direct allocation through NVIDIA's EGL device interface, written by the project.
- **Explicit synchronisation.** NVIDIA doesn't do implicit sync. Without explicit sync (DRM sync objects in the kernel, and a protocol through which clients hand the server a fence with each buffer), Xwayland and GL applications flicker or show half-drawn frames. NVIDIA supports explicit sync from driver 555, so Debian 13's driver 550 predates it.

**Design decisions:**

1. **Explicit sync is part of the native protocol from the start.** Every buffer a client submits carries an acquire point and gets a release point. Implicit sync stays only as a fallback at the compatibility edges, for drivers and clients that don't provide fences. It is the right design on every GPU, not just a workaround for one vendor: the server knows exactly when each buffer is ready, which also helps frame timing (§6.7).
2. **One renderer per GPU.** The server already composes each monitor in its own buffer (§6.2). Each buffer is rendered on the GPU that drives that monitor. When an application draws on another GPU, its buffer is copied across once per frame, and the server tells the application which GPU sits under its window, so it can move there. This covers hybrid machines (integrated plus discrete), and multi-card walls (§5.1).
3. **Capabilities are probed, never assumed by vendor.** Formats, modifiers, sync support and GBM are each queried from the driver at start-up and logged, and the server picks its path from the answers, not from the vendor's name. A driver that improves gets used fully without a code change.
4. **Display lists need Vulkan compute** (§6.6). Intel and AMD have it through Mesa on both systems, and NVIDIA's proprietary driver on Linux. On FreeBSD with NVIDIA it is to be checked (§10). Bitmap surfaces never need it.
5. **The server survives a GPU reset.** When the kernel resets a hung GPU, every GL context on it may be lost. On Hubble a reset caused by a client left the server drawing black while every GL call still reported success. The server's renderer uses robust contexts, checks reset status every frame, and on a reset rebuilds its context and re-uploads its textures without a restart. A client that hangs the GPU is logged by name.

**Test hardware, planned.** A dual-boot FreeBSD and Linux machine (likely Fedora, as on the Linux test workstation) with an Intel UHD 630 and an NVIDIA RTX card covers integrated graphics, proprietary NVIDIA on both systems, nouveau/NVK on Linux, and the hybrid two-GPU case in one box. AMD comes later. A working NVIDIA compute machine is used only for reading driver facts, not for display tests.

---

## 9. Roadmap

| Phase | Deliverable | Value on its own | Estimated effort |
|---|---|---|---|
| **1** | X11 consistency daemon: EDID profiles, scaling modes A/B, toolkit settings, layout memory | Fixes daily annoyances on any X11 desktop, FreeBSD and Linux | Days to weeks |
| **2** | Display server, first nested in a window, then on real hardware: native per-monitor rendering, placement memory, reference-unit scaling, per-window scaling of X11 apps with correct input, the session bus with its D-Bus edge (§6.8), the project's own toolkit and the tools on it (§6.10), the shell (§6.9) | A usable personal desktop that beats X11 and Wayland on consistency | Months |
| **3** | Accessibility contract and display-list surfaces: Rust reference implementation, then the native Avalonia backend (§8.1) | Sharp magnification, sharp straddling windows, accessibility by design | Research |
| **4** | Remote display over display lists | Efficient, sharp remote desktops | Research |

**Status, 9 November 2026.**

- **Phase 1** is built through its fourth milestone (profiles, layout memory, window memory, scaling) and runs on FreeBSD. It is frozen except for blockers, because X11's walls (§5) cost more effort than they teach.
- **Phase 2** runs on real hardware as the daily session on FreeBSD 15.1 with Intel UHD 630 graphics, on two monitors of very different density: a 14″ 4K panel at scale 3 and an ordinary 1080p monitor at 1, the defaults from their EDIDs (calculated 3.28 and 1.07, snapped to whole numbers as Contract 1 says). Wayland applications draw natively at each monitor's scale. X11 applications run through rootless Xwayland at the densest monitor's scale and are shown smaller on the others; a smoothing filter for that shrink was tried and rejected because it softened text. The server draws title bars, places new windows on the monitor under the pointer, remembers window places per monitor setup (a first piece of Contract 2), follows monitors being plugged in and out, and has a control socket for scripted tests. The session bus (§6.8) works: its core (monitors, settings and notifications on the project's own protocol) and a D-Bus edge written by the project, which provides the settings portal (applications follow dark or light and the accent live) and draws notifications itself. A settings app (iced, a sidebar of pages) sets appearance, each screen's wallpaper (one screen's is the default), each screen's scale and the arrangement of the screens, all applied live. On 8 November it also ran on Linux for the first time, on Fedora with Intel UHD 770 graphics, started from the GDM login screen, with the same monitors at 60 Hz and the same scales; the only changes needed were the peer-credential call (glibc has no `getpeereid`) and seat access through logind instead of seatd. Windows, placement and accessibility on the native protocol, and explicit sync (§8.3), come next. The shell (§6.9: one shelf, an apps popup, live tiles) was designed on 9 November and is not built yet; until it is, applications start from the menu on the desktop.

**The order of work in phase 2, from 9 November 2026.** The server has been installed as a session on Void Linux since that day (§8), with screens-off and sleep, window borders, and the terminal and settings app as its first tools. What comes next, in order:

1. **The toolkit's drawing and text core, and the terminal on it** (§6.10; the terminal moved on 10 November 2026): scenes in the display-list format, a GPU renderer with a CPU reference renderer, the project's own Wayland client code. This moved ahead of the shell when the borrowed toolkit's faults were measured under load: with six busy tabs in a window of 3792 × 2118 pixels its software renderer held one processor core and nothing else, and its GPU renderer was ten to fifteen times faster on the same load but lost the window's buttons.
2. **The shell** (§6.9), built in the server on the same drawing code: the shelf, the apps popup, running and pinned applications, then live tiles; snapping and keyboard switching with it.
3. **Widgets, and the settings app on them**; the borrowed toolkit removed.
4. **Windows, placement and accessibility on the native protocol**, which the toolkit then uses instead of Wayland, and explicit sync and the rest of §8.3 alongside as hardware allows.

The display-list renderer was phase 3 work. Step 1 brings its first version forward into phase 2, because the tools need it; what stays in phase 3 is the protocol side (applications submitting scenes to the server), the accessibility contract on the wire, and Avalonia.

**What FreeBSD taught phase 2.** As in phase 1 (§4.7), the real machine showed things a Linux-first build would have missed:

- There is no logind, so seat and device access go through seatd. A display manager's Wayland sessions proved unreliable without logind, so the session starts from a console login.
- Every query of a DRM connector's state took about 327 ms on FreeBSD, and polling connectors stalled the whole server, freezing dragged windows every few seconds. Hot-plug is detected from devd's DRM events instead, and connectors are never polled.
- FreeBSD has no abstract Unix sockets, so Xwayland can only be reached through its socket file, and the first X application waits until Xwayland is ready.

**Where the real costs are.** Writing the code is not the bottleneck. The bottlenecks are:

- **real-hardware testing:** GPUs, monitors with unusual EDIDs, docks, sleep and wake. Most of it can be automated (§4.8), which turns weeks of manual testing into overnight runs; what remains manual is real docks and KVM switches;
- **the user's time and judgment:** what "the right size" means to their eyes;
- for phases 3–4, **toolkit adoption**, which only a working demonstration can start.

---

## 10. Open questions and items to verify

1. Exact live-update behaviour of Qt 6 and Chromium scaling on X11.
2. Avalonia 12's X11 per-screen scaling variables and behaviour.
3. Reliability of EDID physical-size data across real monitors, and the fallback when it is missing. In the linuxhw database 0.3% report no size at all, 0.4% an implausible one, and 6.6% give a millimetre size more than 2 cm away from their own centimetre size.
4. How to capture viewing distance simply (presets: laptop, desktop, TV; or a one-time calibration). *Partly answered (8 November 2026): it is guessed from the connector (built-in panels at laptop distance), and users change the result, a screen's scale, rather than the distance.*
5. Status of Wayland session-restore and placement protocols, and whether the server should implement them for compatibility.
6. Display-list format: the minimal validated command set (paths, fills, strokes, gradients, images, text runs with font references, clips, layers), and how text is shipped (glyph runs plus font identity, so the server can re-render text sharply at any density).
7. FreeBSD specifics: DRM/KMS driver coverage for the target GPUs beyond Intel (§8.3); libinput and seat management; a suspend-and-wake loop for testing.
8. The tools' user interface. *Answered (9 November 2026): the project's own toolkit (§6.10) replaces iced.* Open within it: text shaping for scripts that need it, right-to-left text and input methods, and whether font reading and shaping are written by the project or borrowed; which GPU interface the renderer uses first (OpenGL ES through EGL, which every target has today, or Vulkan); and how much isolation each platform allows for decoding and parsing.
9. The scope of the native Avalonia backend: which parts of Avalonia's platform interfaces it must implement, and how its automation peers map to Contract 3.
10. Firefox and Thunderbird: confirm that Firefox on X11 has no per-monitor scaling; check whether Mozilla implements (or plans) the Wayland session-restore protocol; how well heuristics match several Firefox windows to saved layouts; the scope of an AT-SPI bridge into Contract 3.
11. Many monitors on several GPUs (§5.1): NVIDIA's FreeBSD driver with more than one card (Xinerama versus PRIME output sinks for phase 1; its DRM/KMS support for phase 2); how many monitors report a real EDID serial number (the linuxhw database can answer this), since identical panels without one are told apart only by port.
12. GPUs and drivers (§8.3): whether NVIDIA's FreeBSD driver provides GBM, KMS and DRM sync objects well enough for the server, and Vulkan compute for display lists; which nouveau/NVK generations are usable on Linux; the cost of copying each frame between GPUs on a hybrid machine; AMD on FreeBSD's drm-kmod, on real hardware.
13. Session bus (§6.8): which D-Bus interfaces everyday applications (Firefox, Thunderbird, GTK and Qt applications, Avalonia) actually call, measured with a logging edge before anything is translated; which of them only warn when a service is missing and which fail; the scope of the AT-SPI bridge on the bus side. *First measurement (8 November 2026, Firefox, Slint, Avalonia): the settings portal (now provided), GNOME's idle monitor (Firefox polls it), gvfs, screensaver inhibit, Ubuntu's menu registrar and the accessibility bus. None of the applications failed without them.*
14. The agent interface (§6.5): the observations and actions it offers, and how they follow from the accessibility tree; how a grant is scoped (an application, a window, a kind of action); a seat for each agent in the server (how its input is routed, what holds its focus, what the user sees of it); which actions wait for the user's confirmation, and how an agent misled by text on the screen is kept from harm; the protocol at the edge.
15. Session restore (§6.9): how the project's tools are told they are being restored and what they keep (the stable window identity of Contract 2 is the natural key); how much of the Wayland session-management protocol and X11's are worth speaking at the edge for other toolkits; how an application the user closed on purpose just before the end is told from one the end took with it.

---

## Appendix A: Glossary

- **EDID:** data a monitor reports about itself: manufacturer, model, serial, physical size, supported modes, colour characteristics.
- **RandR:** the X11 extension for querying and configuring outputs (monitors).
- **EWMH:** Extended Window Manager Hints; the standard by which clients and tools talk to X11 window managers.
- **XSETTINGS:** an X11 protocol for publishing desktop settings (fonts, scale) that toolkits read live.
- **CSS reference pixel:** the visual angle of one pixel at 96 dpi seen from about arm's length (roughly 0.0213°).
- **Compositor:** the component that combines window images into the final screen image.
- **Agent:** a program that acts for the user by reading what applications show and operating them; here a permitted client of Contract 3 with a seat of its own (§6.5).
- **Display list:** a recorded sequence of drawing commands that can be replayed at any resolution.
- **KMS/DRM:** the kernel interfaces for display mode-setting and GPU access, on Linux and FreeBSD.
- **GBM:** Generic Buffer Management, the interface through which a display server allocates GPU buffers it can show on a monitor.
- **Mesa:** the open-source implementation of OpenGL, Vulkan and GBM used by Intel, AMD and nouveau drivers.
- **Explicit sync:** passing a fence with each buffer that says when the GPU has finished with it, instead of leaving the kernel to work that out (implicit sync).
- **iced:** a cross-platform GUI toolkit for Rust, with a GPU renderer (wgpu) and a software one (tiny-skia); used by the first tools and being replaced (§6.10); the terminal left it on 10 November 2026.
- **Toolkit:** the code an application draws its buttons, lists and text with. Vergence's own is described in §6.10.
- **Multisampling:** smoothing the edges of drawn shapes by sampling each pixel several times on the GPU.
- **Smithay:** a Rust library for building Wayland compositors.
- **Vello:** a Rust GPU renderer for 2D vector graphics.
- **x11rb:** Rust bindings for the X11 protocol (XCB).
- **Capsicum:** FreeBSD's capability-based sandboxing framework.
- **Shelf:** Vergence's single strip on a screen edge, holding the apps button, application icons and live tiles (§6.9).
- **Window Maker:** a window manager for X11 (1997) modelled on NeXTSTEP's desktop, with a dock of application icons and small live "dockapps".
- **D-Bus:** the message bus Linux and FreeBSD desktops use between programs; the *session* bus serves one user's login, the *system* bus the whole machine. *Activation* starts a service when its name is first called.
- **AT-SPI:** the accessibility interface on Linux and FreeBSD desktops, carried over D-Bus.

## Appendix B: One sentence

*NeXT's single authority and device independence, Windows' placement memory, Wayland's per-monitor rendering, and the web's reference pixel, with accessibility and security part of the protocol rather than add-ons.*
