#!/bin/bash
# Build docs/Vergence-Whitepaper.pdf on a Mac with nothing but what macOS has:
# a Python virtual environment for the Markdown package, and WebKit (through
# docs/html2pdf.swift, compiled with the Command Line Tools' swiftc) to print.
# On Windows, build-pdf.py's HTML can be printed from Edge instead.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${TMPDIR:-/tmp}/vergence-pdf
mkdir -p "$work"
[ -x "$work/venv/bin/python" ] || { python3 -m venv "$work/venv"; "$work/venv/bin/pip" -q install markdown; }
[ "$work/html2pdf" -nt "$here/html2pdf.swift" ] || swiftc -O -o "$work/html2pdf" "$here/html2pdf.swift"
"$work/venv/bin/python" -I "$here/build-pdf.py" "$here/Vergence-Whitepaper.md" "$work/Vergence-Whitepaper.html"
"$work/html2pdf" "$work/Vergence-Whitepaper.html" "$here/Vergence-Whitepaper.pdf" 2>/dev/null
