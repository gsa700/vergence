# AlbumWall milestone 1: the wall keeps its place when music starts and when
# it ends (Dave, 2026-11-26: an album started, the wall went to the top; one
# ended, "the wall scrolled itself to the F section"). The play bar coming and
# going had changed the window's tree, so the wall was a new scroll each time.
# A library of 300 albums made here (Hymie's tone16.flac with tags written
# in, its own picture taken out, two tracks each); the keys page halfway
# down, an album opened, its last track played (Stop when it ends): the
# album at the top of the wall the same while it plays and after. Then the
# album in the row under the open panel takes the panel over in place,
# pointing down at it, with nothing above moving.
scenario() {
    music=$OUT/music
    rm -rf "$music"
    python3 - "$ROOT/crates/vergence-hymie/fixtures/tone16.flac" "$music" <<'PY' || { check 0 "the test library"; return; }
import os, struct, sys
fixture, out = sys.argv[1], sys.argv[2]
d = open(fixture, "rb").read()
blocks, at = [], 4
while True:
    h = d[at]; n = int.from_bytes(d[at+1:at+4], "big")
    blocks.append((h & 0x7F, d[at+4:at+4+n])); at += 4 + n
    if h & 0x80: break
frames = d[at:]
def flac(tags):
    v = b"suite"
    body = struct.pack("<I", len(v)) + v + struct.pack("<I", len(tags))
    for k, val in tags:
        e = f"{k}={val}".encode(); body += struct.pack("<I", len(e)) + e
    keep = [(t, b) for t, b in blocks if t not in (1, 4, 6)]
    keep.insert(1, (4, body))
    o = b"fLaC"
    for i, (t, b) in enumerate(keep):
        o += bytes([t | (0x80 if i == len(keep) - 1 else 0)]) + len(b).to_bytes(3, "big") + b
    return o + frames
for a in range(300):
    artist, album = f"Artist {a // 4:03d}", f"Album {a:03d}"
    folder = os.path.join(out, artist, f"{1960 + a % 60} - {album}")
    os.makedirs(folder)
    for n in (1, 2):
        open(os.path.join(folder, f"{n:02d}.flac"), "wb").write(flac([
            ("ALBUMARTIST", artist), ("ARTIST", artist), ("ALBUM", album), ("TITLE", f"Track {n}"),
            ("TRACKNUMBER", n)]))
PY
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    cfg=$home/.config/vergence/desktop.toml
    mkdir -p "$home/.config/vergence/albumwall"
    printf '# AlbumWall for Vergence: its libraries\ncurrent\t0\nfolder\tScroll\t%s\n' "$music" \
        > "$home/.config/vergence/albumwall/libraries"
    rm -f "$home/.config/vergence/albumwall/session"
    # No [albumwall] table: when an album ends, it stops (the default).
    kept=$(cat "$cfg" 2>/dev/null)
    awk '/^\[/ { skip = ($0 == "[albumwall]") } !skip' "$cfg" > "$cfg.new" 2>/dev/null && mv "$cfg.new" "$cfg"
    vctl spawn "env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    w=$(win_index AlbumWall)
    i=0; while ! vctl tree "$w" | grep -q '300 albums · 600 tracks'; do sleep 0.5; i=$((i + 1)); [ $i -gt 60 ] && break; done
    check "$(yes_if "vctl tree $w | grep -q '300 albums · 600 tracks'")" "the library read: 300 albums"
    # The album named first in the tree: the top of the wall as built.
    top() { vctl tree "$w" | grep -oE 'button "Album [0-9]+, Artist [0-9]+"' | head -1; }
    vctl focus "$w" >/dev/null; sleep 0.3
    n=0; while [ $n -lt 8 ]; do vctl key Page_Down >/dev/null; sleep 0.25; n=$((n + 1)); done
    vctl key Return >/dev/null; sleep 1.5
    # The open panel centered in the wall (the Avalonia app's TargetOffset;
    # Dave, 2026-11-27): its sleeve's middle near the wall's, not half a row
    # lower as when the album's row and its panel were centered together.
    t=$(vctl tree "$w")
    set -- $(echo "$t" | grep -E 'button "Play Album [0-9]+"' | head -1 | sed -nE 's/.* at -?[0-9]+,(-?[0-9]+) [0-9]+x([0-9]+).*/\1 \2/p') 0 0
    sleeve_mid=$(($1 + $2 / 2))
    wall_h=$(echo "$t" | grep -E 'scroll area "wall"' | sed -nE 's/.* [0-9]+x([0-9]+).*/\1/p' | head -1)
    echo "  .. the open panel's sleeve middle at y $sleeve_mid, the wall's at $((${wall_h:-0} / 2))" >&2
    check "$(yes_if "near $sleeve_mid $((${wall_h:-0} / 2)) 60")" "an opened album's panel centered in the wall"
    shot MEM-1 opened >/dev/null
    before=$(top)
    echo "  .. the top with an album open halfway down: $before" >&2
    check "$(yes_if "[ -n '$before' ] && [ '$before' != 'button \"Album 000, Artist 000\"' ]")" "paged halfway down the wall"
    # Into its tracks, the last one played: 1.5 s, then it stops.
    vctl key Down >/dev/null; sleep 0.2; vctl key Down >/dev/null; sleep 0.2
    vctl key Return >/dev/null; sleep 0.6
    playing=$(top)
    echo "  .. playing: $playing" >&2
    check "$(yes_if "vctl tree $w | grep -q 'button \"Pause\"'")" "its last track playing"
    check "$(yes_if "[ '$playing' = '$before' ]")" "music starts: the wall keeps its place"
    i=0; while vctl tree "$w" | grep -q 'button "Pause"'; do sleep 0.3; i=$((i + 1)); [ $i -gt 20 ] && break; done
    sleep 0.6
    ended=$(top)
    echo "  .. ended: $ended" >&2
    check "$(yes_if "! vctl tree $w | grep -q 'button \"Pause\"'")" "the album ends: the play bar goes"
    check "$(yes_if "[ '$ended' = '$before' ]")" "music ends: the wall keeps its place"
    # The album in the row just under the open panel takes the panel over
    # where it stands, pointing down at it (WallRows.PointsDown): nothing
    # above it moves.
    t=$(vctl tree "$w")
    open=$(echo "$t" | grep -oE 'button "Play Album [0-9]+"' | head -1 | sed -E 's/button "Play (Album [0-9]+)"/\1/')
    y_of() { echo "$t" | grep -F "button \"$1" | sed -nE 's/.* at -?[0-9]+,(-?[0-9]+) .*/\1/p' | head -1; }
    oy=$(y_of "$open, ")
    # The first tile of the next row down: the smallest y below the open album's.
    below=$(echo "$t" | grep -E 'button "Album [0-9]+, Artist [0-9]+"' | sed -nE 's/.*button "(Album [0-9]+), Artist [0-9]+" at -?[0-9]+,(-?[0-9]+) .*/\2 \1/p' |
        awk -v oy="${oy:-0}" '$1 > oy' | sort -n | head -1 | cut -d' ' -f2-)
    echo "  .. open: $open at y ${oy:-?}; the row under the panel starts with: ${below:-?}" >&2
    check "$(yes_if "[ -n '$open' ] && [ -n '$below' ]")" "an open album, and the row under its panel"
    top_before=$(top)
    vctl act "$w" "$(vctl tree "$w" | grep -F "button \"$below, " | grep -oE '#[0-9]+' | head -1 | tr -d '#')" press >/dev/null; sleep 1.2
    t=$(vctl tree "$w")
    check "$(yes_if "echo \"\$t\" | grep -qF 'button \"Play $below\"'")" "the album under the panel took it over"
    py=$(echo "$t" | grep -F "button \"Play $below\"" | sed -nE 's/.* at -?[0-9]+,(-?[0-9]+) .*/\1/p' | head -1)
    ny=$(y_of "$below, ")
    echo "  .. its sleeve at y ${py:-?}, its tile at y ${ny:-?}; top ${top_before} then $(top)" >&2
    check "$(yes_if "[ -n '$py' ] && [ -n '$ny' ] && [ '$py' -lt '$ny' ]")" "the panel stays above it, pointing down"
    shot MEM-1 pointing-down >/dev/null
    check "$(yes_if "[ \"\$(top)\" = '$top_before' ]")" "nothing above the panel moved"
    printf '%s\n' "$kept" > "$cfg"
}
