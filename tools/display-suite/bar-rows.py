#!/usr/bin/env python3
# A bar measured in a screen shot (the display suite's): the color a few rows
# above a window's top at x, and how many rows up from that top stay the
# ground (or its rule), within 3 of it per channel. Standard library only.
#
#   python3 bar-rows.py SHOT.png X TOP RRGGBB RULE  -> "rrggbb rows"
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from region import read  # noqa: E402

w, h, bpp, rows = read(sys.argv[1])
x, top = int(sys.argv[2]), int(sys.argv[3])
ground, rule = (tuple(int(c[i:i + 2], 16) for i in (0, 2, 4)) for c in sys.argv[4:6])


def px(y):
    return tuple(rows[y][x * bpp:x * bpp + 3])


def near(p, q):
    return all(abs(a - b) <= 3 for a, b in zip(p, q))


y, n = top - 1, 0
while y >= 0 and (near(px(y), ground) or near(px(y), rule)):
    n, y = n + 1, y - 1
print("%02x%02x%02x %d" % (px(max(0, top - 6)) + (n,)))
