# Window memory per setup: a window on the second screen is rescued when it
# goes, and back where it was when the same screen comes back.
scenario() {
    headless "1920x1080 531x299@60" "2560x1440 597x336@60" || { check 0 "headless session"; return; }
    set -- $(screen_geo MEM-2)
    pattern W --size 400 300 || check 0 "window opens"
    move W $(($1 + 400)) 300
    where=$(win_geo W)
    check "$(yes_if "[ '$(win_on W)' = MEM-2 ]")" "on the second screen: $where"
    vctl screen remove MEM-2 >/dev/null
    sleep 1.5
    check "$(yes_if "[ '$(win_on W)' = MEM-1 ]")" "rescued while it is gone: $(win_geo W)"
    # The same monitor again (same mode, size and name: the same identity).
    vctl screen add --fake 2560x1440 597x336 --name HL2560x1440 >/dev/null
    sleep 2
    check "$(yes_if "[ '$(win_geo W)' = '$where' ]")" "back where it was: $where, now $(win_geo W) on $(win_on W)"
}
