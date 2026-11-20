#!/bin/bash
set -e
# Makes the JPEG 2000 test files (jpx.rs tests) from synthetic pictures with
# OpenJPEG's tools (opj_compress, opj_decompress; on Hubble only, never
# shipped). The pictures are made here, nothing of anyone's.
mkdir -p ~/jpxfix && cd ~/jpxfix && rm -f -- *
python3 - <<'EOF'
import math
W,H=67,45
rgb=bytearray()
for y in range(H):
    for x in range(W):
        r=(x*255)//(W-1); g=(y*255)//(H-1); b=int(127+120*math.sin(x*0.3)*math.cos(y*0.4))
        if (x//8+y//8)%2==0 and x>40: r,g,b=255-r,b,g
        rgb+=bytes([r,g,b])
open("rgb.ppm","wb").write(b"P6\n%d %d\n255\n"%(W,H)+rgb)
W2,H2=33,21
g=bytes(((x*x+y*13)%256) if x<20 else (255 if (x+y)%3 else 0) for y in range(H2) for x in range(W2))
open("gray.pgm","wb").write(b"P5\n%d %d\n255\n"%(W2,H2)+g)
W3,H3=29,19
g12=bytearray()
for y in range(H3):
    for x in range(W3):
        v=(x*141+y*97+(x*y)%37)%4096
        g12+=v.to_bytes(2,"big")
open("g12.pgm","wb").write(b"P5\n%d %d\n4095\n"%(W3,H3)+g12)
EOF
q() { opj_compress "$@" >/dev/null 2>&1; out="$4"; [ -s "$out" ] || { echo "FAILED: $*"; exit 1; }; }
q -i rgb.ppm -o rgb-plain.j2k
q -i rgb.ppm -o rgb-tiles.j2k -t 32,16 -n 3 -c [32,32],[16,16],[8,8] -p RPCL -SOP -EPH -r 30,10,1
q -i rgb.ppm -o rgb-modes.j2k -M 63 -b 16,16 -n 3
q -i rgb.ppm -o rgb-pcrl.j2k -p PCRL -n 2 -c [16,16] -r 8,1
q -i rgb.ppm -o rgb-cprl.j2k -p CPRL -t 40,30 -n 4 -d 5,3 -T 2,1
q -i rgb.ppm -o rgb-rlcp.j2k -p RLCP -n 5 -r 5,2,1
q -i rgb.ppm -o rgb-nl0.j2k -n 1
q -i rgb.ppm -o rgb.jp2
q -i rgb.ppm -o rgb-lossy.j2k -I -r 8
q -i rgb.ppm -o rgb-lossy-layers.jp2 -I -r 40,15,6 -b 32,32 -M 9
q -i gray.pgm -o gray-sub.j2k -s 2,2 -n 3
q -i gray.pgm -o gray-bypass.j2k -M 1 -b 8,8 -n 3
q -i g12.pgm -o g12.j2k -n 4
q -i g12.pgm -o g12-lossy.j2k -n 4 -I -r 6
opj_decompress -i rgb-lossy.j2k -o rgb-lossy.out.ppm >/dev/null
opj_decompress -i rgb-lossy-layers.jp2 -o rgb-lossy-layers.out.ppm >/dev/null
opj_decompress -i g12-lossy.j2k -o g12-lossy.out.pgm >/dev/null
opj_decompress -i gray-sub.j2k -o gray-sub.out.pgm >/dev/null
ls -l
