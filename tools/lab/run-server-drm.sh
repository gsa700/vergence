#!/bin/sh
# vergence-server on the real monitors (stage 2), from a text console
# (Ctrl+Alt+F2 on Hubble), not inside Xfce. Ctrl+Alt+Backspace quits; Ctrl+Alt+F9
# goes back to Xfce. `vctl restart` (exit status 75) starts the server again,
# with whatever build is there now, so it can be rebuilt and restarted over SSH.
export XDG_RUNTIME_DIR=/tmp/xdg-$(id -u)
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
export LIBSEAT_BACKEND=seatd
# The development control socket (vctl): off in the server unless asked.
export VERGENCE_CONTROL=${VERGENCE_CONTROL:-1}
export RUST_LOG=${RUST_LOG:-info,smithay=warn}
LOG=${XDG_STATE_HOME:-$HOME/.local/state}/vergence/server.log; mkdir -p "$(dirname "$LOG")"
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/release/vergence-server
while :; do
    echo "vergence-server on the real monitors; log in $LOG"
    "$BIN" --drm -c foot > "$LOG" 2>&1
    status=$?
    [ "$status" -eq 75 ] || break
    echo "restart requested"
    sleep 1
done
echo "vergence-server exited ($status); last lines:"
tail -8 "$LOG"
