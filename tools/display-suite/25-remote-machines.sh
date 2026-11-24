# Remote milestone 1, step 4: Vergence Remote's machine list. Started with no
# host it lists the saved computers (here "localhost", this session) and
# offers to add one; choosing a computer opens its window.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    mkdir -p "$home/.config/vergence"
    printf 'localhost\n' > "$home/.config/vergence/remote-machines"
    vctl spawn "$TARGET/vergence-remote-viewer" >/dev/null
    i=0; while ! vctl windows | grep -q '"Vergence Remote"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 1
    check "$(yes_if "vctl windows | grep -q '\"Vergence Remote\" \"vergence-remote-viewer\"'")" "the machine list's window"
    w=$(vctl windows | grep '"Vergence Remote"' | head -1 | cut -d' ' -f1)
    vctl tree "$w" > "$OUT/list.txt"
    check "$(yes_if "grep -q 'button \"Open localhost\"' '$OUT/list.txt' && grep -q 'field \"Computer\"' '$OUT/list.txt'")" "the saved computer, and a field to add one"
    shot MEM-1 list >/dev/null
    n=$(vctl find '* > button:Open localhost' | head -1 | sed -E 's/.*#([0-9]+).*/\1/')
    vctl act "$w" "$n" press >/dev/null
    i=0; while ! vctl windows | grep -q '— Vergence Remote'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    check "$(yes_if "vctl windows | grep '\"vergence-remote-viewer\"' | grep -q '— Vergence Remote'")" "choosing it opens its window"
    check "$(yes_if "vctl remote list | grep -qE 'frames [1-9]'")" "and its session draws"
    pkill -f vergence-remote-viewer; sleep 1
}
