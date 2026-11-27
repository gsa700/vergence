# AlbumWall milestone 2, step 5: AlbumWall on the session bus as a media
# player (MPRIS) and its command line (mpris.rs). Checked through the bus the
# headless session runs, with the system's own dbus-send and gdbus, as any
# MPRIS client would: Stopped before anything plays; an album played from the
# keys is Playing with its track's title and album in Metadata; Pause over the
# bus pauses it; the change is pushed (PropertiesChanged, watched), not only
# answered; a second start with --play or --pause hands it to the first and
# goes, a plain second start opens no second window; --quit closes it, and a
# command with nothing running says so (exit 1).
scenario() {
    command -v dbus-send >/dev/null && command -v gdbus >/dev/null \
        || { echo "  ok   skipped: no dbus-send or gdbus on this machine"; return 0; }
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
PY
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    check "$(yes_if "[ -n \"\${DBUS_SESSION_BUS_ADDRESS:-}\" ]")" "the session's D-Bus address (${DBUS_SESSION_BUS_ADDRESS:-none})"
    home=$HOME/.cache/vergence-headless/home
    cfg=$home/.config/vergence/desktop.toml
    mkdir -p "$home/.config/vergence/albumwall"
    printf '# AlbumWall for Vergence: its libraries\ncurrent\t0\nfolder\tMpris\t%s\n' "$music" \
        > "$home/.config/vergence/albumwall/libraries"
    rm -f "$home/.config/vergence/albumwall/session"
    kept=$(cat "$cfg" 2>/dev/null)
    # No [albumwall] table: auto-play off, Stop at an album's end (the defaults).
    awk '/^\[/ { skip = ($0 == "[albumwall]") } !skip' "$cfg" > "$cfg.new" 2>/dev/null && mv "$cfg.new" "$cfg"
    aw="env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall"
    vctl spawn "$aw" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    w=$(win_index AlbumWall)
    i=0; while ! vctl tree "$w" | grep -q '3 albums · 6 tracks'; do sleep 0.3; i=$((i + 1)); [ $i -gt 60 ] && break; done
    check "$(yes_if "vctl tree $w | grep -q '3 albums · 6 tracks'")" "the library read: 3 albums"

    # The bus, as a client sees it.
    dest=org.mpris.MediaPlayer2.albumwall
    get() { # get <property>: the Player property's value line
        dbus-send --session --print-reply --dest=$dest /org/mpris/MediaPlayer2 \
            org.freedesktop.DBus.Properties.Get string:org.mpris.MediaPlayer2.Player "string:$1" 2>&1
    }
    call() { dbus-send --session --print-reply --dest=$dest /org/mpris/MediaPlayer2 "org.mpris.MediaPlayer2.Player.$1" 2>&1; }
    status() { get PlaybackStatus | grep -oE '"(Playing|Paused|Stopped)"' | tr -d '"'; }
    gdbus monitor --session --dest $dest > "$OUT/monitor.txt" 2>&1 &
    MON=$!
    sleep 0.5
    s=$(status)
    check "$(yes_if "[ '$s' = Stopped ]")" "on the bus as a player, Stopped before anything plays ($s)"
    dbus-send --session --print-reply --dest=$dest /org/mpris/MediaPlayer2 org.freedesktop.DBus.Properties.Get \
        string:org.mpris.MediaPlayer2 string:Identity > "$OUT/identity.txt" 2>&1
    check "$(yes_if "grep -q '\"AlbumWall\"' '$OUT/identity.txt'")" "its Identity: AlbumWall"

    # An album played as a person would: its tile pressed, then its first track.
    press() { vctl act "$w" "$(vctl tree "$w" | grep -F "$1" | grep -oE '#[0-9]+' | head -1 | tr -d '#')" press >/dev/null; }
    press 'button "Album 000, Artist 000"'
    i=0; while ! vctl tree "$w" | grep -q 'button "1 Track 1'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    press 'button "1 Track 1'
    sleep 0.3
    s=$(status)
    get Metadata > "$OUT/metadata.txt"
    check "$(yes_if "[ '$s' = Playing ]")" "an album playing: Playing ($s)"
    check "$(yes_if "grep -q 'Track 1' '$OUT/metadata.txt' && grep -q 'Album 000' '$OUT/metadata.txt'")" "Metadata: its track's title and its album"
    check "$(yes_if "grep -q 'mpris:trackid' '$OUT/metadata.txt' && ! grep -q 'flac' '$OUT/metadata.txt'")" "a track id, and no path of the file on the bus"
    call Pause >/dev/null; sleep 0.4
    s=$(status)
    check "$(yes_if "[ '$s' = Paused ]")" "Pause over the bus: Paused ($s)"
    check "$(yes_if "vctl tree $w | grep -q 'button \"Play\"'")" "the play bar shows it paused"

    # Second starts: commands handed to the first.
    $aw --play > "$OUT/second-play.txt" 2>&1; r1=$?
    sleep 0.4; s1=$(status)
    $aw --pause > "$OUT/second-pause.txt" 2>&1; r2=$?
    sleep 0.4; s2=$(status)
    check "$(yes_if "[ $r1 = 0 ] && [ '$s1' = Playing ] && [ $r2 = 0 ] && [ '$s2' = Paused ]")" "a second start's --play and --pause reach it ($r1 $s1, $r2 $s2)"
    timeout 10 $aw > "$OUT/second-plain.txt" 2>&1; r3=$?
    sleep 0.5
    n=$(vctl windows | grep -c '"AlbumWall" "vergence-albumwall"')
    check "$(yes_if "[ $r3 = 0 ] && [ $n = 1 ]")" "a plain second start goes, no second window ($r3, $n windows)"
    sleep 0.5
    kill $MON 2>/dev/null
    check "$(yes_if "grep -q 'PropertiesChanged' '$OUT/monitor.txt' && grep -q 'Paused' '$OUT/monitor.txt'")" "the change pushed: PropertiesChanged with Paused"

    # --quit closes it; then a command finds nothing running.
    $aw --quit > "$OUT/second-quit.txt" 2>&1
    i=0; while vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    check "$(yes_if "! vctl windows | grep -q '\"AlbumWall\" \"vergence-albumwall\"'")" "--quit closes it"
    owned() {
        dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
            org.freedesktop.DBus.NameHasOwner string:$dest 2>/dev/null | grep -q 'boolean true'
    }
    i=0; while owned; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    check "$(yes_if "! owned")" "and ends: its name let go"
    $aw --next > "$OUT/nothing-running.txt" 2>&1; r4=$?
    check "$(yes_if "[ $r4 = 1 ] && grep -q 'not running' '$OUT/nothing-running.txt'")" "--next with nothing running says so ($r4)"
    printf '%s\n' "$kept" > "$cfg"
}
