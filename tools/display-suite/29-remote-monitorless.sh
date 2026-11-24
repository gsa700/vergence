# Remote milestone 1, step 6: connecting when no session of the person's is
# running starts one with no monitors (Dave: "log in remotely and have no
# active session on the server side. Like ssh can do, but with the vergence
# desktop"), at the viewer's size and scale; it stays when the viewer goes,
# and the next viewer finds it. Its own throwaway HOME and runtime folder:
# never the session on the monitors, nor the suite's headless one.
scenario() {
    base=$HOME/.cache/vergence-monitorless-test
    mrun=$base/run
    mhome=$base/home
    # Only the session this test started, by its process group (its
    # vergence-session is the group's leader: vergence-remote starts it with
    # setsid), never by name: a person's own session with no monitors runs
    # the same commands (one of Dave's was killed so, 2026-11-23 22:35).
    pidfile=$base/session.pid
    ours() { [ -s "$pidfile" ] && pgrep -g "$(cat "$pidfile")" >/dev/null 2>&1; }
    stop_it() {
        if ours; then
            kill -- "-$(cat "$pidfile")" 2>/dev/null
            i=0; while ours && [ $i -lt 30 ]; do sleep 0.2; i=$((i + 1)); done
            ours && kill -9 -- "-$(cat "$pidfile")" 2>/dev/null
        fi
        rm -f "$pidfile"
    }
    stop_it
    rm -rf "$base"
    mkdir -p "$mhome/.config/vergence" "$base/sshrun"
    # The session starter: notes its pid (the group's), then the real one.
    # (Its comment for vergence-remote's check that it knows --monitorless.)
    printf '#!/bin/sh\n# hands --monitorless) to vergence-session\necho $$ > "%s"\nexec "%s" "$@"\n' \
        "$pidfile" "$ROOT/tools/vergence-session" > "$base/session.sh"
    chmod 755 "$base/session.sh"
    printf '[remote]\nenabled = false\n' > "$mhome/.config/vergence/desktop.toml"
    probe() {
        # XDG_RUNTIME_DIR as an SSH login's own (empty: no session there), so
        # the search never reaches the usual places, where the real session is.
        env -u WAYLAND_DISPLAY -u VERGENCE_BUS XDG_RUNTIME_DIR="$base/sshrun" HOME="$mhome" \
            VERGENCE_MONITORLESS_DIR="$mrun" \
            VERGENCE_SESSION="$base/session.sh" VERGENCE_SERVER="$TARGET/vergence-server" \
            VERGENCE_DEFAULTS=none VERGENCE_CONTROL=1 PATH="$TARGET:$PATH" \
            "$TARGET/vergence-remote-probe" "$@" -- "$TARGET/vergence-remote"
    }
    # Off in the settings: nothing started, and the viewer told why.
    probe --size 1600x900 --frames 1 --seconds 5 > "$OUT/off.txt" 2>&1
    check "$(yes_if "grep -q 'Vergence Remote is off' '$OUT/off.txt' && [ ! -e '$pidfile' ]")" "Remote off: nothing started, the viewer told so"
    printf '[remote]\nenabled = true\n\n[x11]\nenabled = false\n' > "$mhome/.config/vergence/desktop.toml"
    probe --size 1600x900 --frames 2 --seconds 40 --out "$OUT/first.ppm" > "$OUT/first.txt" 2>&1
    sed 's/^/  .. /' "$OUT/first.txt" | head -4
    check "$(yes_if "grep -q 'probe: 2 frame(s), 1600x900' '$OUT/first.txt'")" "no session running: one started, its frames at the viewer's size"
    pid=$(pgrep -g "$(cat "$pidfile" 2>/dev/null)" -f -- "--monitorless --screen" 2>/dev/null | head -1)
    check "$(yes_if "[ -n '$pid' ]")" "the session runs with no monitors"
    log=$mhome/.local/state/vergence/server.log
    check "$(yes_if "grep -q 'MEM-1: in memory.*1600x900' '$log'")" "its screen is the viewer's size"
    # The viewer gone: the session stays, and the next viewer gets the same one.
    sleep 1
    check "$(yes_if "kill -0 '$pid' 2>/dev/null")" "the session stays when the viewer goes"
    probe --size 1600x900 --frames 1 --seconds 10 > "$OUT/second.txt" 2>&1
    first=$(cat "$pidfile")
    check "$(yes_if "grep -q 'probe: 1 frame' '$OUT/second.txt' && [ \"\$(cat '$pidfile')\" = '$first' ]")" "the next viewer finds the same session (none started again)"
    # The viewer's window resized: the screen follows once it holds, and the
    # view is the window's size again at one pixel to one.
    probe --size 1600x900 --frames 3 --seconds 10 --script "wait 300; resize 1280 800; wait 2000" > "$OUT/resize.txt" 2>&1
    check "$(yes_if "grep -q 'MEM-1: now 1280x800 (was 1600x900)' '$log'")" "the screen follows the viewer's window (1600x900 to 1280x800)"
    check "$(yes_if "grep -q 'probe: [0-9]* frame(s), 1280x800 scale 1.000' '$OUT/resize.txt'")" "the frames after: the new size at one to one"
    # No lock and no test unlock in a person's session.
    VCTL_SOCKET=$mrun/vergence-control vctl lock now > "$OUT/lock.txt" 2>&1
    sleep 0.5
    check "$(yes_if "VCTL_SOCKET='$mrun/vergence-control' vctl lock | grep -q '^unlocked'")" "nothing to lock (no desk)"
    cp "$log" "$OUT/monitorless-server.log" 2>/dev/null
    stop_it
    check "$(yes_if "! pgrep -g '$first' >/dev/null 2>&1")" "stopped at the end"
}
