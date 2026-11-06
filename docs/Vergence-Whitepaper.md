---
title: "Vergence"
subtitle: "A display architecture for right-sized, right-placed, accessible applications on Linux and FreeBSD"
author: "David Erickson (AB0R), with Claude"
date: "5 November 2026 — Draft 0.5"
---

# Vergence

**A display architecture for right-sized, right-placed, accessible applications on Linux and FreeBSD**

David Erickson (AB0R), with Claude · Draft 0.5 · 5 November 2026

---

## Abstract

*Vergence: the movement of both eyes converging on one point so that you see one clear image. Here, every monitor and every toolkit converge on one answer. (The project was called App Consistency until Draft 0.4.)*

On today's Unix-like desktops, an application's size on screen depends on which monitor it happens to be on, which toolkit it was written with, and which environment variables were set when it started. Where it appears depends on whether the window system lets it choose, and on whether anything remembers where you last put it. Usually nothing does.

This paper proposes a path to **app consistency**: every application the right size, in the right place and readable, on any monitor setup, for every user, including users who depend on accessibility tools. It starts with something buildable in days on an ordinary X11 desktop: a daemon that recognises monitor setups by their EDIDs, applies consistent scaling across toolkits, and remembers window layouts per setup. It ends with a long-term design for a new display server. That server rests on one authority and three contracts: geometry in visual size, placement owned by the server but negotiated with apps, and accessibility built into the protocol. Optional vector "display-list" surfaces revive the best idea of NeXTSTEP's Display PostScript.

The design targets **Debian** as the primary platform and **FreeBSD** as a second platform, tested at every phase.

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
4. **Compatibility at the edges.** Existing X11 and Wayland applications keep working; new capabilities are optional upgrades.
5. **Accessibility and security are part of the design**, not afterthoughts, and neither is allowed to break the other.
6. **Two platforms from day one:** Debian and FreeBSD.

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

**Mode A, "Uniform" (the macOS approach; default).** Render the whole desktop at the highest scale needed and use `xrandr --scale` to scale lower-density monitors down. Every toolkit uses the same scale, so everything is the same size everywhere. The costs are GPU work and some softness on scaled-down monitors.

**Mode B, "Native" (sharpest).** No GPU scaling; each monitor at its native resolution; per-monitor factors go to toolkits that support them (Qt, Avalonia). Those apps are perfect. Toolkits that support only one global value (GTK, Electron, Java) cannot be right on every monitor.

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

- **Virtual screen:** 14,976 × 3,384 pixels, just under the common 16,384-pixel X/GPU limit. A fourth monitor would exceed it.
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

Read EDIDs, hash each setup into a profile key, and log RandR hot-plug events. Small, safe, and the foundation for everything else. It is built and tested on real hardware from the start, on the Debian workstation with real monitors, with FreeBSD alongside: a virtual machine's synthetic EDIDs and hot-plug behaviour would only be worked around, not learned from.

### 4.8 Testing on real hardware, automated

Real-hardware testing is the slow part of phase 1 (§9), so most of it is scripted. The test harness is a separate tool; it may need root, the daemon never does.

1. **Plug and unplug without touching a cable.** On Linux, the kernel's DRM layer gives every connector two debugfs controls: `force` makes it look plugged in or unplugged, with the hot-plug event that goes with it, and `edid_override` makes it report any EDID. The daemon can then be run against thousands of real monitors' EDIDs from the public linuxhw EDID database, including broken ones (zero or wrong physical size, missing serial numbers, identical twins).
2. **Sleep and wake, overnight.** `rtcwake` suspends and wakes the machine in a loop; after every cycle the profile and the layout must be unchanged.
3. **Same physical size, measured.** Small test programs in GTK, Qt, Avalonia, Electron and Java each draw a box of known logical size; the harness screenshots every monitor and measures each box in millimetres using the EDID's physical size. The user's judgment of "the right size" is needed once, as a calibration, not on every run.
4. **Layout memory, scripted.** Windows (several Firefox windows and Thunderbird included) are placed, a monitor is unplugged and replugged through (1), the applications restart, and every window's position and size is compared with what was saved, with a check for drift of a few pixels per restore.
5. **Record and replay.** Every RandR event and EDID seen on real monitors, docks and KVM switches is recorded once; the recordings become tests that run anywhere, with no monitors attached.

What stays manual: the one-time size calibration, occasional real docks and KVM switches, and FreeBSD if its DRM drivers lack the debugfs controls (then real cables plus the replayed recordings). The details are in the milestone 1 plan.

---

## 5. Native per-monitor rendering, and the X11 wall

The ideal is to **separate the logical space** (where windows live, where the pointer moves, where layouts are saved) **from each monitor's native resolution**, so every monitor always shows native pixels. Windows and Wayland both work this way.

On X11 this works in tiers:

1. **Toolkits that support per-monitor scaling** (Qt, Avalonia) are native everywhere with the daemon alone (Mode B). Only a window straddling two monitors is resampled on one side, as on Windows and macOS.
2. **Other applications** can be drawn at one scale and have a compositing window manager scale **just their windows** to suit each monitor. This is Windows' approach.
3. **The wall is input.** If the compositor shows a window at 0.75 size, X still routes clicks by the window's real geometry, so clicks land in the wrong place. X11 has no per-window input transformation. It was prototyped (Sun's Looking Glass input redirection, Compiz experiments) but never merged.

On X11, removing the wall would need a compositing window manager **plus a new X server extension**; the actively developed XLibre fork of Xorg is the most plausible home for one. The long-term architecture below removes the wall a different way: **its server owns input routing**.

---

## 6. The long-term architecture: one authority, three contracts

### 6.1 Root problems

1. **Wrong unit.** X counts pixels; Wayland counts pixels times a guessed scale. Neither knows how big things look to the user.
2. **Wrong placement authority.** X lets apps place windows but nobody remembers; Wayland forbids apps from placing and mostly doesn't remember either.
3. **Accessibility bolted on**, through a side channel that broke under Wayland's security model.
4. **Pixels are the only currency**, so magnification is blurry, straddling windows are wrong on one side, and scaling means resampling.

### 6.2 One authority

A single display server owns all outputs and all input, like NeXT's Window Server. It knows each panel from its EDID: physical size, subpixel layout, colour gamut, refresh capabilities.

### 6.3 Contract 1: geometry in visual size

- The protocol unit is the **CSS reference pixel**: the visual angle of one pixel at 96 dpi seen from arm's length. Browsers have used this definition for decades. It's the right unit because **viewing distance matters**: a TV across the room, a laptop and a 32″ monitor need different densities for the same apparent size.
- Each monitor's scale is **calculated** from physical size and viewing distance. The user sets **one** preference ("how big I like things"), and every monitor follows.
- Applications that support it render **natively per monitor**.
- **Text size is a separate preference from interface size**, a real accessibility need.

### 6.4 Contract 2: placement, cooperative but owned by the server

- Every window has a **stable identity**: application ID, window role and a persistence token.
- Applications **may request** a position, relative to a monitor or a parent window. The **server decides and remembers**, per monitor set.
- **User rules override both** (for example, "the logbook always opens on the left monitor, top half").
- Unplugging, sleeping and waking never pile windows onto one screen.

### 6.5 Contract 3: meaning, not just pixels

- Each window publishes its **accessibility tree as part of the window protocol**: roles, names, bounds in reference units, focus and caret position.
- The server therefore supports, natively: a **magnifier that follows focus and the text caret**, screen readers by design, **voice control and automation** under explicit permission (§7), and consistent, server-drawn focus highlighting.

### 6.6 Optional display-list surfaces

An application may submit either a **bitmap** (always supported) or a **retained vector scene**: a display list of drawing commands in **the server's own minimal, validated format**. Existing serialisations such as Skia's SkPicture were not designed to be read from untrusted processes, so they are not used on the wire; toolkits convert their scenes into this format instead. For display-list surfaces the server can render the window **at any density, at any time**:

- **A window straddling two monitors is sharp on both halves.** No current desktop does this.
- **Magnification is truly sharp:** text at 4× is re-rendered text, not enlarged pixels.
- **Remote display sends drawing commands instead of video**, the network transparency X promised, done properly.

The reference implementation is in Rust, rendering with Vello, and the server's own tools are its first clients. **Avalonia is the first outside toolkit to adopt it** (§8.1): it already draws a retained scene, which converts naturally. Display PostScript died because every application had to speak it; here it is an optional upgrade over bitmaps.

### 6.7 Other display problems addressed

- **Colour:** per-monitor colour management from EDID colorimetry, so the same colour looks the same on every monitor; HDR where the panel supports it; colour-blindness filters and night light applied per output.
- **Timing:** each monitor on its own refresh timing (mixed 60/144 Hz, variable refresh rate); tear-free by default, with a low-latency mode for games.
- **Text rendering per panel:** subpixel layout per panel (RGB, BGR, OLED), and hinting decided by density: hinted at low density, unhinted at high density.
- **Pointer:** a cursor that is the same physical size on every monitor, and pointer speed consistent in physical distance across mixed-density monitors.
- **Accessibility settings in one place:** reduced motion, high contrast, large cursor, sticky keys, all honoured by the server.

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
| Display-list surfaces | The server interprets untrusted drawing commands | Strict validated format; per-frame complexity budgets; rasterisation in a **separate sandboxed process**: seccomp/Landlock on Linux, **Capsicum on FreeBSD**. |
| Legacy X11 apps | X clients can spy on each other within one X server | They cannot see native apps. Untrusted X apps can each run in their own isolated rootless X server. |
| Screen capture | Silent screen recording | Permission plus indicator, scoped to a window or a monitor. |
| Clipboard | Background clipboard sniffing | Read only on the user's paste or by the focused app; clipboard managers need a permission. |
| Input injection | Fake input | Permission plus indicator (remote desktop, voice control). |
| Global hotkeys | Apps keylogging to catch shortcuts | Hotkeys **registered with the server by name and approved by the user**. Push-to-talk keys for radio and voice tools work properly. |
| Remote display | Network eavesdropping and hijack | Always authenticated and encrypted (SSH or WireGuard); never unauthenticated TCP. |
| Phase 1 daemon (X11) | Creating a new hole | No network socket, no unauthenticated control interface; the published property holds only monitor scales; saved layouts stored with owner-only permissions (0600). |

### 7.4 An honest limit

On a typical desktop, any program running as the user can already read the user's files and attach a debugger to their other programs. **A display server cannot create isolation that the operating system doesn't provide.** Its job is to **not be the weak link**. Real application-to-application isolation needs sandboxing (Flatpak on Linux, jails and Capsicum on FreeBSD), which also gives the server a **trustworthy application identity** to attach permissions to. Without a sandbox, identity is only the executable path, which is weaker. The design supports both and is explicit about the difference.

---

## 8. Implementation strategy

**No new drivers.** The server builds on the shared lower layers that **both Linux and FreeBSD** provide: DRM/KMS for display, libinput for input, Vulkan/OpenGL for rendering.

**Language: Rust throughout.** Everything the project itself ships is written in Rust:

- **Phase 1 daemon:** Rust over XCB/RandR (the `x11rb` crate). Its profile, EDID and rules logic lives in a shared crate that moves unchanged into the server.
- **Server core:** Rust, on **Smithay**, the compositor library behind niri and System76's COSMIC. A compositor needs steady frame timing, which rules out garbage-collected runtimes.
- **Settings, profile editor and tray indicator:** Rust with **Slint**, a light declarative UI toolkit that runs on Linux and FreeBSD (iced, used by COSMIC, is the alternative).
- **Display-list reference implementation:** Rust, rendering with **Vello**, a Rust GPU vector renderer (§6.6).

Why one language:

1. **FreeBSD from day one.** Rust and Slint build natively on FreeBSD today. .NET works on FreeBSD (AlbumWall, Deadwax and Marginal all run there as Native AOT builds made with the FreeBSD port's own ILCompiler), but only through the FreeBSD ports' .NET 9, a version behind what our apps target and outside Microsoft's official platforms. System components of the desktop itself should not depend on that.
2. **System components must be small and start instantly.** Settings panels and tray indicators are part of the desktop, not applications, and are packaged with it.
3. **One codebase.** Profile and EDID logic, the configuration format and the protocol types are shared between daemon, server and tools, with no second toolchain or cross-language bindings.

**Compatibility at the edges.**

- **Wayland applications** connect through a standard Wayland front end.
- **X11 applications** run through rootless Xwayland. Because the server owns input routing, legacy windows can be **scaled per monitor with correct input mapping**, which removes the X11 wall from §5 without any X extension.
- The server speaks Wayland and X11 at the edges, but placement, memory, units, accessibility and security follow this design.

**Platforms.** Debian (13, X11, XFCE as the stable session; Xephyr for nested development) is the primary development platform. FreeBSD is tested alongside at every phase.

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

---

## 9. Roadmap

| Phase | Deliverable | Value on its own | Estimated effort |
|---|---|---|---|
| **1** | X11 consistency daemon: EDID profiles, scaling modes A/B, toolkit settings, layout memory | Fixes daily annoyances on any X11 desktop, Debian and FreeBSD | Days to weeks |
| **2** | Display server, first nested in a window, then on real hardware: native per-monitor rendering, placement memory, reference-unit scaling, per-window scaling of X11 apps with correct input | A usable personal desktop that beats X11 and Wayland on consistency | Months |
| **3** | Accessibility contract and display-list surfaces: Rust/Vello reference implementation, then the native Avalonia backend (§8.1) | Sharp magnification, sharp straddling windows, accessibility by design | Research |
| **4** | Remote display over display lists | Efficient, sharp remote desktops | Research |

**Where the real costs are.** Writing the code is not the bottleneck. The bottlenecks are:

- **real-hardware testing:** GPUs, monitors with unusual EDIDs, docks, sleep and wake. Most of it can be automated (§4.8), which turns weeks of manual testing into overnight runs; what remains manual is real docks, KVM switches and FreeBSD hardware;
- **the user's time and judgment:** what "the right size" means to their eyes;
- for phases 3–4, **toolkit adoption**, which only a working demonstration can start.

---

## 10. Open questions and items to verify

1. Exact live-update behaviour of Qt 6 and Chromium scaling on X11.
2. Avalonia 12's X11 per-screen scaling variables and behaviour.
3. Reliability of EDID physical-size data across real monitors (some report zero or wrong sizes), and the fallback when it is missing.
4. How to capture viewing distance simply (presets: laptop, desktop, TV; or a one-time calibration).
5. Status of Wayland session-restore and placement protocols, and whether the server should implement them for compatibility.
6. Display-list format: the minimal validated command set (paths, fills, strokes, gradients, images, text runs with font references, clips, layers), and how text is shipped (glyph runs plus font identity, so the server can re-render text sharply at any density).
7. FreeBSD specifics: DRM/KMS driver coverage for the target GPUs; libinput and seat management.
8. Slint versus iced for the tools, decided by a small prototype of the profile editor on both Debian and FreeBSD.
9. The scope of the native Avalonia backend: which parts of Avalonia's platform interfaces it must implement, and how its automation peers map to Contract 3.
10. Firefox and Thunderbird: confirm that Firefox on X11 has no per-monitor scaling; check whether Mozilla implements (or plans) the Wayland session-restore protocol; how well heuristics match several Firefox windows to saved layouts; the scope of an AT-SPI bridge into Contract 3.

---

## Appendix A: Glossary

- **EDID:** data a monitor reports about itself: manufacturer, model, serial, physical size, supported modes, colour characteristics.
- **RandR:** the X11 extension for querying and configuring outputs (monitors).
- **EWMH:** Extended Window Manager Hints; the standard by which clients and tools talk to X11 window managers.
- **XSETTINGS:** an X11 protocol for publishing desktop settings (fonts, scale) that toolkits read live.
- **CSS reference pixel:** the visual angle of one pixel at 96 dpi seen from about arm's length (roughly 0.0213°).
- **Compositor:** the component that combines window images into the final screen image.
- **Display list:** a recorded sequence of drawing commands that can be replayed at any resolution.
- **KMS/DRM:** the kernel interfaces for display mode-setting and GPU access, on Linux and FreeBSD.
- **Smithay:** a Rust library for building Wayland compositors.
- **Slint:** a lightweight declarative UI toolkit for Rust, running on Linux and FreeBSD.
- **Vello:** a Rust GPU renderer for 2D vector graphics.
- **x11rb:** Rust bindings for the X11 protocol (XCB).
- **Capsicum:** FreeBSD's capability-based sandboxing framework.

## Appendix B: One sentence

*NeXT's single authority and device independence, Windows' placement memory, Wayland's per-monitor rendering, and the web's reference pixel, with accessibility and security part of the protocol rather than add-ons.*
