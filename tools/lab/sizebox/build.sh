#!/bin/sh
# Build the compiled size boxes into ./bin (on Hubble).
set -e
cd "$(dirname "$0")"
mkdir -p bin
cc -O2 -o bin/gtk3 gtk3.c $(pkg-config --cflags --libs gtk+-3.0)
cc -O2 -o bin/gtk4 gtk4.c $(pkg-config --cflags --libs gtk4)
c++ -O2 -fPIC -std=c++17 -o bin/qt qt.cpp $(pkg-config --cflags --libs Qt6Widgets)
cc -O2 -o bin/xlib xlib.c $(pkg-config --cflags --libs x11)
# Avalonia (FreeBSD: needs the native libraries from BsdProbe in $NATIVE).
NATIVE=${NATIVE:-${XDG_CACHE_HOME:-$HOME/.cache}/vergence-lab/native}
if command -v dotnet >/dev/null; then
    dotnet publish avalonia/SizeBox.csproj -c Release -o bin/avalonia -nologo -v quiet
    [ -d "$NATIVE" ] && cp -P "$NATIVE"/* bin/avalonia/
fi
echo "built: $(ls bin | tr '\n' ' ')"
