# Web milestone 1: pages in a Vergence window

*Drafted 2026-11-27 on Control from techbench's plan of 2026-11-26 (shared
memory 01M8DFXCH2CA9BDMAQCYQHJZJJ, the web plan; 01M8DGMVR6FB30Y6WWW2S9KZAQ,
the browser's Protection design) and the roadmap's "The web component"
(docs/apps-roadmap.md). Dave: "webkit in a buildable component model. So we
can include it with any of our apps and others could too." The questions
below wait for his answers; nothing is built.*

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

**Research before building:** whether WPE Platform offers a direct way to
read a page's accessibility tree (milestone 3), or only AT-SPI; how a
platform of our own registers (a loadable module, or set in code).
