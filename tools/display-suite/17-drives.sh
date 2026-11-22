# Drives milestone 1, step 5: one drive list, kept by the server. A disk
# image (vergence-harness iso) mounted through the server (vbus drive image)
# is in the server's list (vctl drives) and in FileView's Locations with its
# eject button; FileView's eject button lets it go: gone from the server's
# list and from FileView's sidebar. FreeBSD: the media helper must be
# installed (sudo tools/install.sh) and the user in its group. The name
# shown is the volume's label on Linux (udisks), the file's on FreeBSD (the
# helper's /mnt/<name>): matched without regard to case.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -f "$home/.local/state/vergence/files-session"
    mkdir -p "$home/.config/vergence"
    printf '[files]\n' > "$home/.config/vergence/desktop.toml"
    # A run that failed half way may have left its image mounted: put away first.
    sleep 2.5
    for left in $(vctl drives | grep -i vsuite | cut -f1); do "$TARGET/vbus" drive eject "$left" >/dev/null 2>&1; done
    iso=$OUT/vsuite.iso
    "$HARNESS" iso "$iso" VSUITE >/dev/null || { check 0 "an ISO made"; return; }
    r=$("$TARGET/vbus" drive image "$iso" 2>&1)
    at=$(echo "$r" | sed -n 's/^done: //p')
    check "$(yes_if "[ -n '$at' ]")" "a disk image mounted through the server: $r"
    [ -n "$at" ] || return
    sleep 2.5
    check "$(yes_if "vctl drives | grep -q \"^$at	Image	mounted	ejectable\"")" "in the server's list: $(vctl drives | grep -i vsuite | cut -f1-3 | tr '\t' ' ')"
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-files $home" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-files"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    w=$(vctl windows | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    check "$(yes_if "vctl tree '$w' | grep -qi 'button \"Eject VSUITE\"'")" "in FileView's Locations, with its eject button"
    shot MEM-1 mounted >/dev/null
    n=$(vctl tree "$w" | grep -iE 'button "Eject VSUITE"' | grep -oE '#[0-9]+' | head -1 | tr -d '#')
    vctl act "$w" "$n" press >/dev/null
    sleep 3
    check "$(yes_if "! vctl drives | grep -qi vsuite && ! grep -qi vsuite /proc/mounts 2>/dev/null && ! mount -p 2>/dev/null | grep -qi vsuite")" "FileView's eject button: gone from the system and the server's list"
    check "$(yes_if "! vctl tree '$w' | grep -qi 'vsuite'")" "and from FileView's sidebar"
}
