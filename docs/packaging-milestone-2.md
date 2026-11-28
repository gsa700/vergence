# Packaging milestone 2: apps updated one at a time

*Drafted 2026-11-28 on Control from Dave's word that night: "you shouldn't
have to download 500 MB just to get the latest calculator or Edit
version", then "likewise, vergence-web will update more often than
AlbumWall". The questions below wait for his answers; nothing is built.
For after the browser's first milestones, or before going public,
whichever comes first.*

## Why

Today Vergence is one package (packaging milestone 1): the server, vdm,
the session and every app. A fix to the calculator ships all of it.
Measured on techbench, 2026-11-28 (0.4.2pre20):

| | Size |
|---|---|
| The `vergence` package, downloaded | 27 MB |
| Installed | 106 MB |
| vergence-calc / vergence-edit / vergence-albumwall | 3.1 / 3.9 / 5.0 MB (about a third of that compressed) |
| vergence-server | 18 MB |

The gap grows with every app, and some parts keep their own pace:
vergence-web follows WebKit's security releases (about monthly) and the
browser's work, AlbumWall changes when it changes. WebKit (wpewebkit,
117 MB installed) is already its own package; only people who use the web
should need it.

## What stands in the way

**"One bus protocol version"** (CONTRIBUTING.md): the bus version changes
with any change to the wire, and the server refuses any other. Every app
must be built for exactly the running server's version (v48 tonight; it
moved several times in a day). Apps shipped apart from the server would
stop working at the first bus change they missed.

## The plan

1. **A window of bus versions:** clients say their version at hello (as
   now); the server accepts any from the oldest it still supports up to its
   own. Inside the window changes are additive only: new messages, new
   fields at the end, nothing changed in meaning, an older client never
   sent what it cannot read. A change that cannot be additive raises the
   floor: that release ships with every app rebuilt. The round-trip and
   fuzz tests gain an older client against a newer server.
2. **Packages:**
   - `vergence`: the server, vdm, the session, Settings, the shell's tools
     (vctl, vbus, vtile, vagent), fonts, icons, the words list, the syntax
     files, the login screen: what moves together.
   - One template per app, each with its own version: `vergence-calc`,
     `vergence-edit`, `vergence-files`, `vergence-reader`, `vergence-closeup`,
     `vergence-term`, `vergence-hw`, `vergence-disks`, `vergence-quit`,
     `vergence-albumwall`, `vergence-remote-viewer` (question 3).
   - `vergence-web`: the web component (the service, later the browser),
     depending on `wpewebkit`; its own app-facing protocol versioned with a
     window from its first release (question 4).
   - Each depends on `vergence>=` the core release whose bus floor it needs.
3. **A meta package** `vergence-apps` depending on the standard set, so one
   install still gives the whole desktop.
4. **The release tooling:** a script that builds only the packages whose
   sources changed since their last release (by crate dependency), bumps
   their versions, and checks each app's bus version against the core's
   window.

## Questions

1. **The bus rule:** (a) a window (oldest supported to current, additive
   inside it); (b) keep one exact version and ship everything together.
   *Lean: (a); per-app updates need it, and it also lets a newer app run on
   an older core where nothing it uses changed.*
2. **How wide the window is:** (a) until a breaking change, however long;
   (b) a fixed number of releases. *Lean: (a), with breaking changes rare
   and announced.*
3. **Which apps stand apart:** (a) every app its own package; (b) only the
   big or fast-moving ones (vergence-web, AlbumWall, Reader), the rest in
   the core. *Lean: (a); the per-app packages are small and the tooling
   makes them cheap.*
4. **vergence-web's protocol** (the app-facing one of the web plan):
   versioned with a window from day one. *Lean: yes; Mail, Preview and the
   browser will all speak it, perhaps updated at different times.*
5. **Where Settings and Terminal go:** core or apart. *Lean: Settings in
   the core (it changes with the server's settings); Terminal apart.*
