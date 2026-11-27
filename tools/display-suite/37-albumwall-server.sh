# AlbumWall milestone 2, step 7: a music server (Subsonic, as Navidrome) as a
# library, checked against the suite's own stand-in (subsonic-standin.py,
# started here on 127.0.0.1). A wrong password is refused with the server's
# words; the right one lists its music folders and a ticked one fills the
# wall; the libraries file is the owner's alone and holds a salted token,
# never the password; an album played from the server is Playing, its song
# asked for whole and the next one fetched ahead; its lyrics come from the
# server; a server answering garbage gets an error, not a crash; the server
# gone, the wall says it cannot be reached.
scenario() {
    command -v python3 >/dev/null && command -v dbus-send >/dev/null \
        || { echo "  ok   skipped: no python3 or dbus-send on this machine"; return 0; }
    port=47533
    WAV_SECONDS=6 python3 "$ROOT/tools/display-suite/subsonic-standin.py" $port good > "$OUT/standin.log" 2>&1 &
    SRV=$!
    python3 "$ROOT/tools/display-suite/subsonic-standin.py" $((port + 1)) garbage > "$OUT/garbage.log" 2>&1 &
    BAD=$!
    sleep 0.5
    headless "1920x1080 531x299@60" || { check 0 "headless session"; kill $SRV $BAD 2>/dev/null; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -rf "$home/.config/vergence/albumwall" "$home/.cache/vergence/albumwall"
    vctl spawn "env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1
    w=$(win_index AlbumWall)
    node() { vctl tree "$1" | grep -F "$2" | grep -oE '#[0-9]+' | head -1 | tr -d '#'; }
    press() { vctl act "$1" "$(node "$1" "$2")" press >/dev/null; }

    # The sign-in window, from the welcome.
    press "$w" 'button "Sign in to a server'
    i=0; while ! vctl windows | grep -q '"Add a Music Server"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    s=$(vctl windows | grep '"Add a Music Server"' | head -1 | cut -d' ' -f1)
    check "$(yes_if "[ -n '$s' ]")" "the sign-in window opens from the welcome"
    # sign_in <address> <password>: the password typed, as a person types it
    # (a secret field cannot be set through the agent interface).
    sign_in() {
        vctl act "$s" "$(node "$s" 'field "Server"')" set "$1" >/dev/null
        vctl act "$s" "$(node "$s" 'field "User name"')" set dave >/dev/null
        vctl focus "$s" >/dev/null
        vctl act "$s" "$(node "$s" 'field "Password"')" set "" >/dev/null 2>&1
        vctl act "$s" "$(node "$s" 'field "Password"')" focus >/dev/null; sleep 0.3
        vctl key ctrl+a >/dev/null; vctl key BackSpace >/dev/null
        vctl type "$2" >/dev/null; sleep 0.3
        press "$s" 'button "Sign in"'
        sleep 2
        vctl tree "$s" > "$OUT/signin-$3.txt"
    }
    sign_in "127.0.0.1:$port" wrong refused
    check "$(yes_if "grep -q 'would not sign dave in' '$OUT/signin-refused.txt'")" "a wrong password refused, in the server's words"
    sign_in "127.0.0.1:$((port + 1))" sesame garbage
    check "$(yes_if "grep -qE 'would not sign dave in|not answering' '$OUT/signin-garbage.txt' && vctl windows | grep -q '\"AlbumWall\" \"vergence-albumwall\"'")" \
        "a server answering garbage: an error, and AlbumWall still there"
    sign_in "127.0.0.1:$port" sesame signed
    check "$(yes_if "grep -q 'checkbox \"Lossless\"' '$OUT/signin-signed.txt' && grep -q 'checkbox \"Lossy\"' '$OUT/signin-signed.txt'")" "signed in: its music folders to tick"
    press "$s" 'checkbox "Lossy"'; sleep 0.4
    press "$s" 'button "Add'
    i=0; while ! vctl tree "$w" | grep -qE '[0-9]+ albums'; do sleep 0.3; i=$((i + 1)); [ $i -gt 40 ] && break; done
    vctl tree "$w" > "$OUT/wall.txt"
    count=$(grep -oE '[0-9]+ albums · [0-9]+ tracks' "$OUT/wall.txt" | head -1)
    echo "  .. the wall: $count" >&2
    check "$(yes_if "grep -q 'Server Album 00, Artist 0' '$OUT/wall.txt' && [ -n '$count' ]")" "the server's albums on the wall ($count)"
    lib=$home/.config/vergence/albumwall/libraries
    mode=$(stat -c %a "$lib" 2>/dev/null || stat -f %Lp "$lib")
    check "$(yes_if "[ '$mode' = 600 ] && ! grep -q sesame '$lib' && grep -qE '[0-9a-f]{32}' '$lib'")" "the libraries file: the owner's only ($mode), a token, never the password"

    # Played from the server.
    press "$w" 'button "Server Album 00, Artist 0"'
    i=0; while ! vctl tree "$w" | grep -q 'button "Play Server Album 00"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    press "$w" 'button "Play Server Album 00"'
    i=0; while ! grep -q '^stream so-0-1' "$OUT/standin.log"; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    sleep 1.5
    status=$(dbus-send --session --print-reply --dest=org.mpris.MediaPlayer2.albumwall /org/mpris/MediaPlayer2 \
        org.freedesktop.DBus.Properties.Get string:org.mpris.MediaPlayer2.Player string:PlaybackStatus 2>&1 | grep -oE '"(Playing|Paused|Stopped)"' | tr -d '"')
    check "$(yes_if "[ '$status' = Playing ]")" "an album from the server playing ($status)"
    check "$(yes_if "grep -q '^stream so-0-1 raw' '$OUT/standin.log'")" "its song asked for whole (format=raw: the music as it is kept)"
    i=0; while ! grep -q '^stream so-0-2' "$OUT/standin.log"; do sleep 0.3; i=$((i + 1)); [ $i -gt 12 ] && break; done
    check "$(yes_if "grep -q '^stream so-0-2' '$OUT/standin.log'")" "the next song fetched ahead, before the first ends (gapless)"
    i=0; while ! vctl tree "$w" | grep -q 'button "Show Lyrics"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 15 ] && break; done
    check "$(yes_if "vctl tree $w | grep -q 'button \"Show Lyrics\"'")" "the server's lyrics for the song playing"
    shot MEM-1 playing >/dev/null

    # The server gone: AlbumWall started again finds it not answering, says so,
    # and forgets nothing.
    kill $SRV 2>/dev/null; wait $SRV 2>/dev/null
    $TARGET/vergence-albumwall --quit >/dev/null 2>&1
    i=0; while vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done
    sleep 1
    vctl spawn "env VERGENCE_SOUND_OFF=pace $TARGET/vergence-albumwall" >/dev/null
    i=0; while ! vctl windows | grep -q '"AlbumWall" "vergence-albumwall"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    w=$(win_index AlbumWall)
    i=0; while ! vctl tree "$w" | grep -q 'cannot be reached'; do sleep 0.5; i=$((i + 1)); [ $i -gt 70 ] && break; done
    vctl tree "$w" > "$OUT/gone.txt"
    check "$(yes_if "grep -q 'cannot be reached' '$OUT/gone.txt'")" "the server gone: the wall says it cannot be reached"
    shot MEM-1 gone >/dev/null
    kill $BAD 2>/dev/null; wait $BAD 2>/dev/null
}
