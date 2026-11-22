//! The machine's own keyboard layout, for the login screen (VDM step 6):
//! Fedora's /etc/vconsole.conf (XKBLAYOUT, XKBVARIANT, XKBOPTIONS, written
//! by localectl), else the console keymap (Void's KEYMAP in /etc/rc.conf,
//! FreeBSD's keymap= in /etc/rc.conf, Fedora's KEYMAP) turned into an xkb
//! layout. vdm hands it to the login screen as XKB_DEFAULT_LAYOUT and
//! friends, which xkbcommon reads when the server names no layout.

/// An xkb layout, variant and options (empty: xkb's default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub layout: String,
    pub variant: String,
    pub options: String,
}

/// A console keymap's name as an xkb layout: the part before '-', '.' or
/// '_' ("de-latin1", "us.kbd", "fr_CH" ...), with the names that differ.
pub fn layout_of_keymap(keymap: &str) -> Option<String> {
    let base = keymap.trim().trim_matches('"').split(['-', '.', '_']).next()?.to_lowercase();
    let xkb = match base.as_str() {
        "" => return None,
        "en" | "defkeymap" => "us",
        "uk" => "gb",
        "dk" => "dk",
        "sg" => "ch",
        "jp106" => "jp",
        other if other.len() == 2 => other,
        _ => return None,
    };
    Some(xkb.to_string())
}

/// `KEY=value` lines (sh-style, quotes taken off).
fn values(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            l.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').trim_matches('\'').to_string()))
        })
        .collect()
}

/// From the files' texts (vconsole.conf, rc.conf), for testing.
pub fn from_texts(vconsole: &str, rc_conf: &str) -> Option<Layout> {
    let v = values(vconsole);
    let get = |k: &str| v.iter().find(|(n, _)| n == k).map(|(_, x)| x.clone()).unwrap_or_default();
    if !get("XKBLAYOUT").is_empty() {
        return Some(Layout { layout: get("XKBLAYOUT"), variant: get("XKBVARIANT"), options: get("XKBOPTIONS") });
    }
    let r = values(rc_conf);
    let keymap = [get("KEYMAP")]
        .into_iter()
        .chain(r.iter().filter(|(n, _)| n == "KEYMAP" || n == "keymap").map(|(_, x)| x.clone()))
        .find(|k| !k.is_empty())?;
    Some(Layout { layout: layout_of_keymap(&keymap)?, ..Default::default() })
}

/// This machine's.
pub fn system() -> Option<Layout> {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    from_texts(&read("/etc/vconsole.conf"), &read("/etc/rc.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_from_the_systems_files() {
        let fedora = "KEYMAP=us\nFONT=eurlatgr\nXKBLAYOUT=us\nXKBOPTIONS=caps:none\n";
        assert_eq!(
            from_texts(fedora, ""),
            Some(Layout { layout: "us".into(), variant: String::new(), options: "caps:none".into() })
        );
        assert_eq!(from_texts("", "# Keymap\nKEYMAP=en\n").map(|l| l.layout), Some("us".into()));
        assert_eq!(from_texts("", "keymap=\"de.kbd\"\n").map(|l| l.layout), Some("de".into()));
        assert_eq!(from_texts("KEYMAP=de-latin1\n", "").map(|l| l.layout), Some("de".into()));
        assert_eq!(from_texts("", "KEYMAP=uk\n").map(|l| l.layout), Some("gb".into()));
        assert_eq!(from_texts("", ""), None);
        assert_eq!(layout_of_keymap("colemak"), None);
    }
}
