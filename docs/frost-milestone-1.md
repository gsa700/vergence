# Frost milestone 1: frosted glass behind see-through windows

**Closed 2026-11-25** (Dave, on techbench with vergence-0.4.0pre12: "The
terminal looks great"; at 90% "a great balance", the light schemes bright
as expected). Built for the terminal alone: frosted glass, the bar as the
window's glass in its colors, its text running up under the bar blurred.

**Drafted 2026-11-24**, from Dave's wish (docs/apps-roadmap.md, "Frosted
see-through windows, as a setting"): "add frosted blur to the wish list, as
a setting", then "Let's get blurry! the macOS actually has two different
levels of blur depending on wether Terminal is in focus, or not. It is a
little MORE transparent when IN focus."

Why: a window less than opaque (techbench's terminal at 97%) shows the
window behind it sharp; an editor's text read through Claude Code's terminal
looked like a stale frame (entry 01M85GX6SXDYSS4NJCMH1X99RK). Frosted, what
is behind keeps its colors and shapes and loses its text.

**Answered 2026-11-24** (Dave): "only do this to the terminal window, none
of the other apps will benefit from this". So the terminal alone; the switch
is the terminal's own (its Settings, beside Opacity), not the desktop's; the
other leans taken (the bar frosted with the window; the terminal not in front
more frosted).

**Built 2026-11-24**, the same evening: no server change and no bus change.
- vergence-ui sends a window's style again when it changes while the window
  is open (it was sent at creation only), so the frost follows the opacity
  and the switch at once.
- vergence-term: `[terminal] frosted` (on by default; Settings > On this
  desktop > Frosted glass). While on and below 100% it asks for the frosted
  style; its ground is drawn over the frost at its opacity, 10 points more
  opaque while another window is in front (UNFOCUSED_MORE); the strip under
  the bar stays empty (lines there would show sharp behind the tabs).
- Display-suite scenario 31-term-frost: a terminal at 80% over a pattern;
  frosted, the pattern's edges behind it are gone (14,292 to 0 on the
  measure); in front it is brighter than behind (34.9 against 32.7); frost
  off, focus changes nothing. Full suite 31 of 31 on Hubble.

- **After Dave's screenshots** (fd40550, pre10): the bar tracks the window
  (blur-only frost for a window whose glass is its own; its ground colors
  bar and body) and takes the terminal's colors (Dark or Light by its
  scheme). Dave settled on 92% ("close to the macOS"). The Title bar row is
  shown locked while the glass applies. Text scrolling under the bar is
  parked (docs/apps-roadmap.md).
- **Text under the glass bar** (2026-11-25, Dave: "it's different than all
  the other windows now"; un-parked): the terminal's lines run up under the
  bar again, as under every bar. The server draws the picture's top strip
  through the frost shader's `clear` mode (blur.rs): the picture's own alpha
  kept, the blur moved toward the window's page color by the desktop's frost
  strength (as other bars soften the text under them), no contrast floor; in
  front of the window's frost, behind the bar. The picture's own sharp strip
  is cut (decor.rs `glass_cut`). The glass bar itself stays: a see-through
  window's bar shows what is behind it, as every bar does (Terminal.app's
  bar is solid gray; this is our rule, not the Mac's). Without the frost
  (Reduce transparency, high contrast, software drawing, the session's CPU
  scenes) the sharp strip is still cut and nothing is blurred in: the strip
  stays empty, as before. Scenario 31 now also fills the terminal and checks
  the strip under the bar changes.

The design below was written before the answer: where it says every app or
Appearance, read the terminal and its Settings.

## Where things stand

- **Every toolkit app has an Opacity**: its own (`opacity` in its section:
  Terminal, Edit, Settings, FileView, Calculator, Close-Up) or the desktop's
  (`[appearance] window-opacity`), vergence_ui::opacity_setting. Below 100
  the app draws its ground with that alpha and the server composites what is
  behind, sharp.
- **The server already frosts a whole window**: the `frosted` window style
  (bus v36, preview milestone 1, step 6b) asks for no ground of its own;
  the server frosts what is behind the body and the bar as one surface,
  rounded at all four corners (decor.rs `body_frost`, backdrop.rs: a
  framebuffer effect, redrawn only when what is behind changes). FileView's
  Quick Look-style preview uses it. Off with Reduce transparency, High
  contrast and software drawing (the ground is then solid).
- **The frost's strength** is the desktop's (Appearance > Frost: Clear,
  Standard, Heavy; vergence_look::frost_strength), the same for bars and the
  dock; its tint is raised, pixel by pixel, where the ink on it needs
  contrast (blur.rs).

## Design

1. **A see-through window is frosted**: when an app's ground is below 100%
   and frosted glass is on, it sets the window style's `frosted` (no bus
   change: the flag exists) and goes on drawing its ground at its opacity,
   over the frost. So the window shows its own color (the terminal's scheme,
   the page's gray) a little, and behind it what is there, blurred and
   tinted. At 100% nothing changes.
2. **Focus, as macOS**: the window in front is the more see-through; the
   others are more frosted. The server knows focus: an unfocused frosted
   window's frost tint is heavier (lean: one and a half times the focused
   one's), so it settles back and the one being typed in is clearest.
3. **The bar is part of the glass**: as with the preview, the bar and the
   body are one frosted surface (macOS Terminal's title bar is translucent
   with its window).
4. **One switch**: Appearance > "Frosted glass behind see-through windows",
   on by default, with the Frost strength already there. Reduce
   transparency and High contrast turn it off as they do the bars' frost.
5. **Cost**: the frost is redrawn only when what is behind the window
   changes (typing in a frosted terminal does not re-blur it). A large
   window at 4K is the most it costs: measured on kepler (UHD 630) and
   techbench before it ships; if it is slow, the blur works at half
   resolution (no visible difference at that radius).

## Steps

1. The toolkit: an app whose opacity is below 100 asks for `frosted` when
   the desktop's switch is on (vergence-ui; every app at once), and keeps
   drawing its ground at its alpha. The terminal first, then the others
   (they share the code).
2. The server: a frosted window with a ground of its own (the frost under
   it, its ground over) and the focus difference (the unfocused tint
   heavier); the switch, Reduce transparency and High contrast.
3. Settings > Appearance: the switch beside Frost.
4. A display-suite scenario: a see-through terminal over a test pattern,
   frosted and not (the pattern's edges blurred behind it with the switch
   on, sharp with it off: `vergence-harness sharper`), focused and not
   (the unfocused one more tinted). Timing on kepler and techbench.
5. A package for techbench (the server changed: log out and in).

## Questions for Dave

1. **One desktop switch, or per app too?** Lean: **one desktop switch**
   (Appearance), on by default; each app's Opacity already says how
   see-through it is. A per-app switch later if wanted.
2. **How much more frosted when not in front?** Lean: **the frost tint one
   and a half times the focused window's**, no setting of its own (the Frost
   strength scales both).
3. **The bar frosted with the window** (one glass surface, as the preview
   and macOS Terminal), or kept as it is now? Lean: **one surface**.
4. **Which apps?** Lean: **every app with an Opacity below 100** (they
   share the code), the terminal first in testing.
