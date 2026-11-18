# Headless milestone 1: screens that are not there (display testing, layer 3)

**Drafted 2026-11-18** (Dave: "yes, plan layer 3"). The third layer of the
display testing method: layers 1 and 2 are in (geometry tests in `cargo
test`; `tools/display-check.sh` on a live session, see CLAUDE.md "Display
checks"). Layer 2 needs a real monitor, takes it over for minutes, and can
only test the monitors a machine has. Layer 3 lets the server draw screens
that exist only in memory, of any size, density and scale, plugged in and
pulled out by a script, so that:

- display checks run on any dev box over SSH, without touching a screen
  Dave is using, and on all three platforms the same way;
- the cases we cannot cable up get tested: three screens, a 5K, a 1440p, a
  screen on a TV's distance, a laptop lid closing, a screen of another
  scale appearing under an open window;
- hotplug, arrangement, window rescue and window memory per setup (the
  bugs of 2026-11-12 and -17) get tests that run every time.

## What is there to build on

- Each screen is a `drm::Surface`: its EDID identity, modes, size in mm,
  scale notes (displays.rs), wl_output global, place in the arrangement.
  All of that is the same for a screen in memory; only how a frame gets
  out is not.
- The relay (GPUs milestone 1) already draws a screen into an off-screen
  buffer on the primary GPU and reads it back (`read_back`, `capture` for
  `vctl shot`).
- `vergence-harness fake` makes EDIDs (name, product, serial, mode, mm).
- `vctl scale` (layer 2) sets scales at runtime, never saved.

## Design

**A screen in memory is a Surface with no connector.** `Surface` gets a
kind: `Drm { drm_output, handle }` as now, or `Virtual { buffers, clock }`:
two GBM buffers on the primary GPU (linear, so a shot is a plain read), a
timer at the screen's refresh standing in for vblank, sending frame
callbacks exactly as a monitor does. Everything above the Surface (scale,
arrangement, window memory, the dock, notifications, `vctl windows`,
`vctl shot`, the bus's `Outputs`) sees an ordinary screen.

**Two ways to have them:**

1. **Beside real screens, in a running session**: `vctl screen add <name>
   --edid FILE | --fake <W>x<H> <mm-W>x<mm-H> [--distance laptop|desktop|tv]`,
   `vctl screen remove <name>`. A test screen is never saved in the
   arrangement or window memory (as `vctl scale`). For hotplug tests on the
   real machines, mixed with real monitors.
2. **With no screens at all**: `vergence-server --headless [--screen
   <spec>]...`: no libseat, no DRM master, no libinput; the GPU through its
   render node (`/dev/dri/renderD128`, which any user in the video group may
   open), every screen virtual, input only through `vctl` (which already
   drives everything). It runs as a second session beside Dave's, with its
   own runtime directory, Wayland socket, bus and control socket: tests
   over SSH never touch the session on the monitors. `tools/vergence-session
   --headless` sets it up; apps started in it find its bus by environment as
   now.

**Pixels are checked two ways:** by rules (layer 2's `vergence-harness
seams`, run on headless shots), and, where a picture is fixed (the test
pattern, a bar, a menu), against the CPU reference renderer's picture of
the same scene (`VERGENCE_RENDER=cpu`), the way vergence-render's GPU tests
already compare byte for byte within a few steps. No stored reference
images: they would be large, and the CPU renderer is the reference.

**A suite, not one script**: `tools/display-suite.sh [host]` starts a
headless server, runs every scenario in `tools/display-suite/*.sh`, each a
short vctl script with its checks, and prints one line per scenario. First
scenarios, each from a bug or a rule we already have:

| Scenario | Checks |
|---|---|
| seams | layer 2 at every clean scale and three off-grid ones |
| text factor | a screen at 133% of a 27" 4K: factor 1.06 at strength 25%, apps told it |
| hotplug add | a window stays put; the new screen goes right of the others |
| hotplug remove | windows on it are rescued onto the main screen, inside it, on its grid |
| scale change under a window | it is re-sized on the new grid, its bar and body still meet |
| setup memory | windows return to their places when a setup comes back (per size and scale) |
| two scales side by side | a window dragged across takes the scale and text of the screen it is mostly on |
| lid and TV distance | defaults: laptop panel closer, TV farther (displays.rs) |

## Steps

1. **Surface kinds**: split what is DRM's out of `Surface` (drm_output,
   connector handle, modes from the connector) behind the kind; the frame
   loop asks the kind how to present. No change to anything a monitor does;
   the tests on all three machines and a session on each show it.
2. **Virtual screens in a session**: `vctl screen add/remove`, GBM buffers,
   the frame clock, `vctl shot` from them; never saved. Check: a 2560x1440
   fake beside Hubble's panel, a window dragged onto it, shot.
3. **`--headless`**: the render-node backend with no seat or input, its own
   sockets; `tools/vergence-session --headless`. Check: started over SSH
   on all three machines while a session runs on the monitors, `vctl
   windows` and a shot.
4. **The CPU picture**: `vctl shot --cpu` draws the same frame with the
   reference renderer; a check compares the two within the GPU tests'
   tolerance.
5. **The suite**: tools/display-suite.sh and the first scenarios above;
   layer 2 runs on headless screens by default (`display-check.sh --headless`).
6. **Docs**: CLAUDE.md, whitepaper §9; the suite in the order of work
   (every display change runs it on the three machines before it is pushed).

## Questions for Dave

1. **Headless as its own start mode** (step 3), or virtual screens in a
   running session only (steps 1-2)? I lean to both: in-session screens
   test real mixes, headless keeps tests off your screens.
2. **The suite before every push** of display code, on all three machines,
   as the tests are now? (A few minutes each; it can run in parallel.)
3. **Fake EDIDs only** (vergence-harness fake, our own numbers), or also
   dumps of the lab's real monitors kept in the repo? Real dumps carry
   serial numbers (the ASUS's is already a concern for going public), so I
   lean to fakes with the real sizes.

## Progress

- Not started.
