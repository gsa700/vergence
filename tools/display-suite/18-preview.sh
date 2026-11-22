# Preview milestone 1, step 7: Space's preview is a window of its own over
# FileView's, shaped for what it shows (docs/preview-milestone-1.md). A text
# file opens it tall, in the fixed-width font; the arrow keys, pressed in the
# preview, move FileView's selection and the preview follows; a picture
# (the suite's own shot of the screen, 16:9) opens it at its proportions; a
# PDF's page (300 x 200 points) beside the thumbnail strip opens it wide;
# a FLAC (Hymie's helper) opens it with its title, cover and waveform; a
# WAV with its album's Cover.png beside it plays at once (a paced, silent
# output), its waveform drawn whole while it plays; Escape closes it.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    rm -f "$home/.local/state/vergence/files-session"
    mkdir -p "$home/.config/vergence"
    printf '[files]\n' > "$home/.config/vergence/desktop.toml"
    d=$OUT/peek
    rm -rf "$d"
    mkdir -p "$d"
    i=1; while [ $i -le 40 ]; do echo "line $i of a text file shown whole in the preview"; i=$((i + 1)); done > "$d/notes.txt"
    printf 'second text file\n' > "$d/second.txt"
    printf '%%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]/Contents 4 0 R>>endobj\n4 0 obj<</Length 27>>stream\n0 0 1 rg 20 20 260 160 re f\nendstream endobj\ntrailer<</Root 1 0 R>>\n%%%%EOF\n' > "$d/page.pdf"
    cp "$ROOT/crates/vergence-hymie/fixtures/tone16.flac" "$d/tone.flac"
    # Ten seconds of silence as a WAV (8 kHz mono 16-bit), its album's cover
    # beside it as Cover.png (any case, as AlbumWall finds it).
    { printf 'RIFF\044\161\002\000WAVEfmt \020\000\000\000\001\000\001\000\100\037\000\000\200\076\000\000\002\000\020\000data\000\161\002\000'; head -c 160000 /dev/zero; } > "$d/long.wav"
    shot MEM-1 empty >/dev/null
    cp "$SHOTS/MEM-1.png" "$d/screen.png"
    vctl spawn "env VERGENCE_SOUND_OFF=pace PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-files $d" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-files"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    w=$(vctl windows | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    # size <title>: the preview window's "width height", or nothing.
    size() { vctl windows | grep "\"$1\" \"vergence-files\"" | grep 'kind=Dialog' | sed -n 's/.*width: \([0-9]*\), height: \([0-9]*\) }).*/\1 \2/p' | head -1; }
    # treeof <title>: the part of `vctl tree` (every native window's tree,
    # one after another) for the window with that title.
    treeof() { vctl tree | awk -v t="\"$1\"" '/^window / { on = index($0, t) > 0 } on'; }
    pick() { # pick <name>: a row clicked, FileView given the keyboard, Space
        n=$(vctl tree "$w" | grep -E "button \"$1\"" | grep -oE '#[0-9]+' | head -1 | tr -d '#')
        vctl act "$w" "$n" press >/dev/null; sleep 0.4
        vctl focus "$w" >/dev/null; vctl key space >/dev/null; sleep 2
    }
    pick notes.txt
    s=$(size notes.txt)
    check "$(yes_if "[ -n '$s' ] && [ \$(echo $s | cut -d' ' -f2) -gt 400 ]")" "a text file: a preview window of its own, tall for 40 lines ($s)"
    shot MEM-1 text >/dev/null
    # The preview has the keyboard: Down moves FileView's selection, the preview follows.
    p=$(vctl windows | grep 'kind=Dialog' | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    vctl focus "$p" >/dev/null; vctl key Down >/dev/null; sleep 1.5
    check "$(yes_if "[ -n \"\$(size second.txt)\" ] && [ -z \"\$(size notes.txt)\" ]")" "Down in the preview: FileView's next file shown in it"
    vctl key Escape >/dev/null; sleep 1
    check "$(yes_if "! vctl windows | grep 'kind=Dialog' | grep -q '\"vergence-files\"'")" "Escape closes the preview"
    pick screen.png
    s=$(size screen.png)
    check "$(yes_if "[ -n '$s' ] && near \$(echo $s | awk '{ print \$1 / \$2 }') 1.778 0.08")" "a picture: the window at its proportions, 16:9 ($s)"
    shot MEM-1 picture >/dev/null
    vctl key Escape >/dev/null; sleep 1
    pick page.pdf
    s=$(size page.pdf)
    check "$(yes_if "[ -n '$s' ] && [ \$(echo $s | awk '{ print (\$1 > 1.4 * \$2) }') = 1 ]")" "a PDF: its page beside the thumbnails, a wide window for a wide page ($s)"
    check "$(yes_if "vctl tree \$(vctl windows | grep 'kind=Dialog' | grep '\"vergence-files\"' | head -1 | cut -d' ' -f1) | grep -c 'button \"Page [0-9]*\"' | grep -qx 1")" "the page's one thumbnail in the strip"
    shot MEM-1 pdf >/dev/null
    vctl key Escape >/dev/null; sleep 1
    pick tone.flac
    s=$(size tone.flac)
    p=$(vctl windows | grep 'kind=Dialog' | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    echo "  (the preview is window $p)"
    treeof tone.flac > "$OUT/sound-tree.txt"
    check "$(yes_if "[ -n '$s' ] && grep -q 'label \"Test Tone\"' '$OUT/sound-tree.txt' && grep -q 'Vergence' '$OUT/sound-tree.txt' && grep -q 'Waveform' '$OUT/sound-tree.txt' && grep -q 'button \"Play\"' '$OUT/sound-tree.txt'")" "a FLAC: its title and artist, the waveform and Play ($s)"
    check "$(yes_if "grep -q '0:01 · 44.1 kHz · 16-bit · stereo' '$OUT/sound-tree.txt'")" "its length, rate, depth and channels"
    shot MEM-1 sound >/dev/null
    check "$(yes_if "grep -q 'Cover art' '$OUT/sound-tree.txt' && grep -q 'drawing \"Waveform\"' '$OUT/sound-tree.txt'")" "its cover from the file, its waveform whole"
    vctl key Escape >/dev/null; sleep 1
    cp "$d/screen.png" "$d/Cover.png"
    pick long.wav
    p=$(vctl windows | grep 'kind=Dialog' | grep '"vergence-files"' | head -1 | cut -d' ' -f1)
    treeof long.wav > "$OUT/long-tree.txt"
    check "$(yes_if "grep -q 'button \"Pause\"' '$OUT/long-tree.txt'")" "a sound plays as soon as its preview opens (paced, silent output)"
    check "$(yes_if "grep -q 'drawing \"Waveform\"' '$OUT/long-tree.txt'")" "its waveform drawn whole while it plays"
    check "$(yes_if "grep -q 'Cover art' '$OUT/long-tree.txt'")" "the album's Cover.png beside it as its cover"
    shot MEM-1 playing >/dev/null
    vctl key Escape >/dev/null; sleep 1
}
