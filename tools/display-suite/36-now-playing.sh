# AlbumWall milestone 2, step 6: Now Playing for the whole desktop
# (nowplaying.rs). The server sees AlbumWall come onto its session bus as an
# MPRIS player; nothing plays, no tile (quiet); an album played puts up the
# Now Playing tile with the track, the player, the controls and the album's
# cover as the card's picture; the media.play-pause action and the media keys
# (XF86AudioPlay, through the seat) reach AlbumWall whichever window has the
# keyboard; the card's Next goes to the next track; AlbumWall gone, the tile
# goes.
scenario() {
    command -v dbus-send >/dev/null || { echo "  ok   skipped: no dbus-send on this machine"; return 0; }
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
for a in range(3):
    artist, album = f"Artist {a:03d}", f"Album {a:03d}"
    folder = os.path.join(out, artist, f"{1970 + a} - {album}")
    os.makedirs(folder)
    for n in (1, 2):
        open(os.path.join(folder, f"{n:02d}.flac"), "wb").write(flac([
            ("ALBUMARTIST", artist), ("ARTIST", artist), ("ALBUM", album), ("TITLE", f"Track {n}"),
            ("TRACKNUMBER", n)]))
    # Its cover, a file of its own (40 x 40, red): the card's picture.
    import zlib
    def chunk(t, b): return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xffffffff)
    row = b"\x00" + b"\xc0\x20\x20" * 40
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 40, 40, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(row * 40)) + chunk(b"IEND", b"")
    open(os.path.join(folder, "cover.png"), "wb").write(png)
PY
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    cfg=$home/.config/vergence/desktop.toml
    mkdir -p "$home/.config/vergence/albumwall"
    printf '# AlbumWall for Vergence: its libraries\ncurrent\t0\nfolder\tNow\t%s\n' "$music" \
        > "$home/.config/vergence/albumwall/libraries"
    rm -f "$home/.config/vergence/albumwall/session"
    kept=$(cat "$cfg" 2>/dev/null)
    awk '/^\[/ { skip = ($0 == "[albumwall]") } !skip' "$cfg" > "$cfg.new" 2>/dev/null && mv "$cfg.new" "$cfg"
    aw="env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall"
    vctl spawn "$aw" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    w=$(win_index AlbumWall)
    i=0; while ! vctl tree "$w" | grep -q '3 albums · 6 tracks'; do sleep 0.3; i=$((i + 1)); [ $i -gt 60 ] && break; done
    check "$(yes_if "vctl tree $w | grep -q '3 albums · 6 tracks'")" "the library read: 3 albums"
    dest=org.mpris.MediaPlayer2.albumwall
    status() {
        dbus-send --session --print-reply --dest=$dest /org/mpris/MediaPlayer2 org.freedesktop.DBus.Properties.Get \
            string:org.mpris.MediaPlayer2.Player string:PlaybackStatus 2>&1 | grep -oE '"(Playing|Paused|Stopped)"' | tr -d '"'
    }
    title() {
        dbus-send --session --print-reply --dest=$dest /org/mpris/MediaPlayer2 org.freedesktop.DBus.Properties.Get \
            string:org.mpris.MediaPlayer2.Player string:Metadata 2>&1 | grep -A1 'xesam:title' | grep -oE '"Track [0-9]"' | tr -d '"'
    }
    np() { vctl tiles | grep '^nowplaying '; }
    sleep 0.5
    check "$(yes_if "[ -z \"\$(np)\" ]")" "a player on the bus with nothing playing: no tile"

    press() { vctl act "$w" "$(vctl tree "$w" | grep -F "$1" | grep -oE '#[0-9]+' | head -1 | tr -d '#')" press >/dev/null; }
    press 'button "Album 000, Artist 000"'
    i=0; while ! vctl tree "$w" | grep -q 'button "1 Track 1'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    press 'button "1 Track 1'
    i=0; while [ -z "$(np)" ]; do sleep 0.1; i=$((i + 1)); [ $i -gt 20 ] && break; done
    t=$(np)
    echo "  .. $t" >&2
    check "$(yes_if "echo \"\$t\" | grep -q 'Desktop \"Track 1\" \"Playing\"'")" "playing: the Now Playing tile, its track"
    check "$(yes_if "echo \"\$t\" | grep -q 'controls 4'")" "previous, play or pause, next and the seek bar"
    i=0; while ! np | grep -q 'icon "vergence-albumwall" badge Some(Play)'; do sleep 0.2; i=$((i + 1)); [ $i -gt 15 ] && break; done
    check "$(yes_if "np | grep -q 'icon \"vergence-albumwall\" badge Some(Play)'")" "on the dock, AlbumWall's icon with the play badge"
    shot MEM-1 dock-playing >/dev/null
    i=0; while ! np | grep -q 'picture 40x40'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    check "$(yes_if "np | grep -q 'picture 40x40'")" "the album's cover on its card"

    vctl action media.play-pause >/dev/null; sleep 0.4
    s=$(status)
    check "$(yes_if "[ '$s' = Paused ] && np | grep -q '\"Paused\"' && np | grep -q 'badge Some(Pause)'")" "media.play-pause pauses AlbumWall ($s), the tile says so (the pause badge)"
    # The card (paused: the short track holds still), opened as a person opens it: the pointer resting on the tile.
    set -- $(vctl shelf | sed -nE 's/^tile nowplaying at (-?[0-9]+),(-?[0-9]+) ([0-9]+)x([0-9]+)$/\1 \2 \3 \4/p') 0 0 0 0
    vctl pointer $(($1 + $3 / 2)) $(($2 + $4 / 2)) >/dev/null
    i=0; while ! vctl tiles | grep -q '^card: nowplaying'; do sleep 0.2; i=$((i + 1)); [ $i -gt 15 ] && break; done
    sleep 0.6
    check "$(yes_if "vctl tiles | grep -q '^card: nowplaying'")" "its card opens on the tile ($(vctl tiles | grep '^card:'))"
    shot MEM-1 card >/dev/null
    vctl pointer 900 400 >/dev/null
    vctl key XF86AudioPlay >/dev/null; sleep 0.4
    s=$(status)
    check "$(yes_if "[ '$s' = Playing ]")" "the play key plays it again ($s)"
    vctl key XF86AudioPlay >/dev/null; sleep 0.3
    vctl tile-action nowplaying next >/dev/null; sleep 0.6
    n=$(title)
    check "$(yes_if "[ '$n' = 'Track 2' ]")" "the card's Next: the next track ($n)"

    $aw --quit >/dev/null 2>&1
    i=0; while [ -n "$(np)" ]; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    check "$(yes_if "[ -z \"\$(np)\" ]")" "AlbumWall gone: the tile goes"
    printf '%s\n' "$kept" > "$cfg"
}
