# The toolkit on the GPU against its CPU reference: the rich test pattern
# (text, rounded shapes, outlines, hairlines) drawn both ways at several
# scales, the two pictures alike within one step (rounding).
scenario() {
    headless "2560x1440 597x336@60" || { check 0 "headless session"; return; }
    for s in 1 1.3333333333 2; do
        vctl scale MEM-1 "$s" >/dev/null
        sleep 1
        pattern GPU --rich --size 420 260 || check 0 "GPU window opens"
        vctl spawn "VERGENCE_RENDER=cpu $PATTERN --title CPU --rich --size 420 260" >/dev/null
        i=0
        while [ -z "$(win_line CPU)" ] && [ $i -lt 80 ]; do sleep 0.1; i=$((i + 1)); done
        sleep 0.5
        g=$(picture GPU "gpu-$s")
        c=$(picture CPU "cpu-$s")
        r=$("$HARNESS" same "$c" "$g" --tolerance 1)
        echo "  $r"
        check "$(yes_if "echo '$r' | grep -q '^PASS'")" "GPU and CPU alike at $s"
        pkill -x testpattern
        sleep 0.7
    done
}
