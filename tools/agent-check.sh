#!/bin/sh
# The agent interface end to end (meaning milestone 1, step 9): an MCP
# agent, through vergence-agent-mcp, working Edit, the settings app and
# FileView in a headless session of its own, the person's answers given by
# `vctl agent-answer` (a test session's: VERGENCE_CONTROL=1, no
# control-socket file); with the refusals: Don't allow, the quiet minute
# after it, a widget that changed, and agents turned off.
#
#   sh tools/agent-check.sh            # on a dev box, from the checkout
#
# Builds into target-check (never the session's build). One line per check;
# exit 1 if any failed. The MCP conversation is kept in
# ~/vergence-agent-check/<time>/.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
T=$ROOT/target-check/release
echo "building into target-check"
CARGO_TARGET_DIR=$ROOT/target-check cargo build -q --release --manifest-path "$ROOT/Cargo.toml" -p vergence-server -p vergence-protocol -p vergence-edit -p vergence-settings -p vergence-files -p vergence-reader -p vergence-pdf || exit 2
OUT=$HOME/vergence-agent-check/$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"
H=$HOME/.cache/vergence-headless/home

"$ROOT/tools/vergence-headless" stop >/dev/null 2>&1
rm -rf "$H/.local/state/vergence/edit" "$H/.local/state/vergence/agents.tsv"
[ -f "$H/.config/vergence/desktop.toml" ] && sed -i.bak '/^\[agents\]/,/^\[/{/^enabled/d;}' "$H/.config/vergence/desktop.toml"
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
# answer <word>: once the question has been up long enough to count.
answer() {
    i=0
    while ! vctl agents | grep -q '^asking'; do sleep 0.1; i=$((i + 1)); [ $i -gt 50 ] && return 1; done
    sleep 1.2
    vctl agent-answer "$1" >/dev/null
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
printf '\n[agents]\nenabled = false\n' >>"$H/.config/vergence/desktop.toml"
sleep 1.6
r=$(call act '{"action":"set","path":"Untitled > text area:Untitled","value":"not this"}')
check "$(yes_if "echo '$r' | grep -q '\"isError\":true' && echo '$r' | grep -qi 'turned off'")" "agents off: refused though Edit was allowed: $(echo "$r" | cut -c1-120)"
sed -i.bak '/^\[agents\]/,$d' "$H/.config/vergence/desktop.toml"

# The log has it all.
log=$H/.local/state/vergence/agents.log
check "$(yes_if "grep -q 'asked (.*): Just now' '$log' && grep -q \"asked (.*): Don't allow\" '$log' && grep -q 'Set' '$log'")" "agents.log has the answers and the action"
cp "$log" "$OUT/agents.log" 2>/dev/null

exec 3>&-
kill $MCP 2>/dev/null
vctl shot >/dev/null; sleep 2; cp "$H"/vergence-shots/*.png "$OUT/" 2>/dev/null
"$ROOT/tools/vergence-headless" stop >/dev/null 2>&1
echo "$FAILS failed; the conversation in $OUT"
[ "$FAILS" -eq 0 ]
