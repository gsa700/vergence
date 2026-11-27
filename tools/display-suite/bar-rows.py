#!/usr/bin/env python3
# A bar measured in a screen shot (the display suite's). Standard library only.
#
#   python3 bar-rows.py SHOT.png X TOP
#       -> "rrggbb rows": the bar's ground (a few rows above the window's top
#          at x), and how many rows up from that top are the bar: its rule (the
#          row just above the window, if it differs) and then rows within 3 of
#          the ground per channel.
#   python3 bar-rows.py SHOT.png chroma X Y W H
#       -> the most colorful pixel's chroma in the region (max - min channel,
#          0-255): a tinted letter on a gray bar shows as a high one.
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from region import read  # noqa: E402

w, h, bpp, rows = read(sys.argv[1])


def px(x, y):
    return tuple(rows[y][x * bpp:x * bpp + 3])


if sys.argv[2] == "chroma":
    x0, y0, rw, rh = (int(v) for v in sys.argv[3:7])
    print(max((max(px(x, y)) - min(px(x, y)) for y in range(max(0, y0), min(h, y0 + rh))
               for x in range(max(0, x0), min(w, x0 + rw))), default=0))
    sys.exit()

x, top = int(sys.argv[2]), int(sys.argv[3])
ground = px(x, max(0, top - 6))


def near(p, q):
    return all(abs(a - b) <= 3 for a, b in zip(p, q))


y, n = top - 1, 0
if y >= 0 and not near(px(x, y), ground):
    n, y = 1, y - 1  # the rule
while y >= 0 and near(px(x, y), ground):
    n, y = n + 1, y - 1
print("%02x%02x%02x %d" % (ground + (n,)))
