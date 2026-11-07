# Milestone 4: the right size on every monitor

Whitepaper §4.2, §4.3, §4.6. Milestones 2 and 3 put monitors and windows
back. Milestone 4 makes things **the right size**: a window, its text and
the pointer are the same physical size on every monitor, and every toolkit
gets the same answer.

**Status (2026-11-06): plan, all decided (Dave, 22:09-22:11): 1 (a) Mode A
first; 2 changed to "vergd is the XSETTINGS manager" (below); 3 (a)
derived scale; 4 (a) off until `vergd scale on`; 5 (a) session file read at
login; 6 (a) test apps for all six toolkits.**

**Separate from the desktop (Dave, 2026-11-06 22:09).** Xfce stays the
desktop on Hubble until the display server replaces it, but it is only a
host: nothing new is built on Xfce's internals (xfconf, xfsettingsd), and
everything must work under any EWMH window manager. Milestones 2 and 3
already do (RandR and EWMH only). Pieces that the phase 2 server needs
anyway are written so they carry over; the XSETTINGS manager is the first:
the server must provide XSETTINGS to X11 apps under Xwayland. Most of it can
be built and tested with fake monitors of different densities (a fake EDID
can claim any physical size). Only "does it look right" needs real monitors:
the one-time size calibration and the final check (acceptance 8).

## What Hubble has (checked 2026-11-06)

- **ASUS VS229:** 1920x1080, 475 x 267 mm, so about 102 ppi. The fake
  (`VERGE FAKE 1`) claims 48 x 27 cm, about the same density. Two more fakes
  are needed: a 27" 4K (60 x 34 cm, about 163 ppi; 3840x2160 at 30 Hz, the
  most HDMI 1.4 carries) and a 24" 1080p (53 x 30 cm, about 92 ppi). Together
  these are the whitepaper's worked example (§4.6).
- **Screen limit:** X reports 16384 x 16384 maximum.
- **XSETTINGS is owned by Xfce** (`xfsettingsd`, from the xfconf `xsettings`
  channel: `Gdk/WindowScalingFactor` 1, `Xft/DPI` 96, cursor size 0).
  `Xft.dpi` in the X resources is 96. Two XSETTINGS managers would fight,
  so vergd taking the role (decision 2) means `xfsettingsd` gives it up.
- **Xfce has its own per-monitor Scale** (Display settings, xfconf
  `displays` channel, `/Default/HDMI-2/Scale` 1.0), using the same RandR
  transform as Mode A. vergd and Xfce must not both set it; vergd acts last
  (as in milestone 2), and the plan says to leave Xfce's at 1.
- **Toolkits installed:** GTK 3 and 4 (with `gtk3-demo`, `gtk4-demo`,
  widget factories), Qt 6 (`qt6-base`; no Qt app yet), Xlib (`xterm`,
  `xclock`). **Available as packages:** Qt apps (`featherpad`, `qterminal`,
  `kcalc`), Java (`openjdk21`), Electron (`electron42`), .NET 9 (`dotnet`)
  for a small Avalonia test app (decision 6).

## The scale of a monitor (derived, not guessed)

From the whitepaper (§3 principle 1, §6.3): the unit is the **CSS
reference pixel**, the size one pixel at 96 dpi appears from 71 cm (28 in).

    scale = (ppi / 96) x (viewing distance / 71 cm) x preference

- **ppi** from the EDID's physical size (milestone 1 parses it; the
  whitepaper's §10 item 3 covers missing or wrong sizes: then 96 ppi, scale
  1, and vergd says so).
- **Viewing distance** from a preset per monitor: desktop 71 cm (default),
  laptop 50 cm, TV 250 cm; editable per monitor in the profile.
- **Preference:** one number for "how big I like things" (1.0 = the web's
  reference size), set once by calibration (acceptance 8).
- **Rounded to steps of 0.25.** Examples at the desktop distance: ASUS 102
  ppi -> 1.06 -> **1.0**; 24" 1080p 92 ppi -> 0.96 -> **1.0**; 27" 4K 163 ppi
  -> 1.70 -> **1.75**.

A profile can override the derived value per monitor.

## Mode A, "Uniform" (macOS approach)

Render the whole desktop at a whole-number scale **R** (GTK only takes whole
numbers): R = the largest monitor scale, rounded up. Each monitor is
rendered at R and RandR shrinks it onto the panel by R / its scale:

| ASUS + 27" 4K | ASUS (1.0) | 4K (1.75) |
|---|---|---|
| R | 2 | 2 |
| RandR scale (rendered / native) | 2.0 | 1.143 |
| Rendered size | 3840x2160 | 4389x2469 |

- Every toolkit gets R: GTK window scale R, `Xft/DPI` and `Xft.dpi` 96 x R,
  cursor size 24 x R, Java/Electron the same at launch. Everything is the
  same physical size everywhere, in every toolkit.
- **Costs:** more GPU work, and monitors scaled by a non-whole factor are a
  little soft (here the ASUS, drawn at 2 and halved). With only monitors of
  scale 1 (Hubble today), R = 1 and **nothing changes**.
- **Limits:** the rendered screen must fit in 16384 x 16384; vergd refuses a
  setup that doesn't and says why (whitepaper pass criterion 3).
- **Edge alignment:** monitors' logical heights now differ; positions are
  stored in the profile as in milestone 2, and vergd's default for a new
  arrangement is top-aligned.
- **Window memory** (milestone 3) works in logical pixels, so it is
  unchanged as long as a setup keeps its scale; a scale change for a setup
  rescales the remembered places by the same factor.

## Mode B, "Native" (sharpest)

No RandR transform; each monitor at native resolution. Per-monitor factors
go to toolkits that can use them (Qt: `QT_SCREEN_SCALE_FACTORS`; Avalonia:
`AVALONIA_SCREEN_SCALE_FACTORS`); GTK, Electron, Java and Xlib get one
global value, so they are the wrong size on some monitor. Useful when every
monitor has nearly the same scale, or for Qt/Avalonia-heavy use.

## How the settings reach applications

| What | Read by | When it takes effect |
|---|---|---|
| XSETTINGS `Gdk/WindowScalingFactor`, `Xft/DPI`, `Gtk/CursorThemeSize` | GTK 3/4 (Firefox, Thunderbird, Xfce) | **live** |
| `Xft.dpi`, `Xcursor.size` (X resources) | Qt, Electron, Xlib fonts | new apps |
| `GDK_SCALE`, `QT_SCREEN_SCALE_FACTORS`, `AVALONIA_SCREEN_SCALE_FACTORS`, Java's `-Dsun.java2d.uiScale` | Java, Qt, Avalonia, some Electron | **next login** (decision 5) |
| `_VERGENCE_SCALES` root-window property (documented) | apps that opt in (our .NET helper, §8.1) | live |

## Built so far

- **Scale and Mode A numbers** (`vergence-profile::scaling`), shown by
  `vergd probe`. Checked with fakes: 27" 4K 1.75 (163 ppi), ASUS 1 (103),
  24" 1 (92).
- **Fake monitors of any density:** `vergence-harness fake`. A 4K fake needs
  an HDMI vendor block, or Intel's driver caps the port at 165 MHz; with it,
  3840x2160 at 30 Hz lights on HDMI.
- **XSETTINGS manager** (22:20): `vergence-xsettings` (wire format) and
  vergd's manager. On start vergd imports the running manager's settings
  once into `settings.toml` (0600), takes the selection with a real
  timestamp, publishes, and announces itself; it checks every 2 s that it
  still owns the selection (its waits swallow events) and re-reads the file
  when it changes. `vergd watch --no-xsettings` turns it off;
  `vergd xsettings` shows what is published. On Hubble: imported Xfce's 30
  settings, took over from xfsettingsd (which keeps running), and an edit of
  `Gtk/FontName` (Sans 10 to 14) was published within 2 s and Dave saw
  every GTK app switch live, without a restart; put back to 10.

- **Mode A** (22:30): `vergd scale on|off|status`. Profiles store each
  monitor's RandR factor; `repack` lays monitors out again in a row or
  column (mirrored ones are refused); the running vergd applies the
  settings within 2 s of `scale on/off`, on every setup change and at
  login. Toolkits: XSETTINGS overlay (`Gdk/WindowScalingFactor`,
  `Gdk/UnscaledDPI`, `Xft/DPI`, cursor size), `Xft.dpi` and `Xcursor.size`
  in the X resources, `session.env` (Java `-Dsun.java2d.uiScale`; at first
  also `AVALONIA_GLOBAL_SCALE_FACTOR`, dropped after the measurement below)
  sourced from `~/.xprofile`. GDK_SCALE is
  deliberately not set: it would pin GTK to one scale and stop it following
  XSETTINGS live. **Live on Hubble, ASUS + fake 27" 4K:** render 2; ASUS
  drawn 3840x2160 and shrunk x2.000, 4K drawn 4389x2469 at +3840 and shrunk
  x1.143; screen 8229x2469; toolkits 2. Unplug: ASUS alone back to 1.
  Replug: Mode A back from the profile. `scale off`: 1:1 again.
- **Still to do:** Java on FreeBSD (check on Debian); Mode B (7); real
  monitors and the calibration (8); Dave trying the Display dialog by hand
  in Mode A once real monitors are here.
- **Window memory across scale changes (23:05):** places are now kept in
  panel pixels (X pixels / the monitor's RandR factor) and sizes in app
  units (X pixels / the render scale); when the scale of the current setup
  changes, vergd waits for the apps to resize and puts every window back.
  Memory recorded unscaled means the same as before. **Checked:** three
  xfce4-terminals on the ASUS and the 4K (one maximised), three cycles of
  `scale on` / `scale off`: scaled, each stays at the same place on its
  panel (305,229 and 585,329) at twice its X size; unscaled, every window
  is back on the same pixel; no drift between cycles. Known wrinkle: a
  maximised window's remembered size alternates between its maximised size
  at each scale (harmless: it is maximised again either way).
- **Three monitors and the limit (23:10-23:12), acceptance 4 and 5,**
  with scaling on: ASUS alone 1:1 (scale 1); + 27" 4K: 8105x2540, scale 2;
  + 24": 12121x2540, scale 2 (24" shrunk x2.09). Each setup came back with
  its own scale through every unplug and replug. **Pass.** Swapping the 27"
  for a fake **13.3" 4K** (332 ppi, needs 4x): refused, "screen 19651x4518
  exceeds this server's maximum 16384x16384; this setup stays at 1:1",
  toolkits back to 1. **Pass.** Fixed on the way:
  - `xrandr --right-of` places a monitor next to its neighbour's
    **unscaled** size, so scaled sizes overlapped and vergd refused a row
    the user had made; rows and columns are now detected from unscaled
    sizes.
  - "Already as saved" now also needs the right screen size: after an
    unplug X had left the screen at 8897x3774 for two monitors.
  - A refusal (mirrored, too large) falls back to 1:1 for monitors and
    toolkits alike; before, the toolkits could stay scaled on an unscaled
    desktop.
  - X keeps a CRTC's transform when another monitor is plugged into the
    port; vergd sets every factor itself, so it is cleared.
- **Xfce reacting (acceptance 6, first look):** right after vergd scaled a
  new setup, Xfce changed it back; vergd logged "the desktop reacting,
  putting the profile back", applied once, and it stayed.
- **Xfce's Display dialog (acceptance 6, 23:18-23:21),** driven the way the
  dialog does it (layout written to the `displays` channel, then
  `/Schemes/Apply`), in Mode A with ASUS + 4K:
  - *4K moved to the left of the ASUS:* xfsettingsd left the 4K's transform
    at x=0 and put the unscaled ASUS at x=250, overlapping. vergd first
    refused to scale an overlap, and could not fall back to 1:1 either, so
    the desktop stayed half scaled with GTK at 2. **Fixed:** partly
    overlapping monitors are laid out in the order of their left (or top)
    edges; only identical positions count as mirrored. Now: one apply, 4K
    left and ASUS right, both scaled, nothing loops. **Pass.**
  - *Xfce's own Scale 1.5 on the ASUS:* learned, then Mode A put back over
    it (vergd's scale wins, as planned; leave Xfce's at 1). No loop.
  - *Appearance DPI 120:* xfsettingsd no longer touches `Xft.dpi` or
    XSETTINGS; vergd's 192 stayed. The Appearance dialog has no effect, as
    decided.
- **Login checked (22:36):** LightDM's Xsession on FreeBSD sources
  `~/.xprofile` (`VERGENCE_SCALE=1` from `session.env` was in the new
  session's environment); at login vergd took XSETTINGS over from
  xfsettingsd, from its own file. Logging out restarts X with LightDM.

- **Measured (22:40-22:50), acceptance 3,** with `tools/sizebox` (a
  200 x 200 box in each toolkit's own units; 52.9 mm at the reference size)
  and `vergence-harness measure`, ASUS (103 ppi) + fake 27" 4K (163 ppi):

  | Toolkit | off: ASUS | off: 4K | Mode A: ASUS | Mode A: 4K |
  |---|---|---|---|---|
  | GTK 3, GTK 4, Qt 6, Electron 42 | 49.5 mm | 31.1 mm | 49.5 mm | 54.4 mm |
  | Xlib | 49.5 | 31.1 | 24.7 | 27.2 |
  | Java 21 | 49.5 | 31.1 | 24.7 | 27.2 |

  - Without scaling the box is 37% smaller on the 4K. In Mode A the four
    scaled toolkits are identical to each other on each monitor.
  - **The 10% left between the monitors is the rounding to 0.25 steps**
    (ASUS exactly 1.07 -> 1, 4K exactly 1.70 -> 1.75). With exact scales
    both would be 52.9 mm (decision 7 below).
  - **Xlib** draws in device pixels: consistently small, as expected.
  - **Java 21 on FreeBSD ignores every HiDPI switch** (`-Dsun.java2d.uiScale`,
    `sun.java2d.uiScale.enabled`, `GDK_SCALE`, `J2D_UISCALE`): no scaling at
    all, even when passed directly. Probably Linux-only code in the port; to
    check on Debian.
  - **Electron 42 on X11 scales its content but not its window**: at scale 2
    a 240 px window cut its content off. It reads the scale from XSETTINGS,
    not `Xft.dpi`. Real Electron apps would open too small at scale 2.
  - **Avalonia 12.1.5** (23:00, `tools/sizebox/avalonia`, .NET 9 with
    BsdProbe's FreeBSD workarounds: our libSkiaSharp 119, the libc-epoll
    glue, a HarfBuzz symlink): at first **drawn at 4x** (105.9 mm), because
    Avalonia reads `Xft.dpi` itself (2) and multiplied it by vergd's
    `AVALONIA_GLOBAL_SCALE_FACTOR=2`. Without that variable: **52.9 mm on
    both monitors**. vergd no longer sets it; Avalonia gets its scale like
    Qt, from `Xft.dpi` when it starts.
  - **With exact scales (decision 7, 22:53): GTK 3, GTK 4, Qt 6 and
    Electron are 52.9 x 52.9 mm on both monitors**, the reference size
    exactly. ASUS drawn 400 px, shrunk x1.87 to 214 panel px; 4K drawn 400
    px, shrunk x1.176 to 340 panel px; screen 8105x2540. **Acceptance 3
    passes for these four toolkits, and Avalonia (five of the six; Java
    open).**

## Decisions for Dave

1. **Scope.** (a) Mode A and the toolkit settings first, then Mode B as the
   second half of this milestone. *(recommended: Mode A is the default and
   the only one that is right for every toolkit)* (b) Both at once.
2. **Where the settings go.** ~~(a) Through Xfce's own settings.~~
   **Decided (Dave, 22:09): vergd is the XSETTINGS manager itself**, on any
   window manager (see "Separate from the desktop" above). On Hubble,
   Xfce's `xfsettingsd` holds the XSETTINGS selection today; it also does
   keyboard, display and other settings, so the first step is to find out
   exactly what is lost when its XSETTINGS part is off (or when vergd takes
   the selection over), and tell Dave before changing his session. Clean
   tests run in a nested X server (Xephyr) with a bare window manager,
   without touching his session.
   **Checked on Hubble (22:15):** XSETTINGS carries 30 settings, not just
   the scale: GTK theme, icon theme, fonts, cursor theme, double-click
   time... so vergd publishes the **full set**: imported once from the
   current owner's `_XSETTINGS_SETTINGS` property (X protocol, no xfconf),
   kept in `~/.config/vergence/settings.toml`, with the scale values
   computed on top. When another program takes the selection,
   `xfsettingsd` 4.20.5 **keeps running** (keyboard layout, pointer and
   display parts unaffected) and does **not** take it back, even on an Xfce
   settings change. Consequences: Xfce's Appearance dialog no longer has an
   effect while vergd holds XSETTINGS (accepted: Hubble is a project
   machine, Dave 22:13); while nobody holds it, new GTK apps get plain
   defaults, so vergd takes it at start and keeps it.
3. **Where the scale comes from.** (a) Derived from the EDID size, viewing
   distance and one preference, as above, with a per-monitor override.
   *(recommended: whitepaper principle 1)* (b) A percentage per monitor per
   profile, set by hand, as Windows does.
4. **When it is on.** (a) Off until `vergd scale on`; then every setup,
   new ones included, gets the derived scale, and `vergd scale off` puts
   everything back to 1:1. *(recommended: Mode A changes the whole
   desktop's rendering, so it starts as an explicit choice)* (b) On by
   default.
5. **Launch-time variables** (`GDK_SCALE`, `QT_SCREEN_SCALE_FACTORS`...).
   (a) vergd writes them to `~/.config/vergence/session.env`, sourced at
   login (one line in `~/.xprofile`, which LightDM reads; to be confirmed
   on FreeBSD), so they apply from the next login; the live parts go
   through XSETTINGS and X resources. *(recommended)* (b) Leave them out;
   live settings only.
6. **Test applications on Hubble.** (a) Install `featherpad` (Qt),
   `openjdk21` (Java), `electron42` (Electron) and `dotnet` (.NET 9, for a
   small Avalonia test app), so all six toolkit rows of whitepaper §4.5 are
   tested. *(recommended; Electron is a large package)* (b) GTK, Qt and Xlib
   only.

7. **Rounding in Mode A** (added 22:52 after the first measurement).
   **Decided (Dave): exact.** Mode A uses each monitor's exact scale, so a
   window is the same physical size on every monitor (both 52.9 mm instead
   of 49.5 / 54.4); no monitor gets a whole-number factor, so all are a
   little soft; a per-monitor override (`scale = 100`) gives one back a
   crisp factor. Rounded scales still decide whether a setup needs scaling
   at all: with no monitor above 1 (e.g. the ASUS alone, exactly 1.07)
   everything stays 1:1 rather than drawing the desktop at 2x for a 7%
   correction.

## Acceptance (Hubble, fake monitors of different densities unless noted)

1. `vergd probe` prints each monitor's ppi and derived scale; missing or
   impossible sizes fall back to 1 and say so (with database EDIDs from the
   milestone 1 harness).
2. **Mode A:** ASUS + fake 27" 4K: R = 2, RandR scales 2.0 and 1.143,
   toolkit settings at 2; `vergd scale off` puts it all back to 1:1.
3. **Same physical size, measured** (whitepaper §4.8.3): a box of known
   logical size drawn by a GTK 3, GTK 4, Qt, Xlib, Java, Electron and
   Avalonia test program on each monitor is screenshotted; its size in mm
   (from the transform and the EDID size) differs by under 5% across
   monitors and toolkits.
4. **Per setup:** ASUS alone, ASUS + 4K, ASUS + 4K + 24": each keeps its
   own scale; unplug/replug puts the right scale and transform back, and
   milestone 3's window memory still restores every window (no drift).
5. **Limits:** a setup whose rendered screen would exceed 16384 px (the ASUS
   and three fakes) is refused, with the reason in the log.
6. **Xfce:** Xfce's Display settings or `xfsettingsd` changing the scale or
   DPI back is noticed and put back (vergd acts last), and nothing loops.
7. **Mode B:** Qt and Avalonia get per-monitor factors and are the same
   size on each monitor; GTK gets one value (documented as expected).
8. **Real monitors (cables):** Dave calibrates the preference once ("this
   box looks the size of a credit card"); then 3 and 4 again, and his
   judgment of the result.

## Real monitors for acceptance 8 (Dave, 2026-11-06 23:21)

When the cables arrive: the **ASUS VS229** (22" 1080p, in use), the **14"
4K** used for the Pi 5 PoE kiosk tests, and the **32" 4K** that was
Techbench's monitor before the LG 6K. Expected, with typical panel sizes
(310 x 174 mm, 708 x 398 mm; the EDIDs will say):

| | ppi | exact scale (desk) |
|---|---|---|
| ASUS 22" 1080p | 103 | 1.07 |
| 32" 4K | ~138 | ~1.43 |
| 14" 4K | ~315 | ~3.28 (~2.3 at laptop distance) |

- ASUS + 32": render 2, about 8,960 px wide. The everyday case.
- ASUS + 14": render 4, about 11,860 px (the ASUS drawn at nearly 4x).
- 14" + 32": render 4, about 15,420 px, just under the limit.
- **All three: render 4, about 22,600 px: refused** (18,400 px even with the
  14" at laptop distance). The real-hardware case for the desktop-size wall
  (whitepaper §5) and for Mode B, which has no such limit.
- Hubble's HDMI port does 4K at 30 Hz only; the 4K monitors belong on the
  DisplayPort outputs. Cabling (Dave, 23:24): ASUS on HDMI (it also has VGA
  and DVI); 32" 4K by DisplayPort (it has DP and HDMI); 14" 4K (mini HDMI
  only) by a DP to mini-HDMI cable rated for 8K. If that cable is active it
  shows as a DP output at 4K 60 Hz; if passive (dual-mode DP), as HDMI-1 or
  HDMI-3 at 4K 30 Hz. `vergd probe` tells which.

## Risks

- Softness in Mode A on monitors scaled by non-whole factors; the reason
  Mode B exists.
- The UHD 630's GPU cost of large rendered screens; measured in 2 and 4.
- Xfce panels and the desktop at scale 2: Xfce 4.20 supports it; to watch.
- HDMI fakes at 4K are 30 Hz only; DP connectors would do 60 Hz but a
  forced DP connector with no monitor may fail link training (not needed
  for the tests).
