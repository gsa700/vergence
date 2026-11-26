# Frost milestone 1: frosted glass behind a see-through terminal
# (docs/frost-milestone-1.md). The terminal at 80% over a bright test pattern:
# with Frosted glass on, the pattern seen through it is blurred (far less
# sharp than with it off); while another window is in front, the terminal's
# dark ground is more opaque (less of the bright pattern shows through) than
# while it is the window in front. Then its text, run up under the bar: the
# strip under the glass bar changes with it (the server blurs it there and
# keeps the bar see-through; the sharp lines are not drawn under the bar).
scenario() {
    cfg=$HOME/.cache/vergence-headless/home/.config/vergence/desktop.toml
    # look <true|false> <name>: a fresh session; the pattern under the
    # terminal; shots with the terminal in front, then with a small window
    # elsewhere in front.
    look() {
        headless "1920x1080 531x299@60" || return 1
        kept=$(cat "$cfg")
        printf '\n[terminal]\nopacity = 0.8\nfrosted = %s\n' "$1" >> "$cfg"
        pattern P --size 900 600 || return 1
        move P 100 100
        pattern Q --size 300 200 || return 1
        move Q 1450 800
        vctl spawn "$TARGET/vergence-term" >/dev/null
        i=0; while ! vctl windows | grep -q '"vergence-term"'; do sleep 0.2; i=$((i + 1)); [ $i -gt 50 ] && break; done
        sleep 2
        t=$(vctl windows | grep '"vergence-term"' | head -1 | cut -d' ' -f1)
        vctl move "$t" 300 250 >/dev/null
        sleep 0.8
        t=$(vctl windows | grep '"vergence-term"' | head -1 | cut -d' ' -f1)
        vctl focus "$t" >/dev/null
        sleep 1.5
        shot MEM-1 "$2-front" >/dev/null
        vctl focus "$(win_index Q)" >/dev/null
        sleep 1.5
        shot MEM-1 "$2-behind" >/dev/null
        # Its output scrolled up under the bar: a screenful and more of lines.
        t=$(vctl windows | grep '"vergence-term"' | head -1 | cut -d' ' -f1)
        vctl focus "$t" >/dev/null
        sleep 0.5
        vctl type "seq 100000 100300" >/dev/null
        vctl key Return >/dev/null
        sleep 2
        shot MEM-1 "$2-text" >/dev/null
        vctl windows > "$OUT/$2-windows.txt"
        printf '%s\n' "$kept" > "$cfg"
    }
    look true frosted || { check 0 "frosted session"; return; }
    look false clear || { check 0 "clear session"; return; }
    # Inside the terminal's body over the pattern's bottom-right corner: its
    # bright color against the dark desktop (an edge to blur, and enough
    # brightness behind for the ground's opacity to show).
    region="880 560 170 190"
    r=$("$HARNESS" sharper "$OUT/clear-front.png" "$OUT/frosted-front.png" $region --by 1.5)
    echo "  .. sharpness, clear then frosted: $r"
    # Frosted, the pattern's edges turn to slopes, which the measure does not
    # count as edges at all: blurred is the clear shot's edges with a quarter
    # or fewer left (or the measure's own verdict when both have some).
    case "$r" in
        PASS*) blurred=1 ;;
        *"almost no edges"*)
            set -- $(echo "$r" | sed -E 's/.*\(([0-9]+), ([0-9]+)\).*/\1 \2/')
            if [ "${1:-0}" -ge 1000 ] && [ $((${2:-0} * 4)) -lt "${1:-0}" ]; then blurred=1; else blurred=0; fi
            ;;
        *) blurred=0 ;;
    esac
    check "$blurred" "frosted: the pattern behind is blurred"
    front=$("$HARNESS" colors "$OUT/frosted-front.png" $region | awk '{print $6}')
    behind=$("$HARNESS" colors "$OUT/frosted-behind.png" $region | awk '{print $6}')
    echo "  .. brightness through the frosted terminal: in front $front, behind $behind"
    check "$(yes_if "awk -v a=\"$front\" -v b=\"$behind\" 'BEGIN { exit !(a > b + 2) }'")" \
        "frosted: clearer while in front (less ground over what is behind)"
    # The strip under the terminal's bar, left of its buttons and clear of the
    # title: its lines show there, blurred (look at frosted-text.png).
    set -- $(grep '"vergence-term"' "$OUT/frosted-windows.txt" | head -1 | sed -E 's/.*\{ x: (-?[0-9]+), y: (-?[0-9]+), width: ([0-9]+), height: ([0-9]+) \}.* bar-h=([0-9]+).*/\1 \2 \3 \4 \5/')
    strip="$(($1 + 12)) $(($2 - $5 + 4)) 120 $(($5 - 8))"
    sa=$("$HARNESS" colors "$OUT/frosted-front.png" $strip | awk '{print $6}')
    sb=$("$HARNESS" colors "$OUT/frosted-text.png" $strip | awk '{print $6}')
    echo "  .. the strip under the glass bar ($strip), before and with text: $sa, $sb"
    check "$(yes_if "! near \"$sa\" \"$sb\" 1.0")" "frosted: the text runs up under the glass bar"
    cf=$("$HARNESS" colors "$OUT/clear-front.png" $region | awk '{print $6}')
    cb=$("$HARNESS" colors "$OUT/clear-behind.png" $region | awk '{print $6}')
    check "$(yes_if "awk -v a=\"$cf\" -v b=\"$cb\" 'BEGIN { d = a - b; if (d < 0) d = -d; exit !(d < 1.5) }'")" \
        "frost off: focus changes nothing ($cf, $cb)"
}
