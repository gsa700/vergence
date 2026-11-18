# The seams at every clean scale and one off the grid: the test pattern's
# edge lines on the pixels they belong to, the bar meeting the body, and
# the screen showing what the pattern drew, pixel for pixel (layer 2's
# check, run in its own headless session).
scenario() {
    sh "$ROOT/tools/display-check-remote.sh" --headless "2560x1440 597x336@60" 1 1.25 1.3333333333 1.5 2 1.4416666667 >"$OUT/check.txt" 2>&1
    status=$?
    tail -1 "$OUT/check.txt"
    check "$(yes_if "[ $status -eq 0 ]")" "display check: $(tail -1 "$OUT/check.txt" | cut -d';' -f1)"
}
