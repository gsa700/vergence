# Reader milestone 1, step 7: the PDF app in a headless session, on the
# repository's own PDF (the whitepaper): the window and where reading is
# (its meaning), two pages side by side, the night look (the screen
# differs, and comes back the same without it), the sidebar, a second file
# in a tab of the same window, a file replaced on disk read again, and
# Close-Up handing a PDF to Reader.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    rm -f "$HOME/.cache/vergence-headless/home/.local/state/vergence/reader-places" "$HOME/.cache/vergence-headless/home/.local/state/vergence/reader-session"
    doc=$ROOT/docs/Vergence-Whitepaper.pdf
    small=$OUT/small.pdf
    # A one-page PDF of its own (the reader repairs its offsets).
    printf '%%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]/Contents 4 0 R>>endobj\n4 0 obj<</Length 27>>stream\n0 0 1 rg 20 20 260 160 re f\nendstream endobj\ntrailer<</Root 1 0 R>>\n%%%%EOF\n' > "$small"
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-reader $doc" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-reader"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
    sleep 2
    w=$(vctl windows | grep '"vergence-reader"' | head -1 | cut -d' ' -f1)
    pages() { vctl find "* > drawing:Pages" | sed -E 's/.*= "([^"]*)".*/\1/'; }
    press() { n=$(vctl find "* > button:$1" | head -1 | sed -E 's/.*#([0-9]+).*/\1/'); vctl act "$w" "$n" press >/dev/null; sleep 1.2; }
    p=$(pages)
    check "$(yes_if "echo '$p' | grep -q '^Page 1 of [0-9]*, continuous'")" "Reader opens the whitepaper, saying where reading is: $p"
    press "Two pages"
    press "Next page"
    p=$(pages)
    check "$(yes_if "echo '$p' | grep -q '^Pages 2 and 3 of .*two pages'")" "two pages side by side after the cover: $p"
    vctl pointer 2 2 >/dev/null; sleep 1.5
    day=$(shot MEM-1 two-pages)
    vctl key n >/dev/null; sleep 2
    night=$(shot MEM-1 night)
    r=$("$HARNESS" same "$day" "$night" --tolerance 8 | head -1)
    n=$(echo "$r" | sed -E 's/.*: ([0-9]+) of .*/\1/')
    check "$(yes_if "[ '$n' -gt 100000 ]")" "the night look changes the pages: $n pixels differ"
    vctl key n >/dev/null; sleep 2
    again=$(shot MEM-1 day-again)
    r=$("$HARNESS" same "$day" "$again" --tolerance 2 | head -1)
    n=$(echo "$r" | sed -E 's/.*: ([0-9]+) of .*/\1/')
    check "$(yes_if "echo '$r' | grep -q '^PASS' || [ '$n' -lt 600 ]")" "without it the screen is as it was (the dock's live tiles aside): $(echo "$r" | cut -c1-60)"
    press "Sidebar"
    check "$(yes_if "vctl find '* > drawing:Page thumbnails' | grep -q 'marked'")" "the sidebar shows the pages' thumbnails"
    # A second file: a tab of the same window.
    vctl spawn "$TARGET/vergence-reader $small" >/dev/null; sleep 3
    count=$(vctl windows | grep -c '"vergence-reader"')
    p=$(pages)
    check "$(yes_if "[ '$count' = 1 ] && echo '$p' | grep -q '^Page 1 of 1,'")" "a second file opens in a tab of the same window: $count window, $p"
    # Replaced on disk: read again.
    cp "$doc" "$small"; sleep 4
    p=$(pages)
    check "$(yes_if "echo '$p' | grep -qv 'of 1,'")" "a file replaced on disk is read again: $p"
    # Close-Up hands a PDF to Reader.
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-closeup $doc" >/dev/null; sleep 4
    check "$(yes_if "! vctl windows | grep -q '\"vergence-closeup\"' && [ \$(vctl windows | grep -c '\"vergence-reader\"') = 1 ]")" "Close-Up given a PDF hands it to Reader"
    shot MEM-1 end >/dev/null
}
