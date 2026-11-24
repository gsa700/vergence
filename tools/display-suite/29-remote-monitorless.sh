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
    stop_it() {
        pid=$(pgrep -f -- "--monitorless --screen" | head -1)
        [ -n "$pid" ] && pkill -f -- "vergence-session --monitorless"
        [ -n "$pid" ] && kill "$pid" 2>/dev/null
        sleep 1
    }
    stop_it
    rm -rf "$base"
    mkdir -p "$mhome/.config/vergence" "$base/sshrun"
    printf '[remote]\nenabled = false\n' > "$mhome/.config/vergence/desktop.toml"
    probe() {
        # XDG_RUNTIME_DIR as an SSH login's own (empty: no session there), so
        # the search never reaches the usual places, where the real session is.
        env -u WAYLAND_DISPLAY -u VERGENCE_BUS XDG_RUNTIME_DIR="$base/sshrun" HOME="$mhome" \
            VERGENCE_MONITORLESS_DIR="$mrun" \
            VERGENCE_SESSION="$ROOT/tools/vergence-session" VERGENCE_SERVER="$TARGET/vergence-server" \
            VERGENCE_DEFAULTS=none VERGENCE_CONTROL=1 PATH="$TARGET:$PATH" \
            "$TARGET/vergence-remote-probe" "$@" -- "$TARGET/vergence-remote"
    }
    # Off in the settings: nothing started, and the viewer told why.
    probe --size 1600x900 --frames 1 --seconds 5 > "$OUT/off.txt" 2>&1
    check "$(yes_if "grep -q 'Vergence Remote is off' '$OUT/off.txt' && ! pgrep -f -- '--monitorless --screen' >/dev/null")" "Remote off: nothing started, the viewer told so"
    printf '[remote]\nenabled = true\n\n[x11]\nenabled = false\n' > "$mhome/.config/vergence/desktop.toml"
    probe --size 1600x900 --frames 2 --seconds 40 --out "$OUT/first.ppm" > "$OUT/first.txt" 2>&1
    sed 's/^/  .. /' "$OUT/first.txt" | head -4
    check "$(yes_if "grep -q 'probe: 2 frame(s), 1600x900' '$OUT/first.txt'")" "no session running: one started, its frames at the viewer's size"
    pid=$(pgrep -f -- "--monitorless --screen" | head -1)
    check "$(yes_if "[ -n '$pid' ]")" "the session runs with no monitors"
    log=$mhome/.local/state/vergence/server.log
    check "$(yes_if "grep -q 'MEM-1: in memory.*1600x900' '$log'")" "its screen is the viewer's size"
    # The viewer gone: the session stays, and the next viewer gets the same one.
    sleep 1
    check "$(yes_if "kill -0 '$pid' 2>/dev/null")" "the session stays when the viewer goes"
    probe --size 1600x900 --frames 1 --seconds 10 > "$OUT/second.txt" 2>&1
    check "$(yes_if "grep -q 'probe: 1 frame' '$OUT/second.txt' && [ \"\$(pgrep -fc -- '--monitorless --screen')\" = 1 ]")" "the next viewer finds the same session (none started again)"
    # No lock and no test unlock in a person's session.
    VCTL_SOCKET=$mrun/vergence-control vctl lock now > "$OUT/lock.txt" 2>&1
    sleep 0.5
    check "$(yes_if "VCTL_SOCKET='$mrun/vergence-control' vctl lock | grep -q '^unlocked'")" "nothing to lock (no desk)"
    cp "$log" "$OUT/monitorless-server.log" 2>/dev/null
    stop_it
    check "$(yes_if "! pgrep -f -- '--monitorless --screen' >/dev/null")" "stopped at the end"
}
