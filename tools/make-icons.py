#!/usr/bin/env python3
"""Vergence's icons (direction D, "Vergence", chosen by Dave 2026-11-18):
C's even tiles colored by kind, lit from the top left with a fine rim as in
A, carrying B's friendly objects drawn with A's crisp edges and bright
touches. Files and places share the folder blue and the same light.

    python3 tools/make-icons.py        # writes data/icons/

Writes data/icons/<app>.svg (the apps' own, installed in hicolor) and
data/icons/places/*.svg, data/icons/kinds/*.svg (Vergence's apps draw them:
installed in share/vergence/icons). Every file is self-contained (its own
gradients), 64x64. The design canvas these came from:
https://claude.ai/artifact/KE2ECDjA9NM5YR2yCEYyQ3 (board D).
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "data", "icons")

INK = "#1a1d24"
LINE = "#9aa3b5"
AMBER, RED, BLUE, GREEN, TEAL, VIOLET = "#f0b43c", "#e0574a", "#2560c4", "#4caf72", "#14a39a", "#8a63d2"

# Tile colors by kind of app: (top, bottom, rim).
TILES = {
    "slate": ("#5d6b86", "#3b465c", "rgba(10,14,24,0.45)"),
    "blue": ("#4a8cec", "#2560c4", "rgba(10,30,70,0.45)"),
    "graphite": ("#454952", "#25272d", "rgba(0,0,0,0.55)"),
    "steel": ("#6c7280", "#474c57", "rgba(0,0,0,0.45)"),
    "teal": ("#1a9e94", "#0b6b64", "rgba(0,30,28,0.5)"),
    "red": ("#e0574a", "#a8322a", "rgba(60,0,0,0.5)"),
}

DEFS = f"""<defs>
<linearGradient id="paper" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#e3e6ec"/></linearGradient>
<linearGradient id="folderBack" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#5b97ee"/><stop offset="1" stop-color="#3b77d6"/></linearGradient>
<linearGradient id="folderFront" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#8fbaf6"/><stop offset="1" stop-color="#5b97ee"/></linearGradient>
<linearGradient id="metal" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#c9ced8"/><stop offset="0.45" stop-color="#f2f4f7"/><stop offset="1" stop-color="#a9b0bd"/></linearGradient>
</defs>"""


def svg(body, comment, tile=None):
    defs = DEFS
    if tile:
        top, bottom, _ = TILES[tile]
        defs = defs.replace("</defs>", f'<linearGradient id="tile" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient>\n</defs>')
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">\n<!-- {comment} Drawn by the project (tools/make-icons.py). -->\n{defs}\n{body}\n</svg>\n'


def tile(kind):
    rim = TILES[kind][2]
    return (f'<rect x="2" y="2" width="60" height="60" rx="14" fill="url(#tile)"/>'
            f'<rect x="2.5" y="2.5" width="59" height="59" rx="13.5" fill="none" stroke="rgba(255,255,255,0.28)"/>'
            f'<rect x="2" y="2" width="60" height="60" rx="14" fill="none" stroke="{rim}"/>')


# ---- the apps ----
# The apps' own icons are not drawn here: Dave preferred the hand-drawn ones
# from before direction D (2026-11-18, side by side), so data/icons/
# vergence-<app>.svg are kept as they are. Direction D's app tiles are in
# git (ee1af80) if they are wanted again.

# ---- places: the folder, with a mark pressed into it ----

FOLDER = ('<path d="M6 15 Q6 11 10 11 L23 11 L28 16 L54 16 Q58 16 58 20 L58 50 Q58 54 54 54 L10 54 Q6 54 6 50 Z" fill="url(#folderBack)" stroke="#123f8a" stroke-width="1.2"/>'
          '<path d="M6 25 Q6 22 9 22 L55 22 Q58 22 58 25 L58 50 Q58 54 54 54 L10 54 Q6 54 6 50 Z" fill="url(#folderFront)" stroke="#123f8a" stroke-width="1.2"/>'
          '<line x1="9" y1="23.5" x2="55" y2="23.5" stroke="rgba(255,255,255,0.7)"/>')


def pressed(mark):
    return FOLDER + f'<g fill="{BLUE}" stroke="none" opacity="0.85">{mark}</g>'


CAN = ('<path d="M15 18 L49 18 L46 56 Q45.7 59 42.7 59 L21.3 59 Q18.3 59 18 56 Z" fill="url(#metal)" stroke="#2a303c" stroke-width="1.2"/>'
       '<rect x="11" y="11" width="42" height="7" rx="3" fill="#dfe3ea" stroke="#2a303c" stroke-width="1.2"/>'
       '<rect x="26" y="6" width="12" height="5" rx="2" fill="#dfe3ea" stroke="#2a303c" stroke-width="1.2"/>'
       '<g stroke="#8a919e" stroke-width="2" stroke-linecap="round"><line x1="25" y1="25" x2="26" y2="52"/><line x1="32" y1="25" x2="32" y2="52"/><line x1="39" y1="25" x2="38" y2="52"/></g>')

# Full: heaped, the lid askew, and a few on the floor (Dave: "We aren't that neat").
CAN_FULL = ('<path d="M17 14 L27 7 L33 14 Z" fill="#ffffff" stroke="#2a303c" stroke-width="1"/>'
            '<path d="M30 13 L43 7 L42 15 Z" fill="#fff4c9" stroke="#2a303c" stroke-width="1"/>'
            '<circle cx="25" cy="12" r="4" fill="#f4f6f9" stroke="#2a303c" stroke-width="1"/>'
            '<path d="M15 19 L45 19 L42.5 54 Q42.2 57 39.2 57 L20.8 57 Q17.8 57 17.5 54 Z" fill="url(#metal)" stroke="#2a303c" stroke-width="1.2"/>'
            '<rect x="11" y="15" width="38" height="7" rx="3" fill="#dfe3ea" stroke="#2a303c" stroke-width="1.2" transform="rotate(-8 30 18)"/>'
            '<g stroke="#8a919e" stroke-width="2" stroke-linecap="round"><line x1="24" y1="27" x2="25" y2="51"/><line x1="30" y1="27" x2="30" y2="51"/><line x1="36" y1="27" x2="35" y2="51"/></g>'
            '<path d="M46 55 Q49 50 54 52 Q58 54 56 58 Q52 61 47 59 Q44 58 46 55 Z" fill="#f4f6f9" stroke="#2a303c" stroke-width="1"/>'
            '<path d="M48 55 Q51 54 54 56" fill="none" stroke="#9aa3b5" stroke-width="0.8"/>'
            '<path d="M4 57 L13 53 L15 60 L6 61 Z" fill="#fff4c9" stroke="#2a303c" stroke-width="1"/>'
            '<line x1="7" y1="57.5" x2="12" y2="55.5" stroke="#c9b98a" stroke-width="0.8"/>')

DISK = ('<rect x="6" y="20" width="52" height="26" rx="5" fill="url(#metal)" stroke="#2a303c" stroke-width="1.2"/>'
        '<rect x="6" y="38" width="52" height="8" rx="3" fill="#a9b0bd" stroke="#2a303c" stroke-width="1.2"/>'
        f'<circle cx="50" cy="42" r="2" fill="{GREEN}"/><g stroke="#8a919e" stroke-width="1.5"><line x1="12" y1="27" x2="34" y2="27"/><line x1="12" y1="31" x2="28" y2="31"/></g>')

USB = ('<rect x="24" y="6" width="16" height="12" rx="1.5" fill="#dfe3ea" stroke="#2a303c" stroke-width="1.2"/>'
       '<g fill="#2a303c"><rect x="28" y="10" width="3" height="3"/><rect x="33" y="10" width="3" height="3"/></g>'
       '<rect x="18" y="18" width="28" height="40" rx="5" fill="#3b465c" stroke="#1a1d24" stroke-width="1.2"/>'
       '<rect x="18.5" y="18.5" width="27" height="39" rx="4.5" fill="none" stroke="rgba(255,255,255,0.25)"/>'
       f'<rect x="24" y="30" width="16" height="10" rx="2" fill="{AMBER}" stroke="#1a1d24" stroke-width="1"/>')

PLACES = {
    "home": ("Home: the folder with a house, its door open.", pressed('<path fill-rule="evenodd" d="M20 39 L32 28.5 L44 39 L40 39 L40 48 L24 48 L24 39 Z M29.5 48 L29.5 41.5 L34.5 41.5 L34.5 48 Z"/>')),
    "desktop": ("Desktop: the folder with a screen.", pressed('<rect x="22" y="30" width="20" height="13" rx="1.5"/><rect x="28" y="44" width="8" height="3"/>')),
    "downloads": ("Downloads: the folder with an arrow down.", pressed('<path d="M29 29 L35 29 L35 37 L40 37 L32 46 L24 37 L29 37 Z"/>')),
    "documents": ("Documents: the folder with a page.", pressed('<path d="M26 29 L34 29 L38 33 L38 47 L26 47 Z"/>')),
    "pictures": ("Pictures: the folder with a hill and a sun.", pressed('<path d="M22 46 L29 36 L34 41 L37 38 L42 46 Z"/><circle cx="37" cy="32" r="2.5"/>')),
    "music": ("Music: the folder with a note.", pressed('<path d="M35 29 L35 41 Q35 45 31 45 Q27 45 27 42 Q27 39 31 39 Q32 39 33 39.4 L33 31 L39 30 L39 33 Z"/>')),
    "videos": ("Videos: the folder with a play mark.", pressed('<path d="M28 30 L40 38 L28 46 Z"/>')),
    "folder": ("A folder.", FOLDER),
    "recent": ("Recent: the folder with a clock.", FOLDER + f'<g fill="none" stroke="{BLUE}" stroke-width="2.2" stroke-linecap="round" opacity="0.85"><circle cx="32" cy="38" r="9"/><path d="M32 32.5 L32 38 L36 40.5"/></g>'),
    "network": ("Network: the folder with a globe (a server's folder).", FOLDER + f'<g fill="none" stroke="{BLUE}" stroke-width="2" opacity="0.85"><circle cx="32" cy="38" r="9"/><ellipse cx="32" cy="38" rx="3.6" ry="9"/><line x1="23" y1="38" x2="41" y2="38"/><path d="M24.5 33.5 Q32 31.5 39.5 33.5 M24.5 42.5 Q32 44.5 39.5 42.5"/></g>'),
    "trash": ("The Trash, empty.", CAN),
    "trash-full": ("The Trash, full, a few dropped beside it.", CAN_FULL),
    "disk": ("A disk inside the machine.", DISK),
    "usb": ("A removable drive.", USB),
}

# ---- kinds of files: the page, with what it holds ----

PAGE = ('<path d="M17 4 L40 4 L51 15 L51 57 Q51 60 48 60 L17 60 Q14 60 14 57 L14 7 Q14 4 17 4 Z" fill="url(#paper)" stroke="#2a303c" stroke-width="1.2"/>'
        '<path d="M40 4 L40 13 Q40 15 42 15 L51 15" fill="#e3e6ec" stroke="#2a303c" stroke-width="1.2"/>')

KINDS = {
    "text": ("Plain text: lines on a page.", PAGE + f'<g stroke="{LINE}" stroke-width="2" stroke-linecap="round"><line x1="20" y1="25" x2="45" y2="25"/><line x1="20" y1="31" x2="41" y2="31"/><line x1="20" y1="37" x2="44" y2="37"/><line x1="20" y1="43" x2="35" y2="43"/></g>'),
    "document": ("A document: a heading over lines.", PAGE + f'<rect x="20" y="22" width="18" height="4" rx="1" fill="{BLUE}"/><g stroke="{LINE}" stroke-width="2" stroke-linecap="round"><line x1="20" y1="33" x2="45" y2="33"/><line x1="20" y1="39" x2="42" y2="39"/><line x1="20" y1="45" x2="44" y2="45"/><line x1="20" y1="51" x2="33" y2="51"/></g>'),
    "picture": ("A picture.", PAGE + f'<rect x="19" y="23" width="27" height="22" rx="2" fill="#cfe6ff" stroke="#2a303c" stroke-width="1"/><path d="M19 45 L27 34 L33 40 L37 36 L46 45 Z" fill="{GREEN}"/><circle cx="39" cy="29" r="2.5" fill="{AMBER}"/>'),
    "sound": ("Sound: a note.", PAGE + f'<path d="M36 23 L36 41 Q36 46 31 46 Q26 46 26 42.5 Q26 39 31 39 Q32.5 39 33.5 39.5 L33.5 26 L43 24 L43 28 Z" fill="{VIOLET}" stroke="#2a303c" stroke-width="1"/>'),
    "video": ("Video: a frame and a play mark.", PAGE + '<rect x="18" y="24" width="29" height="22" rx="3" fill="#1a1d24"/><g fill="#5c6270"><rect x="20" y="26" width="3" height="3"/><rect x="20" y="41" width="3" height="3"/><rect x="42" y="26" width="3" height="3"/><rect x="42" y="41" width="3" height="3"/></g><path d="M29 29 L37 35 L29 41 Z" fill="#ffffff"/>'),
    "archive": ("An archive: a zipped band.", PAGE + f'<rect x="28" y="4" width="9" height="56" fill="{AMBER}" opacity="0.9"/><g fill="#2a303c">' + "".join(f'<rect x="{29 if i % 2 else 33}" y="{8 + i * 4}" width="3" height="2"/>' for i in range(8)) + '</g><rect x="28.5" y="40" width="8" height="10" rx="1.5" fill="#dfe3ea" stroke="#2a303c" stroke-width="1"/>'),
    "pdf": ("A PDF: a page with a red band.", PAGE + f'<rect x="14" y="40" width="37" height="12" fill="{RED}"/><g fill="none" stroke="#ffffff" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M22 49.5 V42.5 H25 Q27 42.5 27 44.5 Q27 46.5 25 46.5 H22"/><path d="M30 42.5 V49.5 H32 Q35 49.5 35 46 Q35 42.5 32 42.5 Z"/><path d="M38 49.5 V42.5 H42.5 M38 46 H41.5"/></g><g stroke="{LINE}" stroke-width="2" stroke-linecap="round"><line x1="20" y1="22" x2="45" y2="22"/><line x1="20" y1="28" x2="40" y2="28"/></g>'),
    "program": ("A program: a small window with a prompt.", '<rect x="8" y="10" width="48" height="44" rx="6" fill="#25272d" stroke="#1a1d24" stroke-width="1.2"/><rect x="8.5" y="10.5" width="47" height="9" rx="5.5" fill="#454952"/><g fill="#8a919e"><circle cx="15" cy="15" r="1.8"/><circle cx="21" cy="15" r="1.8"/></g><path d="M16 29 L23 35 L16 41" fill="none" stroke="#6fe08a" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/><line x1="26" y1="42" x2="36" y2="42" stroke="#6fe08a" stroke-width="2.6" stroke-linecap="round"/>'),
    "other": ("A file of no known kind: a blank page.", PAGE),
}


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def main():
    for name, (comment, body) in PLACES.items():
        write(os.path.join(OUT, "places", f"{name}.svg"), svg(body, comment))
    # The Trash window's own icon, for the dock (Dave, 2026-11-18): the can
    # itself, no tile, as the desktop's Trash.
    write(os.path.join(OUT, "vergence-trash.svg"), svg(CAN, "The Trash: its window's icon, the can itself."))
    for name, (comment, body) in KINDS.items():
        write(os.path.join(OUT, "kinds", f"{name}.svg"), svg(body, comment))
    print(f"the Trash window, {len(PLACES)} places, {len(KINDS)} kinds in {os.path.normpath(OUT)}")


if __name__ == "__main__":
    main()
