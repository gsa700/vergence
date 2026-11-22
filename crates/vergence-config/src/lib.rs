//! desktop.toml as it is in effect (defaults milestone 1, step 1).
//!
//! Three layers, each under the next, merged key by key:
//!
//! 1. Vergence's shipped defaults, `share/vergence/defaults/desktop.toml`:
//!    beside the program (`$bin/../share`), a checkout's `data/defaults`
//!    (`$bin/../../data`), else `/usr/local/share`, else `/usr/share`; the
//!    first that exists. A distribution, or our own ISO, changes what a new
//!    account starts with here without touching code.
//! 2. The machine's, `/etc/vergence/desktop.toml` and (FreeBSD's place for
//!    installed software's settings) `/usr/local/etc/vergence/desktop.toml`.
//! 3. The person's own, `~/.config/vergence/desktop.toml`.
//!
//! Tables merge key by key at every depth; any other value, arrays and arrays
//! of tables included (`[[wallpaper.monitor]]`), is replaced whole by the
//! layer above, so a person's list of monitors is never mixed with the
//! machine's. A layer that is not valid TOML is skipped: the rest still apply.
//!
//! Only readers use this. Whatever writes desktop.toml reads and writes the
//! person's own file alone, so a default is never copied into it (it would
//! stop following the default when that changes).
//!
//! `VERGENCE_DEFAULTS=none` leaves out layers 1 and 2 (a test session that
//! must not depend on what this machine has installed); any other value is
//! read as the path of the shipped defaults file instead of looking for one.

use std::path::{Path, PathBuf};

pub use toml::Table;

/// The person's own desktop.toml: `$XDG_CONFIG_HOME` when set and absolute,
/// else `~/.config`.
pub fn user_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("vergence/desktop.toml"))
}

/// The shipped defaults file in use, if there is one.
pub fn shipped_path() -> Option<PathBuf> {
    match std::env::var_os("VERGENCE_DEFAULTS") {
        Some(v) if v == "none" => return None,
        Some(v) if !v.is_empty() => return Some(PathBuf::from(v)),
        _ => {}
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        candidates.push(bin.join("../share/vergence/defaults/desktop.toml"));
        candidates.push(bin.join("../../data/defaults/desktop.toml"));
    }
    candidates.push("/usr/local/share/vergence/defaults/desktop.toml".into());
    candidates.push("/usr/share/vergence/defaults/desktop.toml".into());
    candidates.into_iter().find(|p| p.is_file())
}

/// The machine's own files, in the order they apply.
pub fn machine_paths() -> Vec<PathBuf> {
    if std::env::var_os("VERGENCE_DEFAULTS").is_some_and(|v| v == "none") {
        return Vec::new();
    }
    vec!["/etc/vergence/desktop.toml".into(), "/usr/local/etc/vergence/desktop.toml".into()]
}

/// Every layer under the person's own, lowest first.
pub fn layer_paths() -> Vec<PathBuf> {
    shipped_path().into_iter().chain(machine_paths()).collect()
}

/// What is in effect, with the person's file at `user` (each program keeps
/// its own idea of where that is; `user_path()` is the usual one).
pub fn effective_with(user: Option<&Path>) -> Table {
    let mut paths = layer_paths();
    paths.extend(user.map(Path::to_path_buf));
    merge_files(&paths)
}

/// What is in effect, with the person's file at `user_path()`.
pub fn effective() -> Table {
    effective_with(user_path().as_deref())
}

/// The same as TOML text, for code that parses desktop.toml's text itself.
pub fn effective_text_with(user: Option<&Path>) -> String {
    let table = effective_with(user);
    if table.is_empty() { String::new() } else { toml::to_string(&table).unwrap_or_default() }
}

/// The same as TOML text, with the person's file at `user_path()`.
pub fn effective_text() -> String {
    effective_text_with(user_path().as_deref())
}

/// The newest change time among the layers and the person's file, for code
/// that reloads when desktop.toml changes: a change to the machine's or the
/// shipped file counts too.
pub fn modified_with(user: Option<&Path>) -> Option<std::time::SystemTime> {
    layer_paths()
        .iter()
        .map(PathBuf::as_path)
        .chain(user)
        .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .max()
}

/// The files merged in order, later over earlier; missing or invalid ones
/// are skipped.
pub fn merge_files(paths: &[PathBuf]) -> Table {
    let mut out = Table::new();
    for p in paths {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Ok(table) = text.parse::<Table>() else { continue };
        merge(&mut out, table);
    }
    out
}

/// `over` into `base`: tables key by key at every depth, anything else
/// replaced whole.
pub fn merge(base: &mut Table, over: Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(key, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vergence-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(d: &Path, name: &str, text: &str) -> PathBuf {
        let p = d.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn nested_tables_merge_key_by_key_and_the_person_wins() {
        let d = dir("nested");
        let shipped =
            write(&d, "shipped.toml", "[wallpaper]\nimage = \"a.svg\"\nmode = \"fill\"\n[shelf]\nside = \"bottom\"\n");
        let user = write(&d, "user.toml", "[wallpaper]\nimage = \"mine.jpg\"\n");
        let t = merge_files(&[shipped, user]);
        assert_eq!(t["wallpaper"]["image"].as_str(), Some("mine.jpg"));
        assert_eq!(t["wallpaper"]["mode"].as_str(), Some("fill"));
        assert_eq!(t["shelf"]["side"].as_str(), Some("bottom"));
    }

    #[test]
    fn arrays_are_replaced_whole() {
        let d = dir("arrays");
        let shipped = write(&d, "shipped.toml", "[shelf]\npinned = [\"a\", \"b\", \"c\"]\n");
        let user = write(&d, "user.toml", "[shelf]\npinned = [\"z\"]\n");
        let t = merge_files(&[shipped, user]);
        assert_eq!(t["shelf"]["pinned"].as_array().unwrap().len(), 1);
        // An empty list is a choice too: the person unpinned everything.
        let empty = write(&d, "empty.toml", "[shelf]\npinned = []\n");
        let t = merge_files(&[d.join("shipped.toml"), empty]);
        assert!(t["shelf"]["pinned"].as_array().unwrap().is_empty());
    }

    #[test]
    fn arrays_of_tables_are_replaced_whole() {
        let d = dir("aot");
        let machine =
            write(&d, "machine.toml", "[[wallpaper.monitor]]\nid = \"1\"\n[[wallpaper.monitor]]\nid = \"2\"\n");
        let user = write(&d, "user.toml", "[[wallpaper.monitor]]\nid = \"3\"\n");
        let t = merge_files(&[machine, user]);
        let list = t["wallpaper"]["monitor"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"].as_str(), Some("3"));
    }

    #[test]
    fn an_invalid_layer_is_skipped_and_the_rest_apply() {
        let d = dir("invalid");
        let shipped = write(&d, "shipped.toml", "[energy]\nscreens-off = 10\n");
        let broken = write(&d, "machine.toml", "[energy\nthis is not toml");
        let user = write(&d, "user.toml", "[energy]\nsleep = 0\n");
        let t = merge_files(&[shipped, broken, user, d.join("missing.toml")]);
        assert_eq!(t["energy"]["screens-off"].as_integer(), Some(10));
        assert_eq!(t["energy"]["sleep"].as_integer(), Some(0));
    }

    #[test]
    fn a_table_replaces_a_value_and_a_value_replaces_a_table() {
        let mut base: Table = "a = 1\n[b]\nc = 2\n".parse().unwrap();
        merge(&mut base, "b = 3\n[a]\nd = 4\n".parse().unwrap());
        assert_eq!(base["b"].as_integer(), Some(3));
        assert_eq!(base["a"]["d"].as_integer(), Some(4));
    }
}
