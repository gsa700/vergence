# Session milestone 1, step 5: back as it was after the app is ended from
# outside, fresh after the user closes it. Reader ended (killed) at a page
# of the whitepaper comes back there; its tab closed and the file opened
# again starts at page 1. Settings killed on a scrolled page comes back on
# it, scrolled; Settings > Session shows its rows.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    state=$HOME/.cache/vergence-headless/home/.local/state/vergence
    rm -f "$state/reader-session" "$state/reader-places" "$state/settings-page"
    doc=$ROOT/docs/Vergence-Whitepaper.pdf
    pid_of() { vctl windows | grep "\"$1\"" | head -1 | sed -E 's/.* pid ([0-9]+).*/\1/'; }
    wait_for() { i=0; while ! vctl windows | grep -q "\"$1\""; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done; sleep 2; }
    gone() { i=0; while vctl windows | grep -q "\"$1\""; do sleep 0.2; i=$((i + 1)); [ $i -gt 25 ] && break; done; }
    pages() { vctl find "* > drawing:Pages" | sed -E 's/.*= "([^"]*)".*/\1/'; }

    # Reader: page down into the document, then ended from outside.
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-reader $doc" >/dev/null
    wait_for vergence-reader
    vctl focus "$(vctl windows | grep '"vergence-reader"' | cut -d' ' -f1)" >/dev/null
    n=0; while [ $n -lt 12 ]; do vctl key Page_Down >/dev/null; n=$((n + 1)); done
    sleep 2
    before=$(pages | sed -E 's/^(Pages? [0-9]+).*/\1/')
    check "$(yes_if "[ '$before' != 'Page 1' ] && [ -n '$before' ]")" "Reader paged into the whitepaper: $before"
    sleep 1
    kill -9 "$(pid_of vergence-reader)" 2>/dev/null; gone vergence-reader
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-reader" >/dev/null
    wait_for vergence-reader
    after=$(pages | sed -E 's/^(Pages? [0-9]+).*/\1/')
    check "$(yes_if "[ '$after' = '$before' ]")" "ended from outside, Reader comes back at the same page: $after, $before wanted"

    # Closed by the user: the file opened again starts at its first page.
    vctl focus "$(vctl windows | grep '"vergence-reader"' | cut -d' ' -f1)" >/dev/null
    vctl key ctrl+w >/dev/null; gone vergence-reader
    vctl spawn "env PATH=$TARGET:/usr/local/bin:/usr/bin:/bin $TARGET/vergence-reader $doc" >/dev/null
    wait_for vergence-reader
    p=$(pages)
    check "$(yes_if "echo '$p' | grep -q '^Page 1 of'")" "closed by the user, the document opens again at page 1: $p"
    kill -9 "$(pid_of vergence-reader)" 2>/dev/null; gone vergence-reader

    # Settings: the Windows page (taller than the window) scrolled to its
    # end, then ended from outside.
    vctl spawn "$TARGET/vergence-settings --page windows" >/dev/null
    wait_for vergence-settings
    w=$(win_index Windows)
    last=$(vctl tree "$w" | grep -E '^ +#' | tail -1 | sed -E 's/^ +#([0-9]+).*/\1/')
    vctl act "$w" "$last" reveal >/dev/null; sleep 2.5
    kept=$(grep '^scroll ' "$state/settings-page" 2>/dev/null | cut -d' ' -f2)
    check "$(yes_if "[ -n '$kept' ]")" "Settings keeps its page and how far it is scrolled: scroll $kept"
    kill -9 "$(pid_of vergence-settings)" 2>/dev/null; gone vergence-settings
    vctl spawn "$TARGET/vergence-settings" >/dev/null
    wait_for vergence-settings
    title=$(vctl windows | grep '"vergence-settings"' | head -1 | sed -E 's/^[0-9]+ [a-z0-9]+ "([^"]*)".*/\1/')
    y=$(vctl tree "$(win_index Windows)" | grep -E '^ +#' | tail -1 | sed -E 's/.* at -?[0-9]+,(-?[0-9]+) .*/\1/')
    check "$(yes_if "[ '$title' = Windows ] && [ '${y:-9999}' -lt 1080 ]")" "ended from outside, Settings comes back on its page, scrolled: \"$title\", the last row at y $y"
    kill -9 "$(pid_of vergence-settings)" 2>/dev/null; gone vergence-settings

    # Settings > Session.
    vctl spawn "$TARGET/vergence-settings --page session" >/dev/null
    wait_for vergence-settings
    vctl tree "$(win_index Session)" > "$OUT/session-tree.txt"
    t=$OUT/session-tree.txt
    check "$(yes_if "grep -q 'choice \"When Vergence starts\"' '$t' && grep -q 'switch \"Return to where I was in documents\"' '$t' && grep -q 'choice \"Remember the page after closing\" = \"30 minutes\"' '$t'")" "Settings > Session: when Vergence starts, places in documents, how long Settings remembers its page"
    shot MEM-1 session-page >/dev/null
}
