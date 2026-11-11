#!/bin/sh
# The stress test's "shell": every terminal vterm opens runs this in place of
# the user's shell, and each one takes the next job of the list (run.sh).
# The count is kept in $VSTRESS_DIR, a directory made per number so that two
# terminals starting together cannot take the same one.
# VSTRESS_ONLY=<number> gives every terminal that one job (a job by itself).
n=1
while ! mkdir "$VSTRESS_DIR/taken-$n" 2>/dev/null; do n=$((n + 1)); done
n=${VSTRESS_ONLY:-$n}
flood="$VSTRESS_FLOOD"
case "$n" in
    1) command -v btop >/dev/null 2>&1 && exec btop; exec top ;;
    2) command -v htop >/dev/null 2>&1 && exec htop; exec top ;;
    3) exec "$flood" time "$VSTRESS_DIR/seq.log" seq 1 1000000 ;;
    4) exec "$flood" color "$VSTRESS_DIR/color.log" ;;
    5) exec "$flood" cursor "$VSTRESS_DIR/cursor.log" ;;
    6) exec "$flood" unicode "$VSTRESS_DIR/unicode.log" ;;
    *) exec /bin/sh ;;
esac
