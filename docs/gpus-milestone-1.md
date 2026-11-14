# GPUs milestone 1: screens on a second GPU (Hubble's HDMI)

Drafted 2026-11-14, when Dave plugged the 14" 4K touch monitor into
Hubble's HDMI port and nothing showed (Dave: "yes, draft the multi-GPU
milestone plan"). The whitepaper's §8.3 has the design this milestone
starts: one renderer per GPU, capabilities probed and never assumed by
vendor, explicit sync later. **Draft: questions for Dave at the end.**

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
