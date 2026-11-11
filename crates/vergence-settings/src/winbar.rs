// The title bar's layout: which side the window controls are on and which
// buttons there are (2026-11-08, Dave: controls on either side, buttons chosen
// in the settings app). Kept as GTK's Gtk/DecorationLayout in settings.toml, so
// GTK apps' own bars follow it too, and the server's bars and the terminal's
// read it: "close,minimize,maximize:" (left), ":minimize,maximize,close" (right).

pub const KEY: &str = "Gtk/DecorationLayout";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub left: bool,
    pub minimize: bool,
    pub maximize: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { left: false, minimize: true, maximize: true }
    }
}

/// Read a GTK decoration layout: the side "close" is on, and whether the
/// other buttons appear at all.
pub fn parse(s: &str) -> Layout {
    let Some((left, right)) = s.split_once(':') else { return Layout::default() };
    let has = |side: &str, b: &str| side.split(',').any(|x| x.trim() == b);
    let on_left = has(left, "close") || (!has(right, "close") && (has(left, "minimize") || has(left, "maximize")));
    Layout {
        left: on_left,
        minimize: has(left, "minimize") || has(right, "minimize"),
        maximize: has(left, "maximize") || has(right, "maximize"),
    }
}

/// Write it back: close outermost, then minimize and maximize.
pub fn format(l: Layout) -> String {
    let mut buttons = Vec::new();
    if l.left {
        buttons.push("close");
        if l.minimize {
            buttons.push("minimize");
        }
        if l.maximize {
            buttons.push("maximize");
        }
        format!("{}:", buttons.join(","))
    } else {
        if l.minimize {
            buttons.push("minimize");
        }
        if l.maximize {
            buttons.push("maximize");
        }
        buttons.push("close");
        format!(":{}", buttons.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        // Hubble's Xfce-era value: buttons on the right.
        assert_eq!(parse("icon,menu:minimize,maximize,close"), Layout { left: false, minimize: true, maximize: true });
        assert_eq!(parse("close,minimize,maximize:"), Layout { left: true, minimize: true, maximize: true });
        assert_eq!(parse(":close"), Layout { left: false, minimize: false, maximize: false });
        assert_eq!(parse("nonsense"), Layout::default());
        for l in [Layout { left: true, minimize: false, maximize: true }, Layout { left: false, minimize: true, maximize: false }] {
            assert_eq!(parse(&format(l)), l);
        }
        assert_eq!(format(Layout::default()), ":minimize,maximize,close");
    }
}
