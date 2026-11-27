# Brightness milestone 1: the screens' brightness on a stand-in backlight
# (VERGENCE_BRIGHTNESS_ROOT: a directory laid out as /sys, never a real
# backlight; set there directly, as vdm sets a real one). The Display tile
# with a slider for the built-in panel, the brightness keys stepping 5% (1%
# with Shift) with the level shown as the volume's, the slider setting it,
# and a machine with no control having no Display tile at all.
scenario() {
    sb=$OUT/standin
    light=$sb/sys/class/drm/card0-eDP-1/test_backlight
    mkdir -p "$light"
    printf 'raw\n' > "$light/type"
    printf '1000\n' > "$light/max_brightness"
    printf '505\n' > "$light/brightness"
    export VERGENCE_BRIGHTNESS_ROOT=$sb
    headless "1920x1080 531x299@60" || { check 0 "headless session"; unset VERGENCE_BRIGHTNESS_ROOT; return; }
    raw() { cat "$light/brightness" 2>/dev/null | tr -d '\n'; }
    # wait_raw <value>: the thread sets it within a few tenths of a second.
    wait_raw() { i=0; while [ "$(raw)" != "$1" ] && [ $i -lt 30 ]; do sleep 0.1; i=$((i + 1)); done; }
    vctl tiles > "$OUT/tiles.txt"
    check "$(yes_if "grep -q '^display ' '$OUT/tiles.txt' && grep -q 'Built-in display' '$OUT/tiles.txt'")" "a Display tile with the built-in panel's slider"
    check "$(yes_if "grep -E '^display ' '$OUT/tiles.txt' | grep -q '50%'")" "its level as found, 50% (505 of 1000, above the 1% floor)"
    vctl key BrightnessUp >/dev/null; wait_raw 555
    check "$(yes_if "[ '$(raw)' = 555 ]")" "the brightness key up: 55% ($(raw) of 1000: 1% floor + 55% of the rest)"
    vctl notes > "$OUT/notes.txt"
    check "$(yes_if "grep -q 'Brightness 55%' '$OUT/notes.txt' && grep -q 'Built-in display' '$OUT/notes.txt'")" "the level shown, with the screen's name"
    vctl key shift+BrightnessDown >/dev/null; wait_raw 545
    check "$(yes_if "[ '$(raw)' = 545 ]")" "Shift and the key down: 1% ($(raw))"
    vctl tile-action display eDP-1 0 >/dev/null; wait_raw 10
    check "$(yes_if "[ '$(raw)' = 10 ]")" "the slider at 0: the floor, never black ($(raw))"
    vctl tile-action display eDP-1 100 >/dev/null; wait_raw 1000
    check "$(yes_if "[ '$(raw)' = 1000 ]")" "the slider at 100: full ($(raw))"
    # Changed by something else (a laptop's own keys): the tile follows.
    printf '505\n' > "$light/brightness"
    i=0; while ! vctl tiles | grep -E '^display ' | grep -q '50%' && [ $i -lt 60 ]; do sleep 0.1; i=$((i + 1)); done
    check "$(yes_if "vctl tiles | grep -E '^display ' | grep -q '50%'")" "a level changed elsewhere shows within seconds"
    # No control: no tile.
    rm -rf "$sb/sys/class/drm/card0-eDP-1"
    mkdir -p "$sb/sys/class/drm/card0-DP-1"
    unset VERGENCE_BRIGHTNESS_ROOT
    export VERGENCE_BRIGHTNESS_ROOT=$sb
    headless "1920x1080 531x299@60" || { check 0 "headless session again"; unset VERGENCE_BRIGHTNESS_ROOT; return; }
    check "$(yes_if "! vctl tiles | grep -q '^display '")" "no screen with a control: no Display tile"
    unset VERGENCE_BRIGHTNESS_ROOT
}
