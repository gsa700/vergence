# tools/lab: the project's own lab

Scripts for the machines this project is developed on, not for building or
running Vergence elsewhere. They assume the lab's way of working: the git
copy on one machine (a Mac with no Rust), the builds and tests on dev boxes
reached over ssh by name (hubble, techbench, kepler). CLAUDE.md describes
those machines.

- `sync.sh [host]`: copy the working tree to a dev box (~/src/vergence).
- `display-suite.sh <host> [scenario...]`: the display suite on a dev box.
  On your own machine: `tools/display-suite/run.sh`.
- `display-check.sh <host> <screen> [scale...]`: the display check on a dev
  box. On your own machine: `tools/display-check.sh`.
- `fetch-gpu-reports.sh [host...]`: copy the boxes' GPU hang reports
  (`tools/gpu-report.sh`) off them.
- `ups-tile`, `vtile-demo`, `autostart.desktop`: dock tiles fed by scripts
  (the rack's UPS over NUT), installed by tools/install.sh as examples.
- From earlier phases, kept for reference: `sizebox/` and `xtest.sh` (window
  sizes across toolkits, phase 1), `vterm-stress/` (the terminal's stress
  runs), `start-xfce`, `install-session.sh`, `run-server-drm.sh` (ways the
  server was started before the session and login screen existed).
