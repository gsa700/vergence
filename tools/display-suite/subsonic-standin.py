#!/usr/bin/env python3
# A stand-in Subsonic (Navidrome) server for the display suite (AlbumWall
# milestone 2, step 7): user "dave", password "sesame"; two music folders,
# 24 albums of 3 songs in "Lossless", 6 in "Lossy"; covers as PNGs (album 0
# wide, album 1 nearly square); song 1 has timed lyrics, song 2 plain ones;
# songs are WAV_SECONDS long and each stream answers half a second late.
#
#   python3 subsonic-standin.py PORT [MODE]
#
# MODE: "good" (the default), or "garbage": every answer is a page of HTML
# with status 200, as a misconfigured proxy gives. Each stream asked for is
# printed ("stream <id> <format>"), for the scenario to read.
# Written for the suite; it serves on 127.0.0.1 only.
import hashlib, json, math, os, struct, sys, time, zlib
SECONDS = int(os.environ.get("WAV_SECONDS", "6"))
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse, parse_qs

def wav(freq, seconds=None, rate=44100):
    seconds = seconds or SECONDS
    n = rate * seconds
    frames = b"".join(struct.pack("<hh", v, v) for v in (int(8000 * math.sin(2 * math.pi * freq * i / rate)) for i in range(n)))
    return b"RIFF" + struct.pack("<I", 36 + len(frames)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 2, rate, rate * 4, 4, 16) + b"data" + struct.pack("<I", len(frames)) + frames

WAVS = {}
STREAMS = []

def png(rgb, side=300, tall=None):
    tall = tall or side
    row = b"\0" + bytes(rgb) * side
    def chunk(k, b): return struct.pack(">I", len(b)) + k + b + struct.pack(">I", zlib.crc32(k + b) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", side, tall, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(row * tall)) + chunk(b"IEND", b"")

ALBUMS = {"1": [(i, f"Server Album {i:02d}", f"Artist {i // 4}", 1980 + i) for i in range(24)],
          "2": [(100 + i, f"Lossy Album {i}", "Lossy Artist", 2000 + i) for i in range(6)]}

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, body, kind="application/json"):
        self.send_response(200); self.send_header("Content-Type", kind); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def ok(self, extra): self.send(json.dumps({"subsonic-response": dict(status="ok", version="1.16.1", **extra)}).encode())
    def do_GET(self):
        u = urlparse(self.path); q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if MODE == "garbage":
            return self.send(b"<html><body><h1>Welcome to nginx!</h1></body></html>", "text/html")
        if q.get("u") != "dave" or q.get("t") != hashlib.md5(("sesame" + q.get("s", "")).encode()).hexdigest():
            return self.send(json.dumps({"subsonic-response": {"status": "failed", "error": {"code": 40, "message": "Wrong username or password"}}}).encode())
        call = u.path.rsplit("/", 1)[-1]
        folders = [q["musicFolderId"]] if "musicFolderId" in q else ["1", "2"]
        albums = [a for f in folders for a in ALBUMS[f]]
        if call == "ping": return self.ok({})
        if call == "getMusicFolders": return self.ok({"musicFolders": {"musicFolder": [{"id": 1, "name": "Lossless"}, {"id": 2, "name": "Lossy"}]}})
        if call == "getAlbumList2":
            off, n = int(q.get("offset", 0)), int(q.get("size", 500))
            return self.ok({"albumList2": {"album": [{"id": f"al-{i}", "name": nm, "artist": ar, "year": y, "coverArt": f"al-{i}"} for i, nm, ar, y in albums[off:off + n]]}})
        if call == "search3":
            off, n = int(q.get("songOffset", 0)), int(q.get("songCount", 500))
            songs = [{"id": f"so-{i}-{t}", "albumId": f"al-{i}", "title": f"Song {t}", "artist": ar, "track": t, "discNumber": 1, "duration": SECONDS, "suffix": "wav", "bitRate": 900, "size": 20000000, "samplingRate": 44100, "bitDepth": 16} for i, nm, ar, y in albums for t in (1, 2, 3)]
            return self.ok({"searchResult3": {"song": songs[off:off + n]}})
        if call == "getCoverArt":
            i = int(q.get("id", "al-0").split("-")[1])
            rgb = ((i * 53) % 256 or 200, (i * 97) % 256, (i * 31) % 256)
            # Album 0 wide, album 1 nearly square (295 x 300), the rest square.
            return self.send(png(rgb, 300, 170 if i == 0 else 295 if i == 1 else 300), "image/png")
        if call == "getLyricsBySongId":
            t = int(q["id"].split("-")[2])
            if t == 1:
                lines = [{"start": i * 700, "value": w} for i, w in enumerate(["", "First sung line", "The second one, a little longer than the first", "Third", "Fourth line here", "Fifth", "Sixth", "Seventh", "The last line"])]
                return self.ok({"lyricsList": {"structuredLyrics": [{"synced": True, "lang": "eng", "line": lines}]}})
            if t == 2:
                lines = [{"value": f"Plain line number {i}"} for i in range(1, 25)]
                return self.ok({"lyricsList": {"structuredLyrics": [{"synced": False, "lang": "eng", "line": lines}]}})
            return self.ok({"lyricsList": {}})
        if call == "stream":
            t = int(q["id"].split("-")[2]); STREAMS.append(q["id"]); print("stream", q["id"], q.get("format"), flush=True)
            time.sleep(0.5)  # a server a little slow to answer
            if t not in WAVS: WAVS[t] = wav(220 * t)
            return self.send(WAVS[t], "audio/wav")
        self.send_response(404); self.end_headers()

MODE = sys.argv[2] if len(sys.argv) > 2 else "good"
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
