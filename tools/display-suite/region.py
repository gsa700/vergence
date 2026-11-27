#!/usr/bin/env python3
# How much a region of two pictures differs (the display suite's: a scroll bar
# wider than it was, a mark that breathes). Standard library only: reads the
# PNGs the server writes (8-bit RGB or RGBA, not interlaced).
#
#   python3 region.py A.png B.png X Y W H     -> the mean difference per channel, 0-255
#   python3 region.py A.png - X Y W H         -> the region's mean alpha-weighted brightness
import struct, sys, zlib


def read(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", path
    at, idat, w, h, kind = 8, b"", 0, 0, 0
    while at < len(data):
        n, t = struct.unpack(">I4s", data[at:at + 8])
        body = data[at + 8:at + 8 + n]
        if t == b"IHDR":
            w, h, depth, kind, _, _, lace = struct.unpack(">IIBBBBB", body)
            assert depth == 8 and kind in (2, 6) and lace == 0, (path, depth, kind, lace)
        elif t == b"IDAT":
            idat += body
        at += 12 + n
    bpp = 4 if kind == 6 else 3
    raw, stride = zlib.decompress(idat), w * bpp
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        prev = line
    return w, h, bpp, rows


def region(pic, x, y, w, h):
    pw, ph, bpp, rows = pic
    out = []
    for yy in range(max(0, y), min(ph, y + h)):
        for xx in range(max(0, x), min(pw, x + w)):
            px = rows[yy][xx * bpp:(xx + 1) * bpp]
            out.append(tuple(px[:3]) + ((px[3],) if bpp == 4 else (255,)))
    return out


if __name__ == "__main__":
    a = read(sys.argv[1])
    x, y, w, h = (int(v) for v in sys.argv[3:7])
    ra = region(a, x, y, w, h)
    if sys.argv[2] == "-":
        print(round(sum((p[0] + p[1] + p[2]) / 3 * p[3] / 255 for p in ra) / max(1, len(ra)), 1))
    else:
        rb = region(read(sys.argv[2]), x, y, w, h)
        d = sum(abs(p - q) for pa, pb in zip(ra, rb) for p, q in zip(pa, pb)) / max(1, len(ra) * 4)
        print(round(d, 2))
