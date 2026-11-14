# GPUs milestone 1: screens on a second GPU (Hubble's HDMI)

Drafted 2026-11-14, when Dave plugged the 14" 4K touch monitor into
Hubble's HDMI port and nothing showed (Dave: "yes, draft the multi-GPU
milestone plan"). The whitepaper's §8.3 has the design this milestone
starts: one renderer per GPU, capabilities probed and never assumed by
vendor, explicit sync later. **Accepted 2026-11-14: "yes to your
proposals, go ahead with step 1"** (our own code for the second GPU; the
Intel GPU draws external screens by default; nouveau now, NVIDIA's driver
and explicit sync as the next GPU milestone).
**Closed 2026-11-14** (Dave: "go ahead with the docs, carry the fallback
over"). Carried over: way 2, the copy by the CPU for two GPUs that share no
buffer format, with its test on a fake second GPU (step 4); the second GPU's
probe on waking moved off the server's thread (a 0.45 s hitch now).

## Where things stand (read 2026-11-14 on Hubble)

- **Hubble** is Mobilenode, an ASUS TUF Gaming F15 **FX506LH** (2020),
  Void, kernel 6.18.63. Two GPUs:

  | Card | GPU | Driver | Connectors | Now |
  |---|---|---|---|---|
  | card0 | Intel UHD (0x9bc4) | i915 | eDP-1 (the laptop's panel) | draws everything, active |
  | card1 | NVIDIA GTX 1650 Mobile, TU117 (0x1f99) | nouveau, GSP firmware RM 570.144 | HDMI-A-1, DP-1 (the USB-C port) | runtime-suspended |

  **No MUX switch** (asus-wmi offers no `gpu_mux_mode` on this model): the
  panel is wired to the Intel GPU only, and the HDMI and USB-C DisplayPort
  outputs to the NVIDIA GPU only. Neither GPU can drive the other's
  connectors; a GPU can only draw for one and hand the frames across.
- **The server** (crates/vergence-server/src/drm.rs, cut down from
  Smithay's anvil) opens one GPU, the seat's primary (`primary_gpu`, else
  the first), with one GBM device, one EGL context, one GlesRenderer and one
  DrmOutputManager; its connector scans, hotplug and surfaces are all that
  GPU's. card1's HDMI-A-1 shows `connected` in /sys and the server never
  looks at it.
- **The Battery page** (settings milestone 1) already shows each GPU awake
  or asleep, so the cost of waking the NVIDIA card is visible: Hubble idles
  at 9.4 W on battery with it asleep.
- **Elsewhere**: kepler (Intel UHD 630) and Techbench (Intel UHD 770) have
  one GPU each. FreeBSD has no nouveau (§8.3), so this is tested on Linux;
  the code stays for both (a FreeBSD machine with Intel and AMD would use it
  unchanged).

## Design

### The GPUs, probed

At start every GPU on the seat is opened (through seatd/libseat, as the
first is now), and for each the server logs and keeps: its driver, its
connectors, the formats and modifiers it can render to, scan out and
import, whether it can render at all (a display-only GPU: some can only
scan out), and its PCI slot. Nothing is decided by vendor name
(whitepaper §8.3, decision 3). `vctl gpus` prints it.

### Who draws a screen on the second GPU

Each screen is drawn by one GPU and shown by the GPU its connector belongs
to. Three ways, chosen from what the two GPUs said they can do:

1. **Drawn on the main GPU, shown by the other (the default).** The Intel
   GPU renders the HDMI screen's frame into a buffer the NVIDIA GPU can
   scan out (a dmabuf both understand, usually linear), and the NVIDIA GPU
   only displays it. The NVIDIA card wakes for the display engine alone;
   every window's buffer stays where it is. The battery-friendly way, and
   what Linux desktops call "reverse PRIME".
2. **Copied** (the fallback when no buffer format is shared): the frame is
   rendered on the Intel GPU offscreen, read back and written into a buffer
   on the NVIDIA GPU. It always works; at 4K 60 Hz it moves about 2 GB/s,
   so it is the last resort, logged as such.
3. **Drawn on the screen's own GPU** (later, chosen per screen): the
   NVIDIA GPU renders the HDMI screen itself, every window's buffer
   imported or copied once per frame (§8.3, decision 2). For the 43" 4K
   144 Hz TV where the Intel GPU may not keep up; costs the battery.

### Hotplug and power

- The second GPU's connectors are scanned and hotplugged like the first's
  (udev on Linux, devd on FreeBSD, the 2-second rescan already there).
- A GPU with no screen and nothing to draw is left free to sleep: the
  server keeps its device open but sets no mode on it, so the kernel's
  runtime power management can suspend it (to be measured: whether nouveau
  sleeps with the server holding the device). Plugging a screen in wakes
  it; unplugging lets it sleep again, seen on the Battery page.

### What follows the screen

Everything per screen works the same on either GPU: scale from the EDID,
window memory, the arrangement, wallpaper, the dock, `vctl shot`, and the
touch panel's tie (the 14" monitor's touchscreen, by USB, on the HDMI
screen: the Touchscreen page).

### What is not in this milestone

The NVIDIA proprietary driver (Hubble runs nouveau), explicit sync (needed
for NVIDIA's own driver; §8.3, decision 1), rendering client windows on the
NVIDIA GPU and telling clients which GPU sits under them, GPU reset
recovery across two GPUs, more than two GPUs, multi-card walls.

## Progress

- **Step 1 in: the probe** (2026-11-14). The server's gpus.rs opens every
  GPU on the seat at start (through libseat, no mode set, no master),
  reads its driver, connectors (and which have a screen), the formats its
  planes scan out, and through EGL on its own GBM device the formats it
  renders to and imports, with whether linear XRGB/ARGB8888 is among them;
  logs it, keeps it for `vctl gpus`, and closes it again. On Hubble: card0
  i915 [0000:00:02.0], eDP-1 with a screen, renders 106 formats and imports
  199; card1 nouveau [0000:01:00.0], **HDMI-A-1 with a screen**, DP-1,
  renders 80 and imports 114; **both render and import linear XRGB**, so
  way 1 (drawn on Intel, shown by NVIDIA, a shared linear buffer) needs no
  copy; the NVIDIA card suspends again after the probe. kepler: one i915,
  five connectors, no screen (its 14" monitor is on Hubble now). 474 tests
  (kepler 470).
- **Steps 2 and 3, first light** (2026-11-14, Dave: "step 3 then"). The
  backend holds a `Device` per GPU with screens (its output manager and
  connector scanner; screens keyed by GPU and CRTC, CRTC handles being per
  GPU; a VBlank source per GPU; pause, resume, scans, hotplug and the
  watchdog for all). **Way 1 as planned failed**: the Intel GPU cannot draw
  into a buffer nouveau allocates for scan-out (EGL BAD_ALLOC importing it:
  nouveau scans out of its video memory only). What works, as Xorg does it:
  the Intel GPU draws the screen into one of two linear XRGB buffers in its
  own memory (damage by buffer age), waits for it, and the NVIDIA GPU, with
  a GlesRenderer of its own, takes the buffer in as a texture and copies it
  into its scan-out buffer, through a hidden scale-1 output so the copy is
  pixel for pixel. On Hubble: the 14" 4K on HDMI-A-1 at 300%, "shown by
  /dev/dri/card1, drawn by /dev/dri/card0 and copied there", window memory
  put the settings window back on it; the NVIDIA card awake while it is
  plugged in. Frames take 20 to 40 ms (the server waits for the Intel GPU
  before the copy): to make fast next. 474 tests (kepler 470); kepler's
  one-GPU session unchanged.
- **Frames made fast** (Dave: "go ahead and make the frames faster").
  Timed: drawing under 1 ms, the copy about 1 ms, the server **waiting**
  for the Intel GPU 10 to 26 ms. A GPU-side wait (Smithay's
  `Renderer::wait`, the Intel fence imported into nouveau's EGL) fell back
  to blocking: these drivers share no fences that way. Now pipelined: three
  buffers; each turn the Intel GPU draws any change into the next free one
  without waiting, and the NVIDIA GPU copies the newest buffer whose fence
  has already signaled (`SyncPoint::is_reached`, never waited on); not yet
  signaled, the server looks again in 3 ms instead of a frame. At most a
  frame of extra latency on that screen; the server never blocks. Hubble:
  100 pointer moves on each screen, **0 slow frames** (before, every frame
  on the 14" took 20 to 40 ms). Slow-frame warnings name the screen;
  `VERGENCE_RELAY_TIMES=1` logs every relay turn.
- **Unplugged** (Dave): HDMI-A-1 removed, the windows back on the panel, the
  NVIDIA card **suspended**, the draw on battery 13.3 W falling to 9.9 W in
  30 s (acceptance 4). Dave saw **glitches** on the 4K screen: the pipeline
  drew newer frames every 3 ms while one waited, and could cycle into the
  buffer the NVIDIA GPU was still copying. Now no frame is drawn while one
  waits to be copied, and the buffer copied last is never drawn into until
  another has been copied. To be seen on the screen again.
  Still glitching (Dave: "when the mouse moves", then "without me moving the
  mouse"). `vctl shot` of a screen on another GPU now reads back the very
  buffer the screen got: clean and steady, so the fault was on the NVIDIA
  side. Timed for real (the earlier 1 ms was only the submission): **a full
  4K copy takes nouveau 40 to 77 ms**, and with no fence from nouveau's EGL
  the screen flipped to half-copied frames. And every frame was drawn and
  copied whole (8.3 million pixels): the buffer ages ran past the few frames
  Smithay's tracker keeps. Now one buffer, age always 1 (it always holds the
  newest frame), the Intel GPU's damage passed to the NVIDIA side
  (`TextureRenderBuffer`, so its compositor copies only those rectangles),
  and the copy finished (`glFinish`) before the flip and before the Intel
  GPU draws again. Hubble: 0 slow frames idle for 10 s; 2 in 60 pointer
  moves on the 14" (the full copies after start); a pointer move copies a
  few thousand pixels. Full-screen changes still cost a 40 ms copy: why
  nouveau's copy from system memory is that slow is open.
- **The likely why** (glmark2 on Hubble, 2026-11-14): offscreen the Intel GPU
  scores 4383 (thousands of frames a second; in a window it held 46 FPS, the
  session's pacing, to look at); with `DRI_PRIME=1` Mesa 26 tried nouveau's
  GL through Zink, which needs NVK and the Vulkan loader, not installed, and
  fell back to **llvmpipe**. The server's copier on the NVIDIA card is very
  likely the same software renderer, so the "GPU copy" was the CPU's. The
  server now logs each GPU's GL renderer at start and warns when a showing
  GPU's is software. Next on Hubble: install `mesa-vulkan-nouveau` and
  `vulkan-loader` (Void), confirm the copier is Zink on NVK, time the copy
  again. Hubble's battery was run flat by Dave meanwhile (the history kept
  the drain to 0%).
- **NVK installed on Hubble** (Void's `mesa-vulkan-nouveau`, `vulkan-loader`;
  test tools glmark2 and mesa-demos too): nouveau's GL is now "zink Vulkan
  1.4 (NVIDIA GeForce GTX 1650 (NVK TU117))", and the server logs "card1
  copies with zink ... NVK". A window moved 20 times across the 14": no
  relay turn over 20 ms, one slow frame in all (the copy had been llvmpipe,
  the CPU, at 40 to 77 ms a full frame). Boot note: with the 14" on the HDMI
  port at boot, nouveau took about 70 s to bring up its console (GSP
  firmware), and the text console seemed to have no keyboard until Dave
  unplugged it. install.sh should say, where a GPU's GL would be software,
  which packages give it a driver (step 5).
- **Benchmarks on Hubble with NVK** (glmark2): offscreen 1280x720, Intel
  4383 and the NVIDIA card thousands of FPS a scene; **offscreen 4K, Intel
  397, NVIDIA 1229** (3x; the heaviest blur 96 vs 391 FPS). Full screen on
  the 14" rendered by the NVIDIA card (`DRI_PRIME=1`: NVIDIA renders, Intel
  composes, NVIDIA copies to the screen): at 300%, 176 to 228 FPS but
  glmark2 drew a 3840x2160 frame as points and showed its top-left third
  (it ignores the scale); with the 14" set to 100% for the test (restored
  after), **native 4K, 124 to 171 FPS, score 148**, 1 slow frame in 48 s, a
  full 4K copy about 15 ms (one 23.8 ms). Open: clients are asked for frames
  faster than the screen refreshes (glmark2 above 60 FPS on a 60 Hz screen;
  46 FPS in a window on the panel earlier): frame callbacks should follow
  each screen's own refresh.
- **Touch on the 14"** (Dave plugged its USB, 2026-11-14): the panel seen
  ("Silicon Works Multi-touch SW4101C"), tied to the 14" on the
  Touchscreen page (its two-screen choice, untried until now), and the
  server logged its touches "on screen ae05ec511722c55d" (the 14"): touch
  by USB, the picture from the NVIDIA card's HDMI (acceptance 2's touch). Dave:
  Firefox on the 14" "scrolls clicks and zooms FB".
- **Frame pacing**: glmark2 benchmarks with vsync off (swap interval 0), so
  its frame rates were throughput, not pacing (the 46 FPS earlier was the
  CPU copy). Paced with es2gears, which waits for each frame request: a
  relay turn that only looks whether the copy may start no longer asks
  windows for their next frame. Hubble: es2gears **144.0 FPS on the panel
  (144 Hz), 59.98 on the 14" (60 Hz)**.
- **Step 5's Screens page in** (2026-11-14): bus protocol **v8**, each
  screen's `OutputInfo` says which GPU shows it and which draws it
  (`shown_by`, `drawn_by`, the GPU named by its driver: "NVIDIA graphics
  (nouveau)"); a screen shown by one GPU and drawn by another has a
  Graphics row in its Display section ("Shown by NVIDIA graphics (nouveau),
  drawn by Intel graphics (i915) and copied there"), every screen one in
  Advanced; and Shown at says what the scale comes to ("it looks like 1280
  x 720 (3840 x 2160 pixels)", Dave's ask). `vbus outputs` prints both. On
  Hubble over the bus: eDP-1 Intel/Intel, HDMI-A-1 NVIDIA/Intel. All three
  machines on v8 (Techbench's dev session; its installed one needs Dave's
  sudo). 476 tests (kepler 472).
- **install.sh names a second GPU's driver** (step 5): on Linux, for each
  nouveau GPU it checks for NVK's ICD and the Vulkan loader and says either
  "OpenGL through Zink on NVK, on the GPU" or which packages to install
  (Void, Fedora, Debian names) because its screens would be drawn by the
  CPU; it installs nothing. Hubble: "on the GPU" (it said otherwise before
  the check looked at each file on its own). Techbench (i915 only) and
  kepler (FreeBSD) print nothing new.

## Steps

1. **Probe**: every GPU opened read-only enough to list it; capabilities
   logged; `vctl gpus`. No behavior changes.
2. **Two GPUs in the backend**: the backend holds a device per GPU (its
   DRM device, GBM, connectors, surfaces, scanner); surfaces keyed by GPU
   and CRTC; the second GPU's connectors scanned and hotplugged; nothing
   drawn on them yet (logged).
3. **Way 1, drawn here, shown there**: a buffer format both GPUs share,
   allocated on the showing GPU and imported as a render target on the
   drawing one (or the reverse), page-flipped by the showing GPU; frame
   pacing from its vblank.
4. **Way 2, copied**, when step 3's probe finds no shared format; and a
   test with a fake second GPU (vkms or a software device) so the copy
   path runs on kepler and Techbench too.
5. **Power and hotplug**: the second GPU released when its last screen
   goes; suspend measured on Hubble; the Screens page says which GPU shows
   each screen and which draws it (Advanced: the way, frame times).
6. **Docs**: whitepaper §8.3 and §9, README, CLAUDE.md, this plan.

## Acceptance

1. Tests green on Hubble, kepler and Techbench; nothing changes on the
   one-GPU machines (kepler, Techbench).
2. On Hubble: the 14" 4K monitor on HDMI shows the desktop at its own
   scale beside the laptop's panel; windows move between them; its touch
   lands on it once tied; the arrangement and window memory work as for
   any second screen.
3. Frames on the HDMI screen keep up at 4K 60 Hz with way 1 (frame times
   logged), and the Battery page shows the NVIDIA GPU awake while it is
   plugged in.
4. Unplugged, the NVIDIA GPU sleeps again (the Battery page says Asleep)
   within a minute, and the idle draw on battery returns to about 9.4 W.
5. Hotplug both ways while the session runs, and with the screens off
   (energy.rs), without a restart.

## Questions for Dave

1. **Whose code for the second GPU.** Smithay (already in, MIT) has a
   multi-GPU layer (`GpuManager`, `MultiRenderer`) that does ways 1 to 3.
   (a) Use it now, with more of Smithay to replace later (the renderer is
   next in docs/borrowed-review.md's order anyway); (b) our own, just ways
   1 and 2 against the DRM, GBM and EGL calls we already make. Proposed:
   **(b)**: it is the part we will replace next, and ways 1 and 2 are small.
2. **Which GPU draws an external screen by default.** (a) The Intel GPU
   (way 1: the NVIDIA card only displays; best for the battery); (b) the
   screen's own GPU (way 3, later). Proposed: **(a)** now, with a per-screen
   choice when way 3 comes.
3. **nouveau or NVIDIA's driver on Hubble.** (a) nouveau (now: display
   works with the GSP firmware, NVK for rendering later); (b) NVIDIA's own
   driver (Void's `nvidia` package; needs `modeset=1`, then explicit sync).
   Proposed: **(a)** for this milestone; NVIDIA's driver and explicit sync
   as the next GPU milestone, since other people's machines will have it.

## Where acceptance stands (closed 2026-11-14)

1. Tests: 476 on Hubble and Techbench, 472 on kepler; the one-GPU machines
   unchanged (kepler, Techbench). **Met.**
2. The 14" on HDMI at its own scale beside the panel, windows moving between
   them, its touch on it, the arrangement and window memory: **met** (Dave:
   Firefox "scrolls clicks and zooms FB").
3. Keeping up at 4K 60 Hz: **met** with NVK (no relay turn over 20 ms moving
   a window; native 4K glmark2 124 to 171 FPS; es2gears 59.98 FPS); the
   Battery page shows the NVIDIA GPU awake while plugged.
4. Unplugged, the NVIDIA GPU asleep and the draw falling back: **met**
   (13.3 W to 9.9 W in 30 s).
5. Hotplug both ways while the session runs: **met**. With the screens off
   (Dave, 2026-11-14): unplugged while off, the 14" stayed listed after
   waking: the NVIDIA card had gone to sleep with its screen blanked, and a
   sleeping card sees no cable leave (the kernel still said "connected").
   Fixed: the other GPUs' connectors are probed again (a forced probe, which
   wakes the card) whenever the screens come back on. **Met** once retried
   (Dave, 16:56: woke it, the 14" removed 2 s later, the windows back on the
   panel, the NVIDIA card asleep again). Cost: the probe wakes the card while
   the server waits, a 0.45 s hitch on waking; carried over: probe off the
   main thread.
