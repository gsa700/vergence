# AlbumWall milestone 1, step 9 (docs/albumwall-milestone-1.md): AlbumWall
# for Vergence on a small library made here (five albums of Hymie's
# tone16.flac with tags written in, colored covers beside four of them, one
# under 300 pixels, one album with none; the fixture's own embedded picture
# taken out). The wall: the counts, the covers
# drawn. An album unfolds: its sleeve (the play button) and its tracks.
# Playing, on a paced, silent output: the play bar, the position moving.
# With "next album" chosen (in desktop.toml, as Settings writes it) the
# album ends, the next plays and the wall opens it; the play mode's button
# then goes on to "random album" and writes that. Search:
# "cobalt", art:missing, art:small. Settings (Ctrl+,): the Library page's
# numbers, a check box written to desktop.toml and the play bar following.
scenario() {
    music=$OUT/music
    rm -rf "$music"
    python3 - "$ROOT/crates/vergence-hymie/fixtures/tone16.flac" "$music" <<'PY' || { check 0 "the test library"; return; }
import os, struct, sys, zlib
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
def png(path, side, rgb):
    row = b"\0" + bytes(rgb) * side
    def chunk(k, b): return struct.pack(">I", len(b)) + k + b + struct.pack(">I", zlib.crc32(k + b) & 0xffffffff)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(row * side)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)
albums = [("Aqua Group", "Blue Lagoon", 1990, 600, (40, 90, 200)),
          ("Aqua Group", "Deep Water", 1992, 600, (20, 120, 160)),
          ("Cobalt", "Night Sky", 1995, 600, (30, 40, 120)),
          ("Cobalt", "Red Desert", 1997, 200, (190, 60, 40)),
          ("The Plain", "No Cover", 2000, 0, None)]
for artist, album, year, side, rgb in albums:
    folder = os.path.join(out, artist, f"{year} - {album}")
    os.makedirs(folder)
    for n, title in enumerate(["One", "Two", "Three"], 1):
        open(os.path.join(folder, f"{n:02d} {title}.flac"), "wb").write(flac([
            ("ALBUMARTIST", artist), ("ARTIST", artist), ("ALBUM", album), ("TITLE", title),
            ("TRACKNUMBER", n), ("DATE", year)]))
    if rgb:
        png(os.path.join(folder, "cover.png"), side, rgb)
PY
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    cfg=$home/.config/vergence/desktop.toml
    mkdir -p "$home/.config/vergence/albumwall"
    printf '# AlbumWall for Vergence: its libraries\ncurrent\t0\nfolder\tSuite Music\t%s\n' "$music" \
        > "$home/.config/vergence/albumwall/libraries"
    rm -f "$home/.config/vergence/albumwall/session"
    # When an album ends: the next (the tracks are 1.5 s, so it is set before
    # anything plays). The profile's desktop.toml is kept between runs: an
    # [albumwall] table left in it is taken out first (a second one would make
    # the whole file invalid TOML), and the file is put back at the end.
    kept=$(cat "$cfg" 2>/dev/null)
    awk '/^\[/ { skip = ($0 == "[albumwall]") } !skip' "$cfg" > "$cfg.new" 2>/dev/null && mv "$cfg.new" "$cfg"
    printf '\n[albumwall]\nafter-album = "next"\n' >> "$cfg"
    vctl spawn "env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    w=$(win_index AlbumWall)
    # The library read: its counts in the bottom bar.
    i=0; while ! vctl tree "$w" | grep -q '5 albums · 15 tracks'; do sleep 0.5; i=$((i + 1)); [ $i -gt 40 ] && break; done
    check "$(yes_if "vctl tree $w | grep -q '5 albums · 15 tracks'")" "the library read: 5 albums · 15 tracks"
    sleep 2
    pic=$(picture AlbumWall wall)
    n=$("$HARNESS" colors "$pic" | awk '{print $2}')
    echo "  .. the wall: ${n:-0} colored pixels"
    # Three blue covers of about 185 points and a red one: far more color
    # than the dark wall's text and bars alone.
    check "$(yes_if "[ \"${n:-0}\" -gt 60000 ]")" "the covers drawn on the wall"
    # node <window> <text>: the first meaning node whose line holds the text.
    node() { vctl tree "$1" | grep -F "$2" | grep -oE '#[0-9]+' | head -1 | tr -d '#'; }
    vctl act "$w" "$(node "$w" 'button "Deep Water, Aqua Group"')" press >/dev/null; sleep 1.5
    check "$(yes_if "vctl tree $w | grep -q 'button \"Play Deep Water\"'")" "an album unfolds: its sleeve is the play button"
    check "$(yes_if "vctl tree $w | grep -q 'Three'")" "its tracks listed"
    shot MEM-1 open >/dev/null
    vctl act "$w" "$(node "$w" 'button "Play Deep Water"')" press >/dev/null; sleep 1
    check "$(yes_if "vctl tree $w | grep -q 'button \"Pause\"'")" "playing: the play bar, with Pause"
    a=$(vctl tree "$w" | grep -F 'slider "Position"' | sed -nE 's/.*= ?"?([0-9.]+).*/\1/p' | head -1)
    sleep 0.6
    b=$(vctl tree "$w" | grep -F 'slider "Position"' | sed -nE 's/.*= ?"?([0-9.]+).*/\1/p' | head -1)
    echo "  .. position ${a:-?} then ${b:-?}"
    # The tracks are 1.5 s: the second reading may be in the next one.
    check "$(yes_if "[ -n '$a' ] && [ -n '$b' ] && [ '$a' != '$b' ]")" "the position moves while it plays"
    shot MEM-1 playing >/dev/null
    # Three 1.5-second tracks: the album ends, the next plays, the wall opens it.
    i=0; while ! vctl tree "$w" | grep -q 'button "Go to Night Sky"'; do sleep 0.5; i=$((i + 1)); [ $i -gt 20 ] && break; done
    check "$(yes_if "vctl tree $w | grep -q 'button \"Play Night Sky\"' && vctl tree $w | grep -q 'button \"Go to Night Sky\"'")" "the album ends: the next one plays and the wall opens it"
    # The play mode's button: on to a random album, written as Settings writes it.
    vctl act "$w" "$(node "$w" 'button "When this album ends: play the next album"')" press >/dev/null; sleep 0.8
    check "$(yes_if "grep -q 'after-album = \"random\"' '$cfg'")" "the play mode's button: a random album, in desktop.toml"
    vctl act "$w" "$(node "$w" 'button "Pause"')" press >/dev/null; sleep 0.5
    # Search.
    for pair in 'cobalt:2 of 5 albums' 'art:missing:1 of 5 albums' 'art:small:1 of 5 albums'; do
        q=${pair%:*}; want=${pair##*:}
        vctl act "$w" "$(node "$w" 'field "search"')" set "$q" >/dev/null; sleep 1
        check "$(yes_if "vctl tree $w | grep -q '$want'")" "search \"$q\": $want"
    done
    shot MEM-1 search >/dev/null
    vctl act "$w" "$(node "$w" 'field "search"')" set "" >/dev/null; sleep 0.8
    # Settings.
    vctl focus "$w" >/dev/null; vctl key ctrl+, >/dev/null; sleep 2
    p=$(win_index "AlbumWall Settings")
    check "$(yes_if "[ -n '$p' ]")" "Ctrl+, opens AlbumWall Settings"
    [ -n "$p" ] || { printf '%s\n' "$kept" > "$cfg"; return; }
    check "$(yes_if "vctl tree $p | grep -q 'Suite Music  —  as it is now'")" "Settings › Library: the library's numbers, as it is now"
    shot MEM-1 settings >/dev/null
    vctl act "$p" "$(node "$p" 'button "page: Appearance"')" press >/dev/null; sleep 0.8
    vctl act "$p" "$(node "$p" 'checkbox "Show meters in the play controls"')" press >/dev/null; sleep 1
    check "$(yes_if "grep -q 'meters = false' '$cfg'")" "a check box written: meters = false"
    check "$(yes_if "vctl tree $p | grep -F 'checkbox \"Show meters in the play controls\"' | grep -q off")" "the check box shows it off"
    printf '%s\n' "$kept" > "$cfg"
}
