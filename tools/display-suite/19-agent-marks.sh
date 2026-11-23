# Terminal agent milestone 1: the tab marks and the pill. A tab an agent
# holds any grant for is marked in the agent's color for as long as the grant
# lasts, that tab only (Dave: "this tab only, with a visual"): an outline for
# reading, a tint and a band for typing; the pane shows "Claude is typing ·
# stop" while it types. Two terminals, the person's (a read grant) and the
# agent's own (typing); `vctl agents` lists the marks the bars draw; Escape
# twice in the person's tab takes its mark away. The shots are for the eye.
scenario() {
    headless "1920x1080 531x299@60" || { check 0 "headless session"; return; }
    home=$HOME/.cache/vergence-headless/home
    mkdir -p "$home/.config/vergence"
    printf '[agents]\nenabled = true\n' > "$home/.config/vergence/desktop.toml"
    rm -f "$home/.local/state/vergence/agents.tsv"
    sleep 1.6
    va() { timeout 25 "$TARGET/vagent" --name Claude "$@" 2>&1; }
    answer() { # answer <word>: once the question has been up long enough to count
        i=0
        while ! vctl agents | grep -q '^asking'; do sleep 0.1; i=$((i + 1)); [ $i -gt 80 ] && return 1; done
        sleep 1.3
        vctl agent-answer "$1" >/dev/null
    }
    idx() { vctl windows | grep "$1" | head -1 | cut -d' ' -f1; }
    P=2305843009214742529 # tab 1's pane
    vctl spawn "$TARGET/vergence-term" >/dev/null
    i=0; while ! vctl windows | grep -q '"vergence-term"'; do sleep 0.1; i=$((i + 1)); [ $i -gt 100 ] && break; done
    sleep 1.5
    vctl move "$(idx '"vergence-term"')" 80 120 >/dev/null
    answer hour & ANS=$!
    r=$(va terminal open /tmp)
    wait $ANS
    set -- $(echo "$r" | sed -nE 's/^terminal ([0-9]+) ([0-9]+) tab.*/\1 \2/p')
    OC=${1:-0} OW=${2:-0}
    check "$(yes_if "[ $OC != 0 ]")" "the agent's own terminal ($r)"
    sleep 1.5
    vctl move "$(idx "Claude's terminal")" 1000 120 >/dev/null
    set -- $(va windows | grep '(vergence-term)' | grep -v " $OC/$OW " | head -1 | sed -E 's/^..([0-9]+)\/([0-9]+) .*/\1 \2/')
    PC=${1:-0} PW=${2:-0}
    vctl agents > "$OUT/agents-before.txt"
    check "$(yes_if "! grep -q 'mark: window $PC/$PW ' '$OUT/agents-before.txt'")" "the person's terminal: no mark before any grant"
    shot MEM-1 before >/dev/null

    # A read grant in the person's tab: an outline, reading.
    answer hour & ANS=$!
    va tree "$PC" "$PW" > "$OUT/person-tree.txt"
    wait $ANS
    sleep 1
    vctl agents > "$OUT/agents-read.txt"
    check "$(yes_if "grep -q 'mark: window $PC/$PW tab #[0-9]* \"Claude\" reading' '$OUT/agents-read.txt'")" "a read grant: the person's tab marked, reading"
    check "$(yes_if "grep -q 'mark: window $OC/$OW tab #[0-9]* \"Claude\" typing' '$OUT/agents-read.txt'")" "the agent's own tab marked, typing"
    check "$(yes_if "[ \$(grep -c 'mark: window $PC/$PW ' '$OUT/agents-read.txt') = 1 ]")" "one mark in the person's window: that tab only"

    # Typing in its own: the pill while it types and for a moment after.
    r=$(va type "$OC" "$OW" $P 'echo the agent types here\n')
    check "$(yes_if "[ '$r' = ok ]")" "typed in its own terminal ($r)"
    shot MEM-1 marks >/dev/null
    echo "  (look: the person's tab outlined, the agent's tab tinted with a band, the pill in its pane)"

    # Escape twice in the person's tab: the grant and its mark go.
    vctl focus "$(idx '"Terminal" "vergence-term"')" >/dev/null; sleep 0.6
    vctl key Escape >/dev/null; vctl key Escape >/dev/null; sleep 1.2
    vctl agents > "$OUT/agents-after.txt"
    check "$(yes_if "! grep -q 'mark: window $PC/$PW ' '$OUT/agents-after.txt' && grep -q 'mark: window $OC/$OW ' '$OUT/agents-after.txt'")" "Escape twice: the person's mark gone, the agent's own kept"
    shot MEM-1 after-escape >/dev/null
}
