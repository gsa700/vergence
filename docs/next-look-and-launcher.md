# Next: the Apps launcher and the frosted look (notes for the plans)

Agreed with Dave on 2026-11-12 ("yes to all"), from two macOS screenshots
he took (the Apps launcher; Mail's translucent toolbar). To be drafted as
two milestone plans for him to accept, the blur first since both use it.

## Shelf milestone 2: the Apps launcher

One thing, not two: the dock's apps popup becomes a launcher.

- A floating panel in the middle of the screen, opened by the dock's apps
  button or the Super key.
- A search field across the top, focused at once: typing filters the
  grid live, the arrow keys move the selection, Enter starts it, Escape
  closes it (as AlbumWall's search does).
- Category tabs ("Utilities", "Entertainment", ...) from the Categories
  every desktop entry already has: nothing kept by hand.
- Larger icons with their names under them, in our look; the panel
  frosted (the blur below).
- Later: the same field finds settings pages ("display" opens Screens);
  apps first.

## Look milestone 1: the frosted bar

- **The blur**: a blur pass in the server's GPU renderer (a strip the
  height of a bar, and the launcher's panel). CPU drawing and a new
  **Reduce transparency** switch (beside High contrast and Reduce motion)
  get a plain translucent tint instead.
- **Frosted bars**: a native window's content runs to its top edge,
  under the bar; the server draws the bar over it, frosted (what is under
  it blurred and tinted), the tabs shaded on that, the buttons on top. In
  the terminal, output scrolls up under the frosted bar, as the letter
  scrolls under Mail's toolbar. Wayland and X11 windows keep their bar
  above their content (they cannot draw under it); their bar can frost
  what is behind the window.
- **Red for the superuser**: a terminal tab turns red, and the frosted bar
  takes a red tint when that tab is in front, while the program in the
  tab's foreground runs as root (sudo -s, su, a root shell). The terminal
  sees each tab's foreground process group and its effective uid (Linux
  /proc, FreeBSD's process information).
- **Light and dark** (Dave: "an inverse model for the dark lovers"): the
  frost follows Settings > Appearance > Style: a pale frost in light, a
  smoky dark one in dark (what is under it shows through darkened, light
  text and controls on it); the tabs, the launcher and the red tab each
  in both, contrast checked in both.
- **Rounded window corners** (waiting until the server draws the frames,
  which it now does for every native window): the server rounds them, at
  the one radius used everywhere (5 pt, decided 2026-11-10: "consistency
  throughout"); square when maximized; the border and the frosted bar
  follow the curve; the resize band keeps its corner cursors.
- **The bar's proportions, reviewed with the corners** (Dave, 2026-11-12):
  the bar's height (36 pt now), the window buttons' size (32 x 30) and the
  tabs' size and spacing (the 6 pt margin rule), looked at together with
  mocks before anything is changed.
