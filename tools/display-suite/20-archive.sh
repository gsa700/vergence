# Archive milestone 1: an archive opened in FileView is extracted beside
# itself by the confined helper (vergence-unpack), into a folder named after
# it; what was left out is said; one top folder in an archive is kept as it is.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -f "$home/.local/state/vergence/files-session"
    d=$OUT/box
    rm -rf "$d"
    mkdir -p "$d"
    python3 - "$d" <<'PY'
import sys, zipfile, tarfile, io
d = sys.argv[1]
with zipfile.ZipFile(d + "/photos.zip", "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("one.txt", "one")
    z.writestr("sub/two.txt", "two")
    z.writestr("../escaped.txt", "x")
with tarfile.open(d + "/project-1.2.tar.gz", "w:gz") as t:
    data = b"hello"
    i = tarfile.TarInfo("project-1.2/README"); i.size = len(data)
    t.addfile(i, io.BytesIO(data))
PY
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-files $d" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-files"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    w=$(vctl windows | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    open_item() {
        n=$(vctl tree "$w" | grep -E "button \"$1\"" | grep -oE '#[0-9]+' | head -1 | tr -d '#')
        vctl act "$w" "$n" press >/dev/null; sleep 0.5
        vctl focus "$w" >/dev/null; vctl key Return >/dev/null
        i=0; while [ ! -d "$d/$2" ] && [ $i -lt 50 ]; do sleep 0.2; i=$((i + 1)); done
        sleep 1
    }
    open_item photos.zip photos
    check "$(yes_if "[ \"\$(cat '$d/photos/one.txt')\" = one ] && [ \"\$(cat '$d/photos/sub/two.txt')\" = two ]")" "a zip opened: extracted beside itself into a folder of its name"
    check "$(yes_if "[ ! -e '$OUT/escaped.txt' ] && [ ! -e '$d/escaped.txt' ]")" "a name with .. not made outside"
    check "$(yes_if "vctl tree '$w' | grep -q 'left out'")" "the item left out said in the strip"
    check "$(yes_if "! ls -A '$d' | grep -q '^\\.vergence-unpack'")" "no hidden folder left behind"
    shot MEM-1 extracted >/dev/null
    open_item project-1.2.tar.gz project-1.2
    check "$(yes_if "[ \"\$(cat '$d/project-1.2/README')\" = hello ] && [ ! -e '$d/project-1.2/project-1.2' ]")" "one folder at the top kept as it is"
}
