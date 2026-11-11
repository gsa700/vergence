# Toolkit spike: Slint vs iced (2026-11-08)

Dave asked for a spike before the terminal picks its toolkit (Slint's license
is not settled; iced is MIT and is what COSMIC Terminal is built on). Both
programs draw the same small window: our own title bar with two tabs, a +
button and window buttons (drag the bar to move the window), two panes side by
side of colored monospace text (a fake shell session), a draggable divider,
and a right-click menu. Separate workspaces: nothing here touches the main
Cargo.lock. Results: RESULTS.md.
