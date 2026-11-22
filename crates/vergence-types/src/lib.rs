//! File types (files milestone 1, step 1): what a file is, and which
//! applications open it.
//!
//! - **What a file is**: its name against our own table ([`table`]: the
//!   longest matching pattern wins), then, when the name says nothing, its
//!   first bytes ([`of_bytes`]: the signatures of the formats in the table),
//!   then whether it reads as text. A folder is `inode/directory`; anything
//!   else unknown is `application/octet-stream`.
//! - **Kinds of kinds**: a type's parents ([`ancestors`]): a shell script
//!   is also plain text, a Word document also a zip archive; every text type
//!   is plain text, and everything is a binary file. An application that
//!   opens plain text is offered for a shell script.
//! - **Which applications open it**: what each application's desktop entry
//!   says it opens (its MimeType line), and the user's choices in the XDG
//!   `mimeapps.list` files ([`MimeApps`], [`lists`]), Vergence's own first:
//!   `~/.config/vergence-mimeapps.list` (see docs/files-milestone-1.md), so a choice made
//!   here holds in a Vergence session whatever else rewrites the shared one.
//!
//! Our own code and data; nothing here reads the system's shared-mime-info.

pub mod table;

use std::path::{Path, PathBuf};

#[cfg(target_os = "freebsd")]
const O_NONBLOCK: i32 = 0x0004;
#[cfg(not(target_os = "freebsd"))]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(target_os = "freebsd")]
const O_NOCTTY: i32 = 0x8000;
#[cfg(not(target_os = "freebsd"))]
const O_NOCTTY: i32 = 0o400;

/// Opens `path` for reading only if it is a regular file (links followed).
/// A named pipe, a device or a socket is refused without waiting: the open
/// does not block (a pipe with no writer would hang it), a terminal does not
/// become ours, and what was opened is checked with fstat before anything is
/// read. Every "read the file the user named" goes through here (security
/// pass 2026-11-15, finding 5).
pub fn open_regular(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(O_NONBLOCK | O_NOCTTY).open(path)?;
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"));
    }
    // O_NONBLOCK means nothing to a regular file's reads; it stays set.
    Ok(f)
}

/// Reads a regular file whole ([`open_regular`]), refusing one longer than
/// `max` bytes (counted as read, not believed from its length).
pub fn read_regular(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    open_regular(path)?.take(max + 1).read_to_end(&mut out)?;
    if out.len() as u64 > max {
        return Err(std::io::Error::new(std::io::ErrorKind::FileTooLarge, "file too large"));
    }
    Ok(out)
}

pub use table::Row;

/// A folder's type.
pub const FOLDER: &str = "inode/directory";
/// What nothing else describes.
pub const BINARY: &str = "application/octet-stream";
/// Plain text.
pub const TEXT: &str = "text/plain";
/// A link's type is this and its scheme ("x-scheme-handler/https"), as
/// desktop entries and mimeapps.list name the applications for links.
pub const SCHEME: &str = "x-scheme-handler/";
/// How much of a file is read to tell its type by its bytes.
pub const HEAD: usize = 4096;
/// The name in mimeapps.list's per-desktop file, as XDG_CURRENT_DESKTOP says it, lower case.
pub const DESKTOP: &str = "vergence";

/// A type's row in the table, by any of its names.
pub fn row(mime: &str) -> Option<&'static Row> {
    let m = canonical(mime);
    table::ROWS.iter().find(|r| r.mime == m)
}

/// A type's name as the table has it: lower case, aliases resolved.
pub fn canonical(mime: &str) -> String {
    let m = mime.trim().to_ascii_lowercase();
    table::ALIASES.iter().find(|(a, _)| *a == m).map(|(_, to)| to.to_string()).unwrap_or(m)
}

/// What a type is called ("PNG picture"); the type itself if not in the table.
pub fn describe(mime: &str) -> String {
    row(mime).map(|r| r.name.to_string()).unwrap_or_else(|| mime.to_string())
}

/// A type's icon names, the most specific first: the type's own
/// ("image-png", by the icon naming spec's rule) then its generic one.
pub fn icons(mime: &str) -> [String; 2] {
    let m = canonical(mime);
    let generic = row(&m).map(|r| r.icon.to_string()).unwrap_or_else(|| {
        match m.split('/').next() {
            Some("text") => "text-x-generic",
            Some("image") => "image-x-generic",
            Some("audio") => "audio-x-generic",
            Some("video") => "video-x-generic",
            Some("font") => "font-x-generic",
            _ => "application-x-executable",
        }
        .to_string()
    });
    [m.replace('/', "-"), generic]
}

/// The type a file's name says, if any: the longest pattern that matches
/// (`*.tar.gz` before `*.gz`); patterns with no `*` are whole names,
/// matched exactly, and the rest without regard to case.
pub fn of_name(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let mut best: Option<(&'static str, usize)> = None;
    for r in table::ROWS {
        for g in r.globs {
            let hit = match g.strip_prefix('*') {
                Some(suffix) => lower.len() > suffix.len() && lower.ends_with(&suffix.to_ascii_lowercase()),
                None => name == *g,
            };
            if hit && best.is_none_or(|(_, len)| g.len() > len) {
                best = Some((r.mime, g.len()));
            }
        }
    }
    best.map(|(m, _)| m)
}

/// The type a file's first bytes say, by the signatures of the formats in
/// the table; then text (no NUL, UTF-8, a character cut at the end
/// allowed). None: not known, and not text.
pub fn of_bytes(head: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, sig: &[u8]| head.get(offset..offset + sig.len()) == Some(sig);
    let signatures: &[(usize, &[u8], &str)] = &[
        (0, b"\x89PNG\r\n\x1a\n", "image/png"),
        (0, b"\xff\xd8\xff", "image/jpeg"),
        (0, b"GIF87a", "image/gif"),
        (0, b"GIF89a", "image/gif"),
        (0, b"II*\0", "image/tiff"),
        (0, b"MM\0*", "image/tiff"),
        (0, b"\xff\x0a", "image/jxl"),
        (0, b"\0\0\0\x0cJXL \r\n\x87\n", "image/jxl"),
        (0, b"%PDF-", "application/pdf"),
        (0, b"%!PS", "application/postscript"),
        (0, b"{\\rtf", "application/rtf"),
        (0, b"PK\x03\x04", "application/zip"),
        (0, b"\x1f\x8b", "application/gzip"),
        (0, b"\xfd7zXZ\0", "application/x-xz"),
        (0, b"\x28\xb5\x2f\xfd", "application/zstd"),
        (0, b"BZh", "application/x-bzip2"),
        (0, b"\x04\x22\x4d\x18", "application/x-lz4"),
        (0, b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed"),
        (0, b"Rar!\x1a\x07", "application/vnd.rar"),
        (257, b"ustar", "application/x-tar"),
        (0, b"!<arch>\ndebian", "application/vnd.debian.binary-package"),
        (0, b"\xed\xab\xee\xdb", "application/x-rpm"),
        (0, b"\x7fELF", "application/x-executable"),
        (0, b"MZ", "application/x-ms-dos-executable"),
        (0, b"SQLite format 3\0", "application/x-sqlite3"),
        (0, b"fLaC", "audio/flac"),
        (0, b"OggS", "audio/ogg"),
        (0, b"ID3", "audio/mpeg"),
        (0, b"MThd", "audio/midi"),
        (0, b"FORM", "audio/x-aiff"),
        (0, b"\x1a\x45\xdf\xa3", "video/x-matroska"),
        (0, b"OTTO", "font/otf"),
        (0, b"\0\x01\0\0\0", "font/ttf"),
        (0, b"wOFF", "font/woff"),
        (0, b"wOF2", "font/woff2"),
        (0, b"d8:announce", "application/x-bittorrent"),
    ];
    if let Some((_, _, m)) = signatures.iter().find(|(o, sig, _)| at(*o, sig)) {
        return Some(m);
    }
    // RIFF holds several formats; its kind is at byte 8.
    if at(0, b"RIFF") {
        return [(b"WEBP", "image/webp"), (b"WAVE", "audio/x-wav"), (b"AVI ", "video/x-msvideo")]
            .into_iter()
            .find(|(k, _)| at(8, *k))
            .map(|(_, m)| m);
    }
    // ISO base media (MPEG-4, QuickTime, HEIF, AVIF): "ftyp" at 4, the brand at 8.
    if at(4, b"ftyp") {
        let brand = head.get(8..12).unwrap_or_default();
        return Some(match brand {
            b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" => "image/heif",
            b"avif" | b"avis" => "image/avif",
            b"M4A " | b"M4B " => "audio/mp4",
            b"qt  " => "video/quicktime",
            _ => "video/mp4",
        });
    }
    // BMP: "BM", then a header size the format has (40 and up at byte 14).
    if at(0, b"BM") && head.len() >= 18 && u32::from_le_bytes([head[14], head[15], head[16], head[17]]) >= 12 {
        return Some("image/bmp");
    }
    // An MPEG transport stream: a sync byte every 188 bytes.
    if head.len() > 376 && head[0] == 0x47 && head[188] == 0x47 && head[376] == 0x47 {
        return Some("video/mp2t");
    }
    if !is_text(head) {
        return None;
    }
    let text = String::from_utf8_lossy(head);
    let start = text.trim_start_matches('\u{feff}').trim_start();
    let lower = start.chars().take(512).collect::<String>().to_ascii_lowercase();
    if let Some(line) = start.strip_prefix("#!") {
        let line = line.lines().next().unwrap_or_default();
        let program =
            line.split_whitespace().flat_map(|w| w.rsplit('/').next()).find(|w| *w != "env").unwrap_or_default();
        return Some(match program {
            p if p.starts_with("python") => "text/x-python",
            p if p.starts_with("perl") => "application/x-perl",
            p if p.starts_with("ruby") => "application/x-ruby",
            p if p.starts_with("node") => "text/javascript",
            p if p.starts_with("lua") => "text/x-lua",
            p if p.ends_with("sh") => "application/x-shellscript",
            _ => TEXT,
        });
    }
    if lower.starts_with("<svg") || (lower.starts_with("<?xml") && lower.contains("<svg")) {
        return Some("image/svg+xml");
    }
    if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
        return Some("text/html");
    }
    if lower.starts_with("<?xml") {
        return Some("application/xml");
    }
    if start.starts_with("[Desktop Entry]") {
        return Some("application/x-desktop");
    }
    Some(TEXT)
}

/// Whether bytes read as text: no NUL, and UTF-8 (a character cut off at
/// the end of what was read is allowed). Empty counts as text, so an empty
/// file opens in a text editor.
pub fn is_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && head.len() - e.valid_up_to() < 4,
    }
}

/// A file's type: a folder; else what its name says; else what its first
/// bytes say; else a binary file. Follows links; a file that cannot be read
/// goes by its name alone, and one that is not a regular file (a pipe, a
/// device) by its name or else as a binary file, without being opened.
pub fn of_file(path: &Path) -> &'static str {
    if path.is_dir() {
        return FOLDER;
    }
    if let Some(m) = path.file_name().and_then(|n| n.to_str()).and_then(of_name) {
        return m;
    }
    let mut head = Vec::with_capacity(HEAD);
    match open_regular(path) {
        Ok(f) => {
            use std::io::Read;
            let _ = f.take(HEAD as u64).read_to_end(&mut head);
        }
        // A pipe, a device or a socket: nothing to open it with.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => return BINARY,
        Err(_) => {}
    }
    of_bytes(&head).unwrap_or(BINARY)
}

/// The type a link is opened by: `x-scheme-handler/<scheme>` for a link
/// with a scheme ("https://..."), None for a plain path or a `file:` link
/// (whose file's own type counts).
pub fn of_link(link: &str) -> Option<String> {
    let (scheme, rest) = link.split_once(':')?;
    let ok = !scheme.is_empty()
        && scheme.len() <= 32
        && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    (ok && !scheme.eq_ignore_ascii_case("file") && !rest.is_empty())
        .then(|| format!("{SCHEME}{}", scheme.to_ascii_lowercase()))
}

/// The path a `file://` link names (its %XX escapes decoded), None for any
/// other link.
pub fn path_of_link(link: &str) -> Option<PathBuf> {
    let rest = link.strip_prefix("file://")?;
    // "file://host/path": only this machine's ("" or "localhost").
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|b| hex(*b)), bytes.get(i + 2).and_then(|b| hex(*b)))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// The type itself and the types it is a kind of, nearest first: its
/// table parents, `text/plain` for any text type, and last
/// `application/octet-stream` (not for folders or links).
pub fn ancestors(mime: &str) -> Vec<String> {
    let mut out = vec![canonical(mime)];
    let mut i = 0;
    while i < out.len() && out.len() < 16 {
        let parent = row(&out[i]).and_then(|r| r.parent).map(str::to_string);
        if let Some(p) = parent
            && !out.contains(&p)
        {
            out.push(p);
        }
        i += 1;
    }
    if out.iter().any(|m| m.starts_with("text/")) && !out.iter().any(|m| m == TEXT) {
        out.push(TEXT.into());
    }
    if !out[0].starts_with("inode/") && !out[0].starts_with(SCHEME) && !out.iter().any(|m| m == BINARY) {
        out.push(BINARY.into());
    }
    out
}

/// Whether `mime` is `of`, or a kind of it.
pub fn is_a(mime: &str, of: &str) -> bool {
    let of = canonical(of);
    ancestors(mime).contains(&of)
}

/// Whether a type an application lists matches a type, with the `image/*`
/// form some entries use.
fn listed(entry_type: &str, mime: &str) -> bool {
    let e = canonical(entry_type);
    match e.strip_suffix("/*") {
        Some(top) => mime.split('/').next() == Some(top),
        None => e == mime,
    }
}

/// The user's choices in one mimeapps.list file (XDG "Association between
/// MIME types and applications"): a default for some types, applications
/// added for some, applications removed from some. Applications by desktop
/// file id ("firefox.desktop").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MimeApps {
    pub defaults: Vec<(String, Vec<String>)>,
    pub added: Vec<(String, Vec<String>)>,
    pub removed: Vec<(String, Vec<String>)>,
    /// Read from a vergence-mimeapps.list: choices made in Vergence.
    pub vergence: bool,
}

/// The longest mimeapps.list read, and the most types one may give.
const MAX_LIST_FILE: usize = 1024 * 1024;
const MAX_TYPES: usize = 4096;

impl MimeApps {
    /// Reads a file's text; what cannot be understood is skipped.
    pub fn parse(text: &str) -> MimeApps {
        let text = text.get(..text.len().min(MAX_LIST_FILE)).unwrap_or_default();
        let mut out = MimeApps::default();
        let mut group = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                group = match line {
                    "[Default Applications]" => Some(0),
                    "[Added Associations]" => Some(1),
                    "[Removed Associations]" => Some(2),
                    _ => None,
                };
                continue;
            }
            let (Some(g), Some((key, value))) = (group, line.split_once('=')) else { continue };
            let apps: Vec<String> =
                value.split(';').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect();
            let list = match g {
                0 => &mut out.defaults,
                1 => &mut out.added,
                _ => &mut out.removed,
            };
            if list.len() < MAX_TYPES && !apps.is_empty() {
                list.push((canonical(key), apps));
            }
        }
        out
    }

    /// The file's text: the three groups, each type on its own line.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (title, list) in [
            ("Default Applications", &self.defaults),
            ("Added Associations", &self.added),
            ("Removed Associations", &self.removed),
        ] {
            if list.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{title}]\n"));
            for (mime, apps) in list {
                out.push_str(&format!("{mime}={};\n", apps.join(";")));
            }
        }
        out
    }

    /// Makes `app` the default for `mime` (the one entry for it).
    pub fn set_default(&mut self, mime: &str, app: &str) {
        let mime = canonical(mime);
        self.defaults.retain(|(m, _)| *m != mime);
        self.defaults.push((mime.clone(), vec![app.to_string()]));
        // Chosen again: no longer removed.
        for (m, apps) in &mut self.removed {
            if *m == mime {
                apps.retain(|a| a != app);
            }
        }
        self.removed.retain(|(_, apps)| !apps.is_empty());
    }

    fn of<'a>(list: &'a [(String, Vec<String>)], mime: &str) -> impl Iterator<Item = &'a String> {
        list.iter().filter(move |(m, _)| m == mime).flat_map(|(_, a)| a)
    }
}

/// The mimeapps.list files in the order the XDG spec reads them, each
/// directory's per-desktop file (Vergence's) before its shared one: the
/// user's config, the system's config directories, then the data
/// directories' `applications/`. Only those that exist.
pub fn list_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let env_dirs = |var: &str, default: &str| -> Vec<PathBuf> {
        match std::env::var_os(var).filter(|v| !v.is_empty()) {
            Some(v) => std::env::split_paths(&v).collect(),
            None => std::env::split_paths(default).collect(),
        }
    };
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let mut dirs = vec![config_home];
    dirs.extend(env_dirs("XDG_CONFIG_DIRS", "/etc/xdg:/usr/local/etc/xdg"));
    dirs.push(data_home.join("applications"));
    dirs.extend(env_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share").into_iter().map(|d| d.join("applications")));
    let mut out = Vec::new();
    for d in dirs {
        for name in [format!("{DESKTOP}-mimeapps.list"), "mimeapps.list".to_string()] {
            let p = d.join(name);
            if p.is_file() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// Where Vergence writes the user's choices: `vergence-mimeapps.list` in
/// the user's config directory (the shared mimeapps.list is never written).
pub fn own_list_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join(format!("{DESKTOP}-mimeapps.list"))
}

/// Every list file read, in [`list_paths`]'s order.
pub fn lists() -> Vec<MimeApps> {
    let ours = format!("{DESKTOP}-mimeapps.list");
    list_paths()
        .iter()
        .filter_map(|p| {
            let mut l = MimeApps::parse(&std::fs::read_to_string(p).ok()?);
            l.vergence = p.file_name().is_some_and(|n| n.to_string_lossy() == ours);
            Some(l)
        })
        .collect()
}

/// An installed application as far as file types go: its desktop file id
/// ("firefox.desktop") and the types its entry lists.
pub struct App<'a> {
    pub id: &'a str,
    pub types: &'a [String],
}

/// The applications that open `mime`, best first, by desktop file id:
/// first the choices made in Vergence (vergence-mimeapps.list) for the type
/// or a type it is a kind of, then Vergence's own apps (`vergence-*`) that
/// open it (Vergence apps first: GNOME's choice of its text editor for
/// Markdown does not beat ours in a Vergence session; docs/files-milestone-1.md); then,
/// for the type and each type it is a kind of, the defaults the other lists
/// give (the first list that names one wins), those added, and those whose
/// entries list it; minus those removed (for the type or any nearer one),
/// and only installed ones. Each application once.
pub fn apps_for(mime: &str, apps: &[App], lists: &[MimeApps]) -> Vec<String> {
    let installed = |id: &str| apps.iter().any(|a| a.id == id);
    let mut out: Vec<String> = Vec::new();
    let push = |id: &str, out: &mut Vec<String>, removed: &[String]| {
        if installed(id) && !out.iter().any(|o| o == id) && !removed.iter().any(|r| r == id) {
            out.push(id.to_string());
        }
    };
    // An application removed for a type, in any list, is not offered for it,
    // nor through the types it is a kind of.
    let kinds = ancestors(mime);
    let all_removed: Vec<String> =
        kinds.iter().flat_map(|m| lists.iter().flat_map(move |l| MimeApps::of(&l.removed, m)).cloned()).collect();
    for m in &kinds {
        for l in lists.iter().filter(|l| l.vergence) {
            for id in MimeApps::of(&l.defaults, m) {
                push(id, &mut out, &all_removed);
            }
        }
    }
    for m in kinds.iter().filter(|m| m.as_str() != BINARY) {
        for a in apps.iter().filter(|a| a.id.starts_with("vergence-")) {
            if a.types.iter().any(|t| listed(t, m)) {
                push(a.id, &mut out, &all_removed);
            }
        }
    }
    let mut removed: Vec<String> = Vec::new();
    for m in kinds {
        removed.extend(lists.iter().flat_map(|l| MimeApps::of(&l.removed, &m)).cloned());
        for l in lists.iter().filter(|l| !l.vergence) {
            for id in MimeApps::of(&l.defaults, &m) {
                push(id, &mut out, &removed);
            }
        }
        for l in lists {
            for id in MimeApps::of(&l.added, &m) {
                push(id, &mut out, &removed);
            }
        }
        // Binary file is everything's last parent: only applications that list it
        // by name, never every application.
        for a in apps {
            if a.types.iter().any(|t| if m == BINARY { canonical(t) == BINARY } else { listed(t, &m) }) {
                push(a.id, &mut out, &removed);
            }
        }
    }
    out
}

/// The application that opens `mime` by default: the first of
/// [`apps_for`].
pub fn default_for(mime: &str, apps: &[App], lists: &[MimeApps]) -> Option<String> {
    apps_for(mime, apps, lists).into_iter().next()
}

/// Makes `app` the default for `mime` in Vergence's own list file.
pub fn choose_default(mime: &str, app: &str) -> std::io::Result<()> {
    choose_defaults(&[mime], Some(app))
}

/// Makes `app` the default for each of `mimes` in Vergence's own list file,
/// or with None forgets Vergence's choice for them (the shared lists'
/// then count). The file is written whole: a temporary file renamed over
/// it, owner only.
pub fn choose_defaults(mimes: &[&str], app: Option<&str>) -> std::io::Result<()> {
    let path = own_list_path();
    let mut list = std::fs::read_to_string(&path).map(|t| MimeApps::parse(&t)).unwrap_or_default();
    for m in mimes {
        match app {
            Some(a) => list.set_default(m, a),
            None => {
                let m = canonical(m);
                list.defaults.retain(|(d, _)| *d != m);
            }
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("list.new");
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(list.to_text().as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)
}

/// Every type in the table under a top-level kind ("image": every picture type).
pub fn of_kind(top: &str) -> Vec<&'static str> {
    table::ROWS.iter().filter(|r| r.mime.split('/').next() == Some(top)).map(|r| r.mime).collect()
}

#[cfg(test)]
mod tests {

    fn fifo(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("vergence-types-fifo-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let ok = std::process::Command::new("mkfifo").arg(&p).status().is_ok_and(|s| s.success());
        assert!(ok, "mkfifo");
        p
    }

    #[test]
    fn a_named_pipe_is_refused_without_waiting() {
        let p = fifo("open");
        let started = std::time::Instant::now();
        assert_eq!(open_regular(&p).map(|_| ()).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(of_file(&p), BINARY, "a pipe with no type in its name is a binary file");
        let _ = std::fs::remove_file(&p);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "returned at once");
    }

    #[test]
    fn devices_are_refused_and_files_are_read_up_to_the_limit() {
        assert!(open_regular(Path::new("/dev/zero")).is_err(), "a device reports length 0 and never ends");
        assert!(open_regular(Path::new("/dev/null")).is_err());
        let p = std::env::temp_dir().join(format!("vergence-types-reg-{}", std::process::id()));
        std::fs::write(&p, b"hello").unwrap();
        assert_eq!(read_regular(&p, 5).unwrap(), b"hello");
        assert_eq!(read_regular(&p, 4).unwrap_err().kind(), std::io::ErrorKind::FileTooLarge);
        let _ = std::fs::remove_file(&p);
    }
    use super::*;

    #[test]
    fn every_row_is_well_formed() {
        for r in table::ROWS {
            assert_eq!(r.mime, r.mime.to_ascii_lowercase(), "{}", r.mime);
            assert!(r.mime.contains('/'), "{}", r.mime);
            if let Some(p) = r.parent {
                assert!(row(p).is_some(), "{}'s parent {p} is in the table", r.mime);
            }
            assert_eq!(table::ROWS.iter().filter(|o| o.mime == r.mime).count(), 1, "{} once", r.mime);
        }
        for (a, to) in table::ALIASES {
            assert!(row(to).is_some(), "alias {a} -> {to}");
        }
    }

    #[test]
    fn names_say_the_type_longest_pattern_first() {
        assert_eq!(of_name("Photo.JPG"), Some("image/jpeg"));
        assert_eq!(of_name("notes.md"), Some("text/markdown"));
        assert_eq!(of_name("backup.tar.gz"), Some("application/x-compressed-tar"));
        assert_eq!(of_name("data.gz"), Some("application/gzip"));
        assert_eq!(of_name("Makefile"), Some("text/x-makefile"));
        assert_eq!(of_name("makefile.bak"), Some("application/x-trash"));
        assert_eq!(of_name("main.rs"), Some("text/rust"));
        assert_eq!(of_name("Cargo.lock"), Some("application/toml"));
        assert_eq!(of_name(".png"), None, "a dot file is not a picture");
        assert_eq!(of_name("noextension"), None);
        assert_eq!(of_name("readme"), None, "whole names are exact");
    }

    #[test]
    fn first_bytes_say_the_type() {
        assert_eq!(of_bytes(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some("image/png"));
        assert_eq!(of_bytes(b"\xff\xd8\xff\xe0\0\x10JFIF"), Some("image/jpeg"));
        assert_eq!(of_bytes(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(of_bytes(b"RIFF\0\0\0\0WAVEfmt "), Some("audio/x-wav"));
        assert_eq!(of_bytes(b"\0\0\0\x18ftypheic\0\0\0\0"), Some("image/heif"));
        assert_eq!(of_bytes(b"\0\0\0\x18ftypisom\0\0\0\0"), Some("video/mp4"));
        assert_eq!(of_bytes(b"%PDF-1.7\n"), Some("application/pdf"));
        assert_eq!(of_bytes(b"\x7fELF\x02\x01\x01"), Some("application/x-executable"));
        let mut bmp = b"BM\0\0\0\0\0\0\0\0\x36\0\0\0\x28\0\0\0".to_vec();
        bmp.extend([0u8; 8]);
        assert_eq!(of_bytes(&bmp), Some("image/bmp"));
        assert_eq!(of_bytes(b"BM is not enough"), Some(TEXT), "\"BM\" text is text");
        let mut tar = vec![0u8; 512];
        tar[..5].copy_from_slice(b"hello");
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(of_bytes(&tar), Some("application/x-tar"));
        assert_eq!(of_bytes(b"#!/usr/bin/env python3\nprint()"), Some("text/x-python"));
        assert_eq!(of_bytes(b"#!/bin/sh\necho"), Some("application/x-shellscript"));
        assert_eq!(of_bytes(b"<?xml version=\"1.0\"?>\n<svg xmlns="), Some("image/svg+xml"));
        assert_eq!(of_bytes(b"<!DOCTYPE html><html>"), Some("text/html"));
        assert_eq!(of_bytes("caf\u{e9} au lait".as_bytes()), Some(TEXT));
        assert_eq!(of_bytes(b""), Some(TEXT), "empty is text");
        assert_eq!(of_bytes(b"\0\x01\x02garbage"), None);
        assert_eq!(of_bytes(&"é".as_bytes()[..1]), Some(TEXT), "a character cut at the end is still text");
        assert_eq!(of_bytes(b"\xff\xfe\xfd not utf-8"), None);
    }

    #[test]
    fn a_file_is_its_name_then_its_bytes() {
        let dir = std::env::temp_dir().join(format!("vergence-types-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("picture");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nrest").unwrap();
        let named = dir.join("notes.txt");
        std::fs::write(&named, b"\x89PNG\r\n\x1a\n").unwrap();
        let blob = dir.join("blob");
        std::fs::write(&blob, [0u8, 1, 2, 3]).unwrap();
        assert_eq!(of_file(&png), "image/png");
        assert_eq!(of_file(&named), "text/plain", "the name wins");
        assert_eq!(of_file(&blob), BINARY);
        assert_eq!(of_file(&dir), FOLDER);
        assert_eq!(of_file(&dir.join("missing.pdf")), "application/pdf");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn kinds_of_kinds() {
        assert_eq!(ancestors("application/x-shellscript"), vec!["application/x-shellscript", "text/plain", BINARY]);
        assert_eq!(ancestors("image/svg+xml"), vec!["image/svg+xml", "application/xml", "text/plain", BINARY]);
        assert_eq!(ancestors("text/rust"), vec!["text/rust", "text/plain", BINARY]);
        assert_eq!(ancestors(FOLDER), vec![FOLDER]);
        assert!(is_a("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "application/zip"));
        assert!(is_a("image/jpg", "image/jpeg"), "aliases");
        assert!(!is_a("image/png", "text/plain"));
        assert_eq!(canonical("Image/JPG"), "image/jpeg");
        assert_eq!(describe("image/png"), "PNG picture");
        assert_eq!(icons("image/png"), ["image-png".to_string(), "image-x-generic".to_string()]);
        assert_eq!(icons("text/x-unknown")[1], "text-x-generic");
    }

    #[test]
    fn a_list_file_round_trips_and_bad_lines_are_skipped() {
        let text = "# comment\n[Default Applications]\nimage/png=closeup.desktop;gimp.desktop;\ntext/plain=vergence-edit.desktop\nnot a line\n\n[Added Associations]\nimage/jpg=closeup.desktop;\n[Removed Associations]\ntext/plain=libreoffice.desktop;\n[Other]\nx/y=z.desktop\n";
        let l = MimeApps::parse(text);
        assert_eq!(
            l.defaults,
            vec![
                ("image/png".into(), vec!["closeup.desktop".into(), "gimp.desktop".into()]),
                ("text/plain".into(), vec!["vergence-edit.desktop".into()])
            ]
        );
        assert_eq!(l.added, vec![("image/jpeg".into(), vec!["closeup.desktop".into()])], "aliases resolved");
        assert_eq!(l.removed.len(), 1);
        assert_eq!(MimeApps::parse(&l.to_text()), l);
        let mut l = l;
        l.set_default("text/plain", "libreoffice.desktop");
        assert_eq!(MimeApps::of(&l.defaults, "text/plain").collect::<Vec<_>>(), vec!["libreoffice.desktop"]);
        assert!(l.removed.is_empty(), "chosen again: no longer removed");
    }

    #[test]
    fn the_apps_for_a_type_best_first() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (edit, closeup, gimp, office, hex, gone) = (
            t(&["text/plain"]),
            t(&["image/png", "image/jpeg"]),
            t(&["image/*"]),
            t(&["text/plain", "application/msword"]),
            t(&["application/octet-stream"]),
            t(&[]),
        );
        let apps = [
            App { id: "vergence-edit.desktop", types: &edit },
            App { id: "closeup.desktop", types: &closeup },
            App { id: "gimp.desktop", types: &gimp },
            App { id: "office.desktop", types: &office },
            App { id: "hex.desktop", types: &hex },
            App { id: "other.desktop", types: &gone },
        ];
        let ours = MimeApps { vergence: true, ..MimeApps::parse("[Default Applications]\nimage/png=gimp.desktop;\n") };
        let shared = MimeApps::parse(
            "[Default Applications]\nimage/png=closeup.desktop;\ntext/plain=uninstalled.desktop;office.desktop;\n[Removed Associations]\napplication/x-shellscript=office.desktop;\n",
        );
        let lists = [ours, shared];
        assert_eq!(
            apps_for("image/png", &apps, &lists),
            vec!["gimp.desktop", "closeup.desktop", "hex.desktop"],
            "ours first; image/* matches; binary last"
        );
        assert_eq!(
            apps_for("text/plain", &apps, &lists),
            vec!["vergence-edit.desktop", "office.desktop", "hex.desktop"],
            "Vergence's own app before another desktop's choice; an uninstalled default is passed over"
        );
        let gnome = MimeApps::parse("[Default Applications]\ntext/markdown=gnome-edit.desktop;\n");
        let md = t(&["text/markdown"]);
        let with_gnome =
            [App { id: "vergence-edit.desktop", types: &edit }, App { id: "gnome-edit.desktop", types: &md }];
        assert_eq!(
            default_for("text/markdown", &with_gnome, std::slice::from_ref(&gnome)).as_deref(),
            Some("vergence-edit.desktop"),
            "Markdown is a kind of text: ours, not GNOME's choice"
        );
        let chosen = MimeApps {
            vergence: true,
            ..MimeApps::parse("[Default Applications]\ntext/markdown=gnome-edit.desktop;\n")
        };
        assert_eq!(
            default_for("text/markdown", &with_gnome, &[chosen, gnome]).as_deref(),
            Some("gnome-edit.desktop"),
            "a choice made in Vergence beats our own apps"
        );
        assert_eq!(
            apps_for("application/x-shellscript", &apps, &lists),
            vec!["vergence-edit.desktop", "hex.desktop"],
            "a script opens with text apps, less the removed"
        );
        assert_eq!(apps_for("image/jpg", &apps, &[]), vec!["closeup.desktop", "gimp.desktop", "hex.desktop"]);
        assert_eq!(
            apps_for("application/x-unknown", &apps, &[]),
            vec!["hex.desktop"],
            "only apps that list binary files by name"
        );
    }

    #[test]
    fn links_are_opened_by_their_scheme() {
        assert_eq!(of_link("https://example.org/a").as_deref(), Some("x-scheme-handler/https"));
        assert_eq!(of_link("MAILTO:someone@example.org").as_deref(), Some("x-scheme-handler/mailto"));
        assert_eq!(of_link("file:///tmp/a"), None);
        assert_eq!(of_link("/tmp/a:b"), None);
        assert_eq!(of_link("notes.txt"), None);
        assert_eq!(ancestors("x-scheme-handler/https"), vec!["x-scheme-handler/https"]);
        assert_eq!(path_of_link("file:///tmp/a%20b.txt"), Some(PathBuf::from("/tmp/a b.txt")));
        assert_eq!(path_of_link("file://localhost/etc/hosts"), Some(PathBuf::from("/etc/hosts")));
        assert_eq!(path_of_link("file://otherhost/etc/hosts"), None);
        assert_eq!(path_of_link("https://x/y"), None);
    }

    #[test]
    fn vergence_choices_go_in_its_own_list() {
        assert!(own_list_path().ends_with("vergence-mimeapps.list"));
    }
}
