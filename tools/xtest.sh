#!/bin/sh
# Vergence X11 test apps (run inside vergence-server: from foot, or `vctl spawn tools/xtest.sh`): size boxes in
# GTK 3 and Qt forced onto X11, an Xft xterm, and AlbumWall (Avalonia, X11).
B=$HOME/src/vergence/tools/sizebox/bin
GDK_BACKEND=x11 "$B/gtk3" &
QT_QPA_PLATFORM=xcb "$B/qt" &
xterm -fa Monospace -fs 11 &
"$HOME/.local/share/albumwall/AlbumWall" > /tmp/albumwall-x.log 2>&1 &
