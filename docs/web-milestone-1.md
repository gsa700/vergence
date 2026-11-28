# Web milestone 1: pages in a Vergence window

*Drafted 2026-11-27 on Control from techbench's plan of 2026-11-26 (shared
memory 01M8DFXCH2CA9BDMAQCYQHJZJJ, the web plan; 01M8DGMVR6FB30Y6WWW2S9KZAQ,
the browser's Protection design) and the roadmap's "The web component"
(docs/apps-roadmap.md). Dave: "webkit in a buildable component model. So we
can include it with any of our apps and others could too." The questions
below were answered 2026-11-27 (see "Dave's answers"); nothing is built.*

**Decided already (Dave, 2026-11-21 and 11-26):**
- The engine is **WPE WebKit** now, and vergence-web is built so the engine
  can be swapped later (Servo): the interface apps see has no WebKit names
  or settings; browser data lives in our stores, not WebKit's; content
  blocking at our own layer; an engine-neutral conformance suite decides
  when another engine is ready.
- **No smooth scrolling:** a wheel notch is an exact step, as everywhere in
  Vergence; touchpad momentum follows Vergence's setting.
- **techbench builds WebKit** for now (a full build about 15 minutes there,
  -j16 at most on 31 GB).
- Order: after Vergence Music (closed 2026-11-27).

## Why

Mail, Preview showing HTML, help pages, and a browser not dependent on
Google all need a web view, and each would otherwise embed one its own
way. One component, a process of its own that any program can place in its
window over Vergence's protocol, gives them all the same engine, the same
look and the same security, and keeps WebKit (LGPL, large) out of every
app.

## What the spike showed (techbench, 2026-11-26)

WPE WebKit 2.54.3 built in 14 min 40 s (8890 steps, 27 GB peak RAM, 166 MB
installed) and showed https://wpewebkit.org/ in Dave's Vergence session,
through WPE's own Wayland platform. Since 2.54 the **WPE Platform API**
(WPEDisplay, WPEToplevel, WPEView) is WPE's stable way to put pages on a
screen; a platform of our own talking Vergence's protocol is written
against it. Neither Void nor FreeBSD packages wpewebkit: we would build and
package it ourselves on both.

## The plan for milestone 1

1. **WebKit built and packaged:** a Void template for WPE WebKit 2.54.x
   from the spike's settings, with WPE's Wayland and DRM platforms off
   (they bring libinput, udev, Wacom, Lua) and its headless platform on for
   tests; built on techbench, with a script that rebuilds it for each
   WebKit security release (about monthly).
2. **vergence-web, the process:** hosts WebKit and **a WPE platform of our
   own** (question 6): a display connected to Vergence's bus, a view per web
   region, frames out as GPU buffers (dmabuf) in the native protocol's
   window pictures, and input in.
3. **Frames:** each WebKit frame goes to the server as a window picture
   (the native protocol already carries dmabuf planes), at the screen's
   scale.
4. **Input:** pointer (move, buttons, wheel by notches, smooth scrolling
   off), keys with text, focus, and the pointer's shape from the page.
5. **A test window:** `vergence-web-view URL`, a plain window showing one
   page, for trying it and for the checks.
6. **Checks:** a display suite scenario loading a local page (fonts, CSS,
   a script) and comparing its picture; wheel steps exact; keys typed into
   a form field; the conformance suite's first pages (rule 5 above).

**Milestone 2 and on** (not this milestone): web regions inside other
apps' windows (focus across the boundary, text input, clipboard, popups and
select lists drawn in Vergence's look); 3, meaning (pages read into the
meaning tree for agents, the magnifier and screen readers); 4, the browser
(tabs, history, bookmarks, downloads, Protection).

## Questions

1. **Build and package WebKit ourselves** on Void (and FreeBSD later): the
   big borrow (GLib, libsoup, HarfBuzz, ICU, image libraries), LGPL, in a
   process of its own. (a) yes; (b) no, wait for distributions. *Lean: (a);
   neither ships it, and the spike measured the cost.*
2. **A service process, or a library apps link:** (a) a process (any app
   in any language embeds it; WebKit never linked into our apps; one place
   for security updates); (b) a library. *Lean: (a).*
3. **Video and audio in pages** (GStreamer): (a) out of the first version;
   (b) in. *Lean: (a); it doubles what is borrowed and packaged.*
4. **First program to use it, after the test window:** (a) Mail's message
   view (static HTML, scripts and network off: the gentlest); (b) the
   browser; (c) Preview showing HTML files. *Lean: (a), after milestone 2.*
5. **Content blocking (milestone 4):** Brave's adblock-rust (Rust, MPL-2.0,
   reads uBlock's lists) or our own filter engine. *Lean: decide at
   milestone 4.*
6. **The WPE platform in Rust or C:** WebKit's platform API is GObject (C).
   (a) Rust with our own small binding to the few GObject calls needed (as
   vergence-dbus is our own); (b) a small C file for the GObject classes,
   the rest Rust; (c) borrow the glib-rs crates. *Lean: (a), keeping Rust
   throughout and nothing more borrowed; (b) if the GObject boilerplate
   proves too much.*
7. **FreeBSD in milestone 1:** (a) Void first, FreeBSD once techbench runs
   it (the 13900K builds WebKit in minutes; kepler would take hours and
   leaves within a week); (b) try a FreeBSD build on kepler now. *Lean: (a).*

## Dave's answers (2026-11-27, 23:45)

1. **(a)** We build and package WPE WebKit ourselves: Void now, FreeBSD
   later.
2. **(a)** A service process.
3. **(a)** No video or audio in the first version.
4. **(b) The browser comes first**, not Mail. The browser's window is a
   native app around a web region, so the order is milestone 1 (pages in a
   test window), milestone 2 (web regions inside other windows), then the
   browser (tabs, history, bookmarks, downloads, Protection) ahead of
   meaning (milestone 3). Mail's message view follows.
5. Decided when the browser's Protection work starts (with the browser
   first, that comes sooner).
6. **(a)** Rust, with our own small binding to the GObject calls needed.
7. **(a)** Void first; FreeBSD once techbench runs it.

## The browser's form and the component's shape (Dave, 2026-11-28)

**The browser's look:** a plain Vergence window. The server draws its tab
bar (Bar::Tabs: tabs dragged between windows, the tab menu, the alarm red
for a superuser's tab, as Terminal's), and under it one toolbar the browser
draws: back, forward, reload, the address field, Protection's shield, the
menu. No status bar by default: a hovered link's address shows in a small
overlay at the page's bottom left while the pointer is on it; View > Status
Bar turns on a real bar (as Safari's switch). Not chosen: Safari's compact
bar (the browser drawing its own tabs, outside the server's) and the
address in a server-drawn bar.

**Frost under the toolbar (Dave asked; milestone 2):** the page scrolls up
under the tab bar and toolbar, frosted, as the terminal's lines do under
its bar. The server composites the page's pixels, so it can blur them; the
browser never sees them. Two pieces: (1) the native protocol lets a window
declare a glass band taller than the bar (the bar plus its toolbar): the
server frosts it and the browser draws its controls with no ground of
their own (a bus version step); (2) the page lays out with its top below
that band, so its top and any fixed header are not hidden at the start:
WebKit has this inside (WebPageProxy::setObscuredContentInsets, how Safari
scrolls pages under its toolbar) but WPE's API does not offer it, so a
small patch in our package and a spike to see WPE's scrolling honor it.
In the neutral interface it is a "top inset"; an engine without it gets
the page starting below the toolbar and plain glass.

**The component's shape: two interfaces, vergence-web between them.**

1. *Apps to vergence-web: a protocol on Vergence's bus.* An app places a
   web view in its native window as one more node of its window tree;
   vergence-web fills it with pixels (composited by the server) and gets
   its input. Asked, in our own terms: open a view with a policy (scripts
   on or off, network on or off, Protection on, which profile), load an
   address or an HTML string, back, forward, reload, stop, find, zoom, top
   inset. Told: title, address, load progress, connection security, the
   site's icon, the hovered link, requests for a new tab or window,
   downloads, permission requests (camera, location, notifications), and
   context menus and select lists, which the app or the server draws in
   Vergence's look. Our apps use a small Rust client crate; any other
   language speaks the protocol.
2. *vergence-web to an engine: a Rust trait inside vergence-web.*
   `engine-webkit` implements it now (through our GObject binding,
   question 6), `engine-servo` later (Servo is a Rust crate). Chosen per
   view, so two engines can run side by side. The trait includes per-site
   data clearing (Protection's Forget on Close needs it; WebKit's website
   data manager has it; any engine must).

History, bookmarks, site permissions and Protection's lists live in
vergence-web's own stores, never the engine's; the engine holds cookies
and site data while in use, cleared through the trait. The conformance
suite drives the app protocol, so the same checks judge any engine.
Processes: one vergence-web per session hosts every view; pages run in the
engine's own confined processes (WebKit's web and network processes).

*Later:* the server knows which part of a window is web content, so it can
add its own mark of the true site beside the address field, which no page
can fake.

## Research (2026-11-27, from the WPE WebKit 2.54.3 source)

**How a platform of our own registers: in code, no module needed.**
WPE looks for platforms as GIO modules (in its module folder or
`WPE_PLATFORMS_PATH`, chosen by `WPE_PLATFORM`), but that is only for
programs that take the default display (`wpe_display_get_default`).
vergence-web is the program WebKit runs in, so it registers its own
`WPEDisplay` subclass as an ordinary GType, connects it, and hands it to
each web view through the construct-only `display` property of
`WebKitWebView` (there since 2.44). No .so in WebKit's folders and no
environment variables.

**What the platform implements** (the headless platform, the smallest
reference, is 482 lines of C++): `WPEDisplay` (connect, create_view,
create_toplevel, EGL display, DRM render device, preferred buffer formats,
screens, keymap, clipboard, input method; gamepads optional), `WPEView`
(render_buffer: a frame in, as a dmabuf or shared-memory buffer;
buffers_changed, cursor by name or bytes, opaque rectangles, pointer lock,
get_accessible), `WPEToplevel` (title, size, maximize/fullscreen/minimize,
screen). A Rust binding for question 6 needs GType registration of
subclasses with class vfuncs, GObject properties and signals for these
few types: small, as leaned.

**Page accessibility: AT-SPI only, but on a bus we choose.** Each web
process publishes its page's accessibility tree itself, as AT-SPI objects
on a D-Bus message bus (WebCore's own AT-SPI code, not ATK); the program
hosting WebKit only receives a "plug id" (bus name and object path) for
each view's root (`WPEViewAccessible`'s `bind`). There is no other hook.
The bus is whatever `AT_SPI_BUS_ADDRESS` (or `WEBKIT_A11Y_BUS_ADDRESS`)
names when vergence-web starts the web processes; with none, WebKit asks
the session's `org.a11y.Bus`. So for milestone 3: vergence-web runs a
**private bus of its own**, only its web processes on it, reads each
page's tree there with vergence-dbus, and translates it into the meaning
tree. The web process needs a real message bus (it calls Hello and owns a
name) and sends change events only to listeners registered with an
`org.a11y.atspi.Registry`, so that private bus answers as both: a small
bus of our own inside vergence-web, or a borrowed dbus-daemon (Dave's
call when milestone 3 comes). Nothing of this reaches the session's real
AT-SPI bus. Milestone 1 sets no bus: pages then publish nothing.

## Progress

- Step 1 (2026-11-28): the template `packaging/void/srcpkgs/wpewebkit`
  (and `-devel`), WPE WebKit 2.54.3, checksum matching wpewebkit.org's,
  built on techbench with xbps-src in ~/.cache/vergence-lab/void-packages
  in about 18 minutes: `wpewebkit` 42 MB (117 MB installed:
  libWPEWebKit-2.0.so.1 with the WPE Platform API inside it, the web,
  network and GPU processes, WPEWebDriver), `wpewebkit-devel` (129 headers,
  pkg-config for wpe-webkit, wpe-platform and wpe-platform-headless). Its
  run-time dependencies hold no Wayland, libinput, X11 or ATK: the
  platform switches did what they should. Not yet installed anywhere; the
  rebuild script for security releases is still to come.
- Inset spike (2026-11-28, techbench, Dave's ask): **frost under the
  toolbar works with WebKit.** WPE 2.54.3 built with a small patch (21 lines: one function, its declaration, an include)
  exposing `webkit_web_view_set_obscured_content_insets()` (WebKit's own
  `WebPageProxy::setObscuredContentInsets`, which on non-Cocoa ports
  already reaches the web process); a headless 800x600 view, the frames
  WebKit hands the platform read through WPEView's `buffer-rendered`.
  With a top inset of 80: at the top the page starts below the band (a
  `position: fixed; top: 0` header drawn at y 80, the document's first
  block right under it, the scroll bar only beside the unobscured part);
  scrolled, the rows pass up through the band (what the frost blurs) and
  the fixed header stays pinned at 80; the page reports innerHeight 520 of
  600 and scrolls 80 further, so its bottom is reachable. The band is
  opaque black where the page has nothing (above the document at the top);
  with the view's background set transparent it is transparent, so the
  server shows the window's ground there (frosted), and vergence-web lays
  white under the page itself for pages with no background of their own.
  Safari fills that strip with the page's own background; a refinement for
  later. The patch is in `packaging/void/srcpkgs/wpewebkit/patches/`
  (built into the next package; 2.54.3_1 lacks it).
- Steps 2, 4 and 5, first cut (2026-11-28, techbench): **crates/vergence-web**
  with our own WPE platform in Rust and the test window
  `vergence-web-view [--size WxH] URL`. WebKit is loaded at run time
  (dlopen, `VERGENCE_WEBKIT_LIB` for a lab build), never linked, so the
  workspace builds and tests on Hubble and kepler, which have no WebKit.
  ffi.rs: our own binding, only the calls used. platform.rs: WPEDisplay,
  WPEView and WPEToplevel subclassed (GType registration; each class's
  size checked against g_type_query before any slot is written, so a
  WebKit of another layout is refused); WPE headless's frame contract
  (render_buffer, then released/rendered at the server's frame callback).
  No EGL display is offered yet, so WebKit draws on the processor into
  shared memory, which goes to the server as the window's picture as it
  is. Seen in a headless session on techbench: a local page, and
  https://wpewebkit.org/ over TLS, fonts and images right, the window
  titled from the page; a click on a link, typing into a field (keys
  through xkb), Page Down scrolling a page. Not yet: the wheel (the control
  socket cannot send one; try by hand), GPU buffers (dmabuf) for the
  frames, the display suite scenario, a live session.
- Step 3, GPU frames (2026-11-28, techbench): our display offers WebKit
  an EGL display on a GBM device (the first render node that works, as the
  toolkit's GPU renderer picks) and the matching DRM device; WebKit then
  draws on the GPU into dmabuf buffers, attached to the window as they are
  (no copy; a buffer is released once a later one is attached and a frame
  has followed, the protocol's rule). Without a GPU, or with
  `VERGENCE_WEB_CPU=1`, shared memory as before. Seen headless: the local
  page and wpewebkit.org drawn right, click, typing and Page Down as on
  the processor. Dave tried the processor build live on techbench: the
  wheel steps by notches, "really responsive, a little lag but not bad".
- Step 1 finished, step 6 (2026-11-28): wpewebkit **2.54.3_2** (with the
  inset patch) built on techbench and installed on techbench and Hubble,
  so the web view needs no lab paths. Display-suite scenario
  **42-web-view** (tools/display-suite/web/page.html): on the GPU and on
  the processor, the window titled from the page, the picture its size,
  the page's blocks and ground where the page puts them, a click, typing,
  Page Down; skipped where WebKit is not installed. 16 of 16 on techbench
  and on Hubble.
- Dave, 2026-11-28 01:20: building WebKit inside Vergence, or a Vergence
  port of WebKit, considered and set aside: WPE is one of WebKit's own
  ports (the same repository as Safari's), our platform is about 1,400
  lines against WPE's ~30,000 of platform code and ~13,000 of WPE-only
  glue we would otherwise carry and merge every release; we pin and build
  WPE ourselves, a stalled WPE would mean forking only its slice, and
  Servo stays the real hedge. "ok, agree."
- Security releases (2026-11-28): `tools/lab/wpewebkit-update.sh`
  (from Control): finds the newest stable WPE release on wpewebkit.org
  (even minor numbers only), takes the newest of our series and only
  reports a new series unless `--series` (a new series may change WPE's
  class layouts or the patch), takes upstream's sha256 from its .sums
  file, updates the template, builds on techbench and adds the package to
  techbench's and Hubble's local repositories, then prints the install and
  display-suite commands. `--check` only reports. Tried: `--check` (2.54.3
  current), and with the template set back to 2.54.1 it found 2.54.3; the
  build half is the sequence run by hand tonight, not yet run by the
  script itself (it will be at the next release).
