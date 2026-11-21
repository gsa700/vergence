# Files milestone 3, step 6: FileView with the best of Explorer and Finder.
# A window with two tabs (two folders on the command line); one toolbar row
# (Back, Forward, Up, the path in its box, the views, New Folder, the
# filter); the sidebar's three sections; the list grouped by kind under
# headings; the free space in the status line; a third folder handed over
# joins the window as a tab; F2 makes a name a field in its row.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -f "$home/.local/state/vergence/files-session"
    mkdir -p "$home/.config/vergence"
    printf '[files]\n' > "$home/.config/vergence/desktop.toml"
    d=$OUT/folders
    rm -rf "$d"
    mkdir -p "$d/one/inner" "$d/two" "$d/three"
    printf 'x' > "$d/one/notes.txt"
    printf 'x' > "$d/one/picture.png"
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-files $d/two $d/one" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-files"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    w=$(vctl windows | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    vctl tree "$w" > "$OUT/tree.txt"
    t=$OUT/tree.txt
    check "$(yes_if "[ \$(vctl windows | grep -c '\"vergence-files\"') = 1 ] && vctl windows | grep '\"vergence-files\"' | grep -q '\"one\"'")" "two folders open as two tabs of one window, the last in front"
    check "$(yes_if "grep -q 'button \"Back\"' '$t' && grep -q 'button \"Up\"' '$t' && grep -q 'button \"Path\"' '$t' && grep -q 'As a list' '$t' && grep -q 'New folder' '$t' && grep -q 'field \"Narrow\"' '$t'")" "one toolbar row: Back, Up, the path, the views, New Folder, the filter"
    check "$(yes_if "grep -q 'Favorites (open)' '$t' && grep -q 'Locations (open)' '$t' && grep -q 'Recent (open)' '$t'")" "the sidebar's sections: Favorites, Locations, Recent"
    check "$(yes_if "grep -q 'label \"Folders\"' '$t' && grep -q 'label \"Text\"' '$t' && grep -q 'label \"Pictures\"' '$t'")" "the list grouped by kind: Folders, Text, Pictures"
    check "$(yes_if "grep -qE 'label \"[0-9.]+ [KMGT]?B free\"' '$t'")" "the disk's free space in the status line"
    shot MEM-1 two-tabs >/dev/null
    # A folder handed over by another start: a third tab, still one window.
    vctl spawn "$TARGET/vergence-files $d/three" >/dev/null; sleep 3
    check "$(yes_if "[ \$(vctl windows | grep -c '\"vergence-files\"') = 1 ] && vctl windows | grep '\"vergence-files\"' | grep -q '\"three\"'")" "a folder handed over joins the window as a tab"
    check "$(yes_if "grep -q 'tab /.*/three' '$home/.local/state/vergence/files-session' && [ \$(grep -c '^tab ' '$home/.local/state/vergence/files-session') = 3 ]")" "the three tabs kept for a restart"
    # F2 on a chosen row: its name a field in the row.
    vctl key ctrl+w >/dev/null; sleep 1
    n=$(vctl tree "$w" | grep -E 'button "notes.txt"' | grep -oE '#[0-9]+' | head -1 | tr -d '#')
    vctl act "$w" "$n" press >/dev/null; sleep 0.5
    vctl focus "$w" >/dev/null; vctl key F2 >/dev/null; sleep 0.8
    check "$(yes_if "vctl tree '$w' | grep -q 'text field \"New name\" = \"notes.txt\"'")" "F2: the name a field in its row"
    vctl key Escape >/dev/null; sleep 0.5
    shot MEM-1 end >/dev/null
}
