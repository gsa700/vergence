# Text on a scaled screen: a 27" 4K (calculated 1.69) at 133% and at its
# default 200% tells its windows text 1.0x. Until 2026-11-19 a text factor
# made it 1.06x and 0.96x ("Text on scaled screens", removed: each screen's
# scale and Accessibility's Size are the controls).
scenario() {
    headless "3840x2160 600x340@60" || { check 0 "headless session"; return; }
    vctl scale MEM-1 1.3333333333 >/dev/null
    sleep 1
    pattern T || check 0 "window opens"
    sleep 0.5
    t=$(win_field T text)
    check "$(yes_if "near '$(win_field T scale)' 1.3333 0.001")" "scale told 1.3333: $(win_field T scale)"
    check "$(yes_if "near '$t' 1.0 0.001")" "text told 1.0x at 133%: $t"
    vctl scale MEM-1 2 >/dev/null
    sleep 1.5
    t=$(win_field T text)
    check "$(yes_if "near '$t' 1.0 0.001")" "text told 1.0x at 200%: $t"
}
