# Web milestone 2: web views inside other programs' windows

*Drafted 2026-11-28 on Control, after web milestone 1 (pages in a window of
their own, vergence-web's own WPE platform, GPU frames; Dave: "lag is gone")
and the browser decisions of the same night (docs/web-milestone-1.md, "The
browser's form and the component's shape"). The questions below wait for
Dave's answers; nothing is built.*

**Decided already (Dave, 2026-11-27 and 11-28):**
- vergence-web is a **service process**: any program places a **web view**
  in its own window over an **engine-neutral protocol**; WebKit is never in
  the program. Its protocol takes a **window of versions** from its first
  release (packaging milestone 2, question 4).
- The **browser comes first** among its users; Mail's message view and
  Preview's HTML after.
- The browser's look: server-drawn tabs, one toolbar the browser draws, a
  hovered link's address in an overlay (View > Status Bar optional), and
  **the page scrolling up under the tabs and toolbar, frosted**: the server
  frosts a glass band, WebKit lays the page out below it (the obscured
  content insets; the inset spike passed, the patch is in our package).

## What is missing

Milestone 1's page is a window of its own: vergence-web is its own client
and its frames are its window's picture. To be *inside* a program's window,
three things are new:

1. **A region of one client's window filled by another** (the server's
   part): today a window's picture comes from its own client only.
2. **The protocol between the program and vergence-web** (open a view,
   load, back, forward, title, address, progress, ...), and vergence-web as
   a service serving many views for many programs.
3. **The toolkit's part:** a `web_view` element in vergence-ui that keeps a
   region in the window's layout and speaks that protocol, so a program
   built on the toolkit writes `web_view(&handle)` and nothing else.

## The plan

1. **Embedded regions in the native protocol** (bus version step): a
   window asks the server for a region (`Embed { window, region, rect }`,
   in points, moved and resized with the layout); the server answers with a
   one-time token. Whoever presents the token (`EmbedJoin { token }`) may
   attach pictures to that region (the same Attach, GPU or shared memory)
   and is told the region's size and scale. The server composites the
   region inside the window: clipped to it, under the window's glass band,
   frosted there with the rest. Pointer, wheel and touch over the region go
   to the region's client; the keyboard goes to it while the region has
   focus (a click in it gives it; the program's own widgets take it back).
   The program can hide, move or remove the region, and closing its window
   ends the region. **The glass band** (`Glass { window, height }`): the
   server frosts that strip over whatever the window shows, its own
   drawing and any region alike.
2. **vergence-web as a service:** one process per session, listening on an
   owner-only socket in the runtime directory (as Edit's and FileView's
   single-instance sockets), started by the first program that asks for a
   view (question 3). Each view: a WebKitWebView on our platform, its
   frames attached to the program's region through the token, its input
   from the server.
3. **The web protocol, version 1** (engine-neutral; a window of versions
   from the start): the program asks `Open { token, policy }` (policy:
   scripts, network, Protection, profile), `Load`, `LoadHtml`, `Back`,
   `Forward`, `Reload`, `Stop`, `Zoom`, `Find`, `TopInset`, `Close`; it is
   told `Title`, `Address`, `Progress`, `Security`, `Icon`, `HoveredLink`,
   `CanGoBack/Forward`, `NewViewRequested`, `ContextMenu`, `SelectList`,
   `Download`, `PermissionRequested`, `Crashed`. Encoded as the bus's own
   messages are (vergence-protocol's encoder), round-trip and fuzz tested.
4. **The toolkit's `web_view` element** (vergence-ui, vergence-widgets):
   keeps the region in step with the layout, passes the token, turns the
   protocol's events into the program's messages, draws context menus and
   select lists as the toolkit's own menus (question 5).
5. **A browser skeleton** (question 6): `vergence-browser`, a window with
   the server's tabs (each tab a web view), the toolbar (back, forward,
   reload, the address field, the shield placeholder, the menu), the
   hovered link's overlay, the glass band over tabs and toolbar with the
   page's top inset under it. No bookmarks, history or Protection yet
   (milestone 4).
6. **Checks:** protocol round-trip and fuzz tests for the new bus messages
   and the web protocol; a display-suite scenario: a program with a web view
   region (the page's blocks inside its window at the right place, the
   program's own widgets around it), a click and typing inside the region,
   focus back to the program's field, the band frosted over the page
   scrolled under it; vergence-web restarted after a crash with the view
   showing so.

## Questions

1. **Who composites the page into the program's window:** (a) the server
   (the region above: the program never holds the page's pixels, the
   server can frost them and mark the region as web content, input goes
   straight to vergence-web); (b) the program (vergence-web hands it the
   frames; it draws them into its own picture and forwards input).
   *Lean: (a); it is what makes the frosted band and the server's own mark
   of the true site possible, and a page's pixels stay out of every
   program.*
2. **The web protocol's road:** (a) vergence-web's own socket (owner-only,
   runtime directory), the server's token tying a view to the program's
   region; (b) carried through the server's bus. *Lean: (a), as the
   single-instance apps do; the server only needs to know about regions.*
3. **vergence-web's life:** (a) started by the first program that wants a
   view, gone a short while after the last view closes; (b) started with
   the session. *Lean: (a); nobody pays for WebKit until a page is shown.*
4. **One process or one per program:** (a) one vergence-web per session,
   pages in WebKit's own web processes (one per site, as WebKit does);
   (b) one per program. *Lean: (a); WebKit already separates pages.*
5. **Context menus and select lists:** (a) drawn by the program's toolkit
   as its own menus (vergence-web sends their items, the program answers);
   (b) drawn by the server. *Lean: (a); they then look and behave as every
   other menu in the program.*
6. **The test program:** (a) the browser skeleton above (the browser comes
   first anyway); (b) a small test program with a web view beside a few
   widgets. *Lean: (a), with the display suite using a small test program
   as well.*
7. **The browser's name** in the launcher and its window: (a) "Web";
   (b) "Browser"; (c) a name of its own. *Lean: (a), as GNOME's; plain
   names are Vergence's way (FileView aside).*
