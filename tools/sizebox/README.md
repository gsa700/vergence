# Size boxes (milestone 4, acceptance 3)

One tiny program per toolkit. Each opens a window titled
`vergence-box-<toolkit>` with a **magenta box of 200 x 200 in the toolkit's
own units** (GTK and Qt logical pixels, Java user space, CSS pixels) on a
white background. At the reference density (96 ppi at 71 cm) 200 units are
52.9 mm. `vergence-harness measure` finds the box on screen and works out its
size on the panel in millimetres; with Mode A it should be the same on every
monitor and in every toolkit (whitepaper §4.8.3).

    sh build.sh          # on Hubble: builds gtk3, gtk4, qt, xlib (Java and Electron run from source)
    sh measure.sh        # opens each, moves it to each monitor, measures

Xlib draws in device pixels and is expected to stay unscaled ("consistently
small", whitepaper §4.6).
