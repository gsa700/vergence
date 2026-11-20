# Text milestone 1: kerning, weight, sharp text at low density, optical sizes

Status: **accepted 2026-11-19** (Dave: "do both, make the milestone doc and
then go ahead"). Steps 1 to 4 built 2026-11-19 (step 3 by pixel size,
not screen scale: below 24 px to the em; hint.rs; the GPU reads the CPU's
coverage tables as a texture since) (kerning matches
HarfBuzz at weights 400, 700, 900; GPU and CPU agree within one step; tests
and the display suite 10/10 on Hubble, kepler, techbench). Shots in
~/vergence-display-checks/text-1/ on Control (`vscene type`). Questions for
Dave at the end (steps 5 and 6 wait on them).

## What Dave asked for

- Wishlist: "We need to check our font layouts against publishing standards
  and readability guidelines and make Vergence the BEST at font rendering
  and scaling. Extra focus on the window and app proportions etc."
- 2026-11-19: "the 22" asus is the worst of all the displays so far, no
  doubt the pixel density has something to do with that but #3 as well."
  (The ASUS VS229 on techbench: 1920x1080 on 21.5", about 102 ppi, scale 1.)

## The audit (2026-11-19)

Compared with macOS (Core Text), Windows (DirectWrite), Chrome (Skia) and
FreeType.

Already at the standard: glyphs placed at four sub-pixel positions across
(as Skia and Chrome), baselines on whole pixels, grayscale smoothing only
(macOS since 10.14; right for high density and rotated screens), a glyph
rasterized once per size whatever its color, contrast held to WCAG 2.
Sizes: interface text 13, menus 13.5, title bars 13 (macOS 13 pt = 13 px
at 1x; Windows 11 14 px; GNOME Cantarell 11 pt about 14.7 px). Inter's
x-height (0.546 em, against about 0.5 for Segoe UI) makes 13 read like 14
elsewhere.

Found, worst first:

1. **Inter is not kerned.** Inter Variable keeps its kerning in `GPOS`
   only, no legacy `kern` table; ab_glyph's `kern_unscaled`
   (owned_ttf_parser's pre-parsed subtables) reads `kern` only. So "AV",
   "To", "Ty", "Yo", "r." are set at their bare advances. JetBrains Mono is
   fixed pitch, so no loss there.
2. **Weight differs between light and dark.** Coverage is blended into
   sRGB values as stored (vergence-render lib.rs). That keeps dark ink on a
   light ground at a familiar weight (a little heavy if anything), but
   light ink on a dark ground comes out thinner than it should: a
   half-covered pixel lands half way in sRGB values, about 21% of the
   light instead of 50%. So dark mode reads thinner than light mode.
   Every platform shapes coverage by the ink: Skia (Chrome, Android, and
   Linux browsers) with a contrast and gamma table chosen by the ink's
   luminance (exponent 1.2, contrast 0.2 by default), DirectWrite with
   gamma 1.8 and "enhanced contrast", macOS with font dilation, FreeType
   with stem darkening. (Corrected 2026-11-19 evening: the first audit had
   the direction backwards.)
3. **Soft at low density.** No hinting and no stem darkening. At about 100
   ppi a 13-point Inter stem is about 1.1 px wide and its x-height 7.1 px:
   stems straddle two pixels, the x-height's top edge is a half-gray row.
4. **Inter's optical size axis is never set.** `opsz` runs 14 to 32 and is
   left at 14 for every size; headings (20 to 28) should get the display
   cut, tighter and finer, as browsers do (`font-optical-sizing: auto`).
5. **The size in Settings' font is not used by our apps.** In
   "Inter Variable 11" the toolkit reads only the family; its text size is
   fixed at 13. The server's own text reads the number.
6. **Line spacing.** Interface text takes the font's own (Inter 1.21 em),
   right for labels. Edit takes max(font, 1.3), the terminal the font's
   (JetBrains Mono 1.32): fine for code, tight for prose. Publishing
   guidance for running text is 1.4 to 1.5; WCAG 2 (1.4.12) wants 1.5 to
   be possible.

## Steps

1. **Kerning from GPOS** (vergence-text, ours: kern.rs). The `kern`
   feature's lookups (pair adjustment, formats 1 and 2, through extension
   lookups too) for the default script and Latin, read once per face into
   a pair table. Variable fonts: the x-advance's variation delta from
   GDEF's item variation store at the face's own coordinates (fvar
   normalized, then avar), so bold is kerned as bold. The legacy `kern`
   table stays as the fallback for fonts that have only that. Untrusted
   input like the rest of sfnt.rs: every offset checked, bounded, fuzzed.
   Tests: "AV" and "To" narrower than their advances in Inter at 400 and
   700; JetBrains Mono's widths unchanged.
2. **Weight correction by the ink** (vergence-render, CPU and GPU alike).
   Skia's model: the ground assumed to be the ink's opposite, the blend
   worked out at a gamma between 1 (today) and 2.2 (physically linear),
   then a contrast lift. Light ink gets fuller edges; dark ink stays about
   as today. The CPU builds a 256-value table per ink color; the GPU shader
   evaluates the same function. The frosted bar's contrast rule and WCAG
   checks are unchanged (they are on colors, not coverage). Gamma and
   contrast set by eye on techbench's LG and ASUS, light and dark, before
   and after.
3. **Low density** (built as: by the em in pixels, below 24; hint.rs).
   (a) The x-height on a whole pixel: glyphs rasterized with their vertical
   scale nudged so the face's x-height (OS/2 `sxHeight`) rounds to whole
   pixels, horizontal scale and advances untouched, so layout does not
   move (ab_glyph's PxScale takes x and y apart). The nudge is a few
   percent at most and only below 1.5x. (b) Stem darkening, FreeType's
   idea: more darkening at small pixel sizes, none from about 24 px up,
   folded into step 2's curve. No full TrueType hinting (macOS has none
   either; what macOS adds is darkening).
4. **Optical sizes** (built: Fonts::sized; layout and both renderers ask for it). A variable font with `opsz` is set to the text's size
   in points (whole numbers, clamped to the axis), so Inter's headings use
   its display cut. Layout and raster agree because the size picks the
   instance before either. Faces are cached per (file, weight, opsz).
5. **The size in Settings** (waits on question 1).
6. **Line spacing in Edit** (waits on question 2).
7. **Checks.** A type sample (vergence-ui example `typesample`: Inter and
   JetBrains Mono at 11 to 28, kerning pairs, light and dark) shot with
   `vctl picture` on Hubble, kepler, techbench's LG and ASUS, before and
   after each step, kept in ~/vergence-display-checks/text-1/ on Control.
   The display suite before every push. Unit tests for kerning, the curve,
   the x-height nudge and the optical size choice.

## Not in this milestone

- Full shaping (ligatures, Inter's contextual alternates, tabular figures
  by feature, combining marks placed by `mark`/`mkmk`, right to left,
  scripts that need shaping; whitepaper §10 question 8). The direct road
  is rustybuzz (pure Rust HarfBuzz port, MIT): borrowing, so Dave's call
  (question 3). Step 1's kern.rs is what we keep either way for fonts and
  callers that do not shape.
- Subpixel (RGB) smoothing: not planned; grayscale is the modern choice and
  safe on rotated and OLED screens.
- GTK, Firefox and X apps' text (their own renderers).

## Questions for Dave

1. **What the number in Settings' font means.** (a) Points at 96 per inch,
   as GTK reads it, so our apps and GTK apps agree; the default becomes
   "Inter Variable 10" (13.3, about today's 13). (b) Drop the number from
   the picker and keep size in Accessibility's Size. Recommended: (a).
2. **Edit's line spacing.** A setting (Compact 1.3, today; Comfortable
   1.45; Loose 1.6), default Comfortable for prose files (.md, .txt) and
   Compact for code? Or one default for all?
3. **Shaping by rustybuzz** (borrowed, MIT) in a later milestone, or our
   own shaping for Latin features only?
