#!/bin/sh
# The agent interface end to end (meaning milestone 1, step 9): an MCP
# agent, through vergence-agent-mcp, working Edit, the settings app and
# FileView in a headless session of its own, the person's answers given by
# `vctl agent-answer` (a test session's: VERGENCE_CONTROL=1, no
# control-socket file); with the refusals: Don't allow, the quiet minute
# after it, a widget that changed, and agents turned off. Then terminals
# (terminal agent milestone 1): a terminal of the agent's own, typed in and
# read; a grant in one terminal covering nothing in another with the same tab
# ids; refusals in a root shell and with a remote shell in front; Escape twice.
#
#   sh tools/agent-check.sh            # on a dev box, from the checkout
#   sh tools/agent-check.sh terminal   # the terminal part only
#
# Builds into target-check (never the session's build). One line per check;
# exit 1 if any failed. The MCP conversation is kept in
# ~/.cache/vergence-lab/agent-check/<time>/.
set -u
ONLY=${1:-all}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
T=$ROOT/target-check/release
echo "building into target-check"
CARGO_TARGET_DIR=$ROOT/target-check cargo build -q --release --manifest-path "$ROOT/Cargo.toml" -p vergence-server -p vergence-protocol -p vergence-edit -p vergence-settings -p vergence-files -p vergence-reader -p vergence-pdf -p vergence-term || exit 2
OUT=${XDG_CACHE_HOME:-$HOME/.cache}/vergence-lab/agent-check/$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"
H=$HOME/.cache/vergence-headless/home

"$ROOT/tools/vergence-headless" stop >/dev/null 2>&1
rm -rf "$H/.local/state/vergence/edit" "$H/.local/state/vergence/agents.tsv"
[ -f "$H/.config/vergence/desktop.toml" ] && sed -i.bak '/^\[agents\]/,/^\[/{/^enabled/d;}' "$H/.config/vergence/desktop.toml"
# Agents are off by default (defaults milestone 1): on for these checks.
if grep -q '^\[agents\]' "$H/.config/vergence/desktop.toml" 2>/dev/null; then
    sed -i.bak '/^\[agents\]/a\
enabled = true
' "$H/.config/vergence/desktop.toml"
else
    printf '\n[agents]\nenabled = true\n' >>"$H/.config/vergence/desktop.toml"
fi
VERGENCE_SERVER=$T/vergence-server "$ROOT/tools/vergence-headless" start >/dev/null || { echo "FAIL no headless session"; exit 1; }
eval "$("$ROOT/tools/vergence-headless" env)"
vctl() { sh "$ROOT/tools/vctl" "$@"; }

FAILS=0
check() { # check <0|1> <what>
    if [ "$1" = 1 ]; then echo "ok   $2"; else echo "FAIL $2"; FAILS=$((FAILS + 1)); fi
}
yes_if() { if eval "$1"; then echo 1; else echo 0; fi; }

wait_title() {
    i=0
    while ! vctl windows | grep -q "\"$1"; do
        sleep 0.1; i=$((i + 1)); [ $i -gt 100 ] && return 1
    done
    sleep 0.8
}

# answer <word>: once the question has been up long enough to count.
answer() {
    i=0
    while ! vctl agents | grep -q '^asking'; do sleep 0.1; i=$((i + 1)); [ $i -gt 50 ] && return 1; done
    sleep 1.2
    vctl agent-answer "$1" >/dev/null
}

if [ "$ONLY" != terminal ]; then
vctl spawn "$T/vergence-edit" >/dev/null; wait_title Untitled || check 0 "Edit opens"
vctl spawn "$T/vergence-settings --page accessibility" >/dev/null; wait_title Accessibility || check 0 "the settings app opens"
vctl spawn "$T/vergence-files $HOME" >/dev/null; sleep 3
rm -f "$H/.local/state/vergence/reader-places"
vctl spawn "$T/vergence-reader $ROOT/docs/Vergence-Whitepaper.pdf" >/dev/null; sleep 3

# The agent: vergence-agent-mcp on a pipe; its replies, a line each, in a file.
mkfifo "$OUT/in"
"$T/vergence-agent-mcp" --name Claude <"$OUT/in" >"$OUT/replies.jsonl" 2>"$OUT/stderr.txt" &
MCP=$!
exec 3>"$OUT/in"
N=0
# call <tool> <arguments json>: the reply's line, after at most 15 s.
call() {
    N=$(($(cat "$OUT/n" 2>/dev/null || echo 0) + 1))
    echo "$N" >"$OUT/n"
    printf '{"jsonrpc":"2.0","id":"c%d","method":"tools/call","params":{"name":"%s","arguments":%s}}\n' "$N" "$1" "$2" >&3
    printf '%s %s\n' "$1" "$2" >>"$OUT/asked.txt"
    i=0
    while ! grep -q "\"id\":\"c$N\"" "$OUT/replies.jsonl"; do
        sleep 0.1; i=$((i + 1)); [ $i -gt 150 ] && { echo "(no reply)"; return; }
    done
    grep "\"id\":\"c$N\"" "$OUT/replies.jsonl" | head -1
}
printf '{"jsonrpc":"2.0","id":"i","method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"agent-check","version":"1"}}}\n' >&3
printf '{"jsonrpc":"2.0","id":"l","method":"tools/list"}\n' >&3
sleep 1
check "$(yes_if "grep -q '\"id\":\"l\".*\"act\"' '$OUT/replies.jsonl'")" "MCP: initialize, and the tools listed"

# Edit: its text set, asked first (Just now).
answer now & ANS=$!
r=$(call act '{"action":"set","path":"Untitled > text area:Untitled","value":"Hello from an agent"}')
wait $ANS
check "$(yes_if "echo '$r' | grep -q '\"isError\":false' && echo '$r' | grep -q 'Hello from an agent'")" "Edit: the text area set through MCP after Just now: $(echo "$r" | cut -c1-140)"
check "$(yes_if "vctl focused | grep -q 'Hello from an agent'")" "Edit shows the text"

# The settings app: Don't allow, then refused quietly for a minute.
answer deny & ANS=$!
r=$(call act '{"action":"press","path":"Accessibility > switch:Reduce motion"}')
wait $ANS
check "$(yes_if "echo '$r' | grep -q '\"isError\":true' && echo '$r' | grep -q 'did not allow'")" "Settings: Don't allow refuses: $(echo "$r" | cut -c1-140)"
r=$(call act '{"action":"press","path":"Accessibility > switch:Reduce motion"}')
check "$(yes_if "! vctl agents | grep -q '^asking' && echo '$r' | grep -q '\"isError\":true'")" "asked again at once: refused with no question"
check "$(yes_if "! grep -q 'reduced-motion = true' '$H/.config/vergence/desktop.toml'")" "the switch was not pressed"

# FileView: allowed for a time, then read and found in.
title=$(vctl windows | grep '"vergence-files"' | grep -v Desktop | head -1 | sed -E 's/^[0-9]+ native "([^"]*)".*/\1/')
answer hour & ANS=$!
r=$(call find "{\"path\":\"$title > button:*\"}")
wait $ANS
check "$(yes_if "[ -n '$title' ] && echo '$r' | grep -q '\"isError\":false' && echo '$r' | grep -q 'button'")" "FileView ($title): its buttons found after For an hour: $(echo "$r" | cut -c1-120)"
r=$(call windows '{}')
check "$(yes_if "echo '$r' | grep -q 'allowed for 60 more min'")" "the window list says what is allowed and for how long"

# The widget changed: an id kept from before acts on nothing else.
r=$(call act '{"action":"press","window":"1/1","node":"12345"}')
check "$(yes_if "echo '$r' | grep -q '\"isError\":true'")" "an id that names nothing now is refused: $(echo "$r" | cut -c1-120)"

# Reader: the page field set and pressed (Enter), where reading is read back.
rtitle=$(vctl windows | grep '"vergence-reader"' | head -1 | sed -E 's/^[0-9]+ native "([^"]*)".*/\1/')
answer hour & ANS=$!
r=$(call act "{\"action\":\"set\",\"path\":\"$rtitle > text field:Page number\",\"value\":\"3\"}")
wait $ANS
r=$(call act "{\"action\":\"press\",\"path\":\"$rtitle > text field:Page number\"}")
sleep 1
r=$(call find "{\"path\":\"$rtitle > drawing:Pages\"}")
check "$(yes_if "echo '$r' | grep -q 'Page 3 of'")" "Reader ($rtitle): page 3 by the page field, read back from the page view: $(echo "$r" | grep -o 'Page[s]* [0-9][^\\]*' | head -1 | cut -c1-60)"

# Agents turned off: everything refused, whatever was allowed.
sed -i.bak '/^\[agents\]/,/^\[/s/^enabled = true/enabled = false/' "$H/.config/vergence/desktop.toml"
sleep 1.6
r=$(call act '{"action":"set","path":"Untitled > text area:Untitled","value":"not this"}')
check "$(yes_if "echo '$r' | grep -q '\"isError\":true' && echo '$r' | grep -qi 'turned off'")" "agents off: refused though Edit was allowed: $(echo "$r" | cut -c1-120)"
sed -i.bak '/^\[agents\]/,/^\[/s/^enabled = false/enabled = true/' "$H/.config/vergence/desktop.toml"
sleep 1.6
fi

# Terminals (bus v39), through vagent: each run is its own connection, so the
# person's answers are "For an hour".
if [ "$ONLY" = all ] || [ "$ONLY" = terminal ]; then
P=2305843009214742529 # tab 1's pane, the same id in every vergence-term
va() { timeout 25 "$T/vagent" --name Claude "$@" 2>&1; }
term_index() { vctl windows | grep "$1" | head -1 | cut -d' ' -f1; }
vctl spawn "$T/vergence-term" >/dev/null
i=0; while ! vctl windows | grep -q '"vergence-term"'; do sleep 0.1; i=$((i + 1)); [ $i -gt 100 ] && break; done
sleep 1.5
answer hour & ANS=$!
r=$(va terminal open /tmp)
wait $ANS
set -- $(echo "$r" | sed -nE 's/^terminal ([0-9]+) ([0-9]+) tab.*/\1 \2/p')
OC=${1:-0} OW=${2:-0}
check "$(yes_if "[ $OC != 0 ]")" "a terminal of its own, after For an hour: $(echo "$r" | cut -c1-80)"
sleep 1.5
mark="agent-check-$$"
r=$(va type "$OC" "$OW" $P "echo $mark-typed\n")
sleep 1.5
check "$(yes_if "[ '$r' = ok ] && va read $OC $OW $P | grep -q '^$mark-typed'")" "typed in it, and the output read back: $r"

# The person's terminal: another program with the same tab and pane ids.
set -- $(va windows | grep '(vergence-term)' | grep -v " $OC/$OW " | head -1 | sed -E 's/^..([0-9]+)\/([0-9]+) .*/\1 \2/')
PC=${1:-0} PW=${2:-0}
answer deny & ANS=$!
r=$(va type "$PC" "$PW" $P "echo $mark-crossed\n")
wait $ANS
sleep 1
check "$(yes_if "[ $PC != 0 ] && echo '$r' | grep -q refused && ! vctl tree | grep -q '$mark-crossed'")" "its grant is nothing in the person's terminal (the same tab ids): asked, refused, nothing typed: $r"

# A root shell (sudo -s; or its password prompt where sudo asks): refused.
vctl focus "$(term_index "Claude's terminal")" >/dev/null
va type "$OC" "$OW" $P 'sudo -s\n' >/dev/null
sleep 2
r=$(va type "$OC" "$OW" $P 'id\n')
r2=$(va key "$OC" "$OW" $P ctrl+d)
check "$(yes_if "echo '$r' | grep -qE 'refused: (this tab runs as root|Terminal is asking for a password)' && echo '$r2' | grep -q refused")" "a root shell or sudo's password prompt: typing and keys refused: $r"
vctl focus "$(term_index "Claude's terminal")" >/dev/null; sleep 0.5
if echo "$r" | grep -q 'as root'; then vctl type exit >/dev/null; vctl key Return >/dev/null; else vctl key ctrl+c >/dev/null; fi
sleep 2

# A remote shell in front (a program named ssh): refused, keys too.
cp "$(command -v sleep)" "$OUT/ssh"
va type "$OC" "$OW" $P "$OUT/ssh 60\n" >/dev/null
sleep 2
r=$(va type "$OC" "$OW" $P 'yes\n')
r2=$(va key "$OC" "$OW" $P ctrl+c)
check "$(yes_if "echo '$r' | grep -q 'refused: this tab runs a program' && echo '$r2' | grep -q refused")" "ssh in front: typing and keys refused: $r"
vctl focus "$(term_index "Claude's terminal")" >/dev/null; sleep 0.5
vctl key ctrl+c >/dev/null; sleep 1.5
r=$(va type "$OC" "$OW" $P "echo $mark-after\n")
check "$(yes_if "[ '$r' = ok ]")" "typing again once the person ended it: $r"

# Escape twice in the tab: the grant ends, and the agent is told why.
sleep 1
vctl focus "$(term_index "Claude's terminal")" >/dev/null; sleep 0.5
vctl key Escape >/dev/null; vctl key Escape >/dev/null; sleep 1
r=$(va type "$OC" "$OW" $P "echo $mark-late\n")
check "$(yes_if "echo '$r' | grep -q 'stopped you'")" "Escape twice: refused after: $r"
log=$H/.local/state/vergence/agents.log
check "$(yes_if "grep -q 'act [0-9]*: acted' '$log' && grep -q 'act [0-9]*: refused: this tab runs a program' '$log'")" "agents.log has the window's answers by act number"
fi

# The log has it all.
log=$H/.local/state/vergence/agents.log
[ "$ONLY" = terminal ] || check "$(yes_if "grep -q 'asked (.*): Just now' '$log' && grep -q \"asked (.*): Don't allow\" '$log' && grep -q 'Set' '$log'")" "agents.log has the answers and the action"
cp "$log" "$OUT/agents.log" 2>/dev/null

exec 3>&-
kill ${MCP:-} 2>/dev/null
vctl shot >/dev/null; sleep 2; cp "$H"/.cache/vergence-lab/shots/*.png "$OUT/" 2>/dev/null
"$ROOT/tools/vergence-headless" stop >/dev/null 2>&1
echo "$FAILS failed; the conversation in $OUT"
[ "$FAILS" -eq 0 ]
