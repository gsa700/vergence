// The desktop portal's file chooser and link opener on the D-Bus edge
// (files milestone 1, step 3; docs/files-milestone-1.md §2).
//
// org.freedesktop.portal.FileChooser: OpenFile, SaveFile and SaveFiles each
// answer at once with a Request object's path, start our dialog
// (libexec/vergence/vergence-choose, vergence-widgets' FileDialog) and, when
// it closes, send that Request's Response signal to the caller alone: 0 and
// the chosen files' `uris`, 1 if cancelled, 2 if it failed. The dialog's
// window is adopted by the caller's window (the window of the caller's
// process with the keyboard, else its newest; `x11:<id>` parents are found
// by id): a modal dialog over it, as one of the app's own would be
// (dialogs.rs). Request.Close ends the dialog. Options read: handle_token,
// accept_label, multiple, directory, filters, current_filter, current_name,
// current_folder, current_file (SaveFile), files (SaveFiles). Not yet:
// `choices` (extra check boxes; none are returned).
//
// org.freedesktop.portal.OpenURI: OpenURI opens a link or a file:// link
// with the app chosen for its type (shelf.rs `open_items`, as the bus's
// Open). OpenFile and OpenDirectory pass the file as a descriptor (the edge
// takes descriptors since 2026-11-17): its path is read from the kernel
// (/proc/self/fd on Linux, F_KINFO on FreeBSD); OpenFile opens it by type,
// OpenDirectory opens the folder it is in (a folder itself, if one was
// passed). Before, the edge refused descriptors, GLib sent one anyway and
// then ended its connection, which ended Firefox ("show in folder" from
// its downloads panel, Hubble, 2026-11-17).
//
// Our apps are not sandboxed: the answer is the file's own file:// link; the
// Documents portal (copies for sandboxed apps) is not provided.
//
// The session environment (main.rs): with desktop.toml `[apps]
// own-file-dialog = true`, the server sets GTK_USE_PORTAL=1,
// GDK_DEBUG=portals and QT_QPA_PLATFORMTHEME=xdgdesktopportal for every app
// it starts (where not set already), so GTK and Qt apps ask the portal
// (answer 6, Dave 2026-11-15: behind a switch, on once Firefox, a GTK and a
// Qt app pass).

use std::collections::HashMap;
use std::path::PathBuf;

use smithay::reexports::calloop::channel;
use vergence_dbus::Value;

use crate::{Server, win::Win};

pub const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
pub const OPEN_URI: &str = "org.freedesktop.portal.OpenURI";
pub const REQUEST: &str = "org.freedesktop.portal.Request";
pub const REQUEST_PREFIX: &str = "/org/freedesktop/portal/desktop/request/";
/// The most dialogs open at once (a program asking in a loop is refused).
const MAX_CHOOSERS: usize = 8;

/// A dialog open for a portal call.
pub struct Chooser {
    /// The D-Bus connection that asked, and its Request object's path.
    pub conn: u32,
    pub handle: String,
    /// The window its dialog is a dialog of.
    pub parent: Option<Win>,
    /// SaveFiles: the names to save, in the folder chosen.
    pub save_files: Option<Vec<String>>,
}

/// The portal's state in the server.
#[derive(Default)]
pub struct Portal {
    /// Open dialogs, by the helper's process id.
    pub choosers: HashMap<u32, Chooser>,
    /// A dialog's window and the window of another program it belongs to.
    pub adopted: HashMap<Win, Win>,
    /// Where the helpers' answers come back (pid, exit code, output).
    done: Option<channel::Sender<(u32, Option<i32>, String)>>,
    /// Request tokens made up for calls without one.
    serial: u32,
}

/// An option of an a{sv} by name, its variant unwrapped.
fn option<'a>(options: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Array(_, entries) = options else { return None };
    entries.iter().find_map(|e| match e {
        Value::Entry(k, v) if k.as_str() == Some(key) => Some(match v.as_ref() {
            Value::Variant(inner) => inner.as_ref(),
            other => other,
        }),
        _ => None,
    })
}

fn opt_str(options: &Value, key: &str) -> Option<String> {
    option(options, key).and_then(Value::as_str).map(str::to_owned)
}

fn opt_bool(options: &Value, key: &str) -> bool {
    matches!(option(options, key), Some(Value::Bool(true)))
}

/// A byte string (`ay`, NUL-ended as the portal sends paths) as a path.
fn bytes_path(v: Option<&Value>) -> Option<PathBuf> {
    let Some(Value::Array(_, items)) = v else { return None };
    let mut bytes: Vec<u8> = items.iter().filter_map(|b| if let Value::Byte(b) = b { Some(*b) } else { None }).collect();
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    use std::os::unix::ffi::OsStringExt;
    (!bytes.is_empty()).then(|| PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// A filter `(sa(us))` as vergence-choose's argument: "Name|glob:*.txt|mime:text/plain".
fn filter_arg(v: &Value) -> Option<String> {
    let Value::Struct(parts) = v else { return None };
    let name = parts.first()?.as_str()?.replace('|', "/");
    let mut out = name;
    if let Some(Value::Array(_, rules)) = parts.get(1) {
        for r in rules {
            let Value::Struct(rule) = r else { continue };
            let (Some(Value::U32(kind)), Some(pattern)) = (rule.first(), rule.get(1).and_then(Value::as_str)) else { continue };
            if pattern.contains('|') {
                continue;
            }
            out.push_str(if *kind == 0 { "|glob:" } else { "|mime:" });
            out.push_str(pattern);
        }
    }
    Some(out)
}

/// A path as a file:// link (bytes outside the safe set %-escaped).
pub fn file_uri(p: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The Request object's path for a caller and token, as the portal spec makes it.
pub fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    let token: String = token.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(64).collect();
    format!("{REQUEST_PREFIX}{sender}/{token}")
}

/// The dialog helper: beside the server (a checkout's target/), else
/// PREFIX/libexec/vergence, else /usr/local's.
fn helper() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("vergence-choose"), dir.parent()?.join("libexec/vergence/vergence-choose"), PathBuf::from("/usr/local/libexec/vergence/vergence-choose")].into_iter().find(|p| p.is_file())
}

/// Whether desktop.toml asks for our dialog in other apps.
pub fn own_dialog_wanted() -> bool {
    let text = crate::wallpaper::desktop_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let table = text.parse::<toml::Table>().unwrap_or_default();
    table.get("apps").and_then(|a| a.get("own-file-dialog")).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// The environment that makes GTK and Qt apps ask the portal (main.rs, at start).
pub fn export_env() {
    if !own_dialog_wanted() {
        return;
    }
    for (k, v) in [("GTK_USE_PORTAL", "1"), ("GDK_DEBUG", "portals"), ("QT_QPA_PLATFORMTHEME", "xdgdesktopportal")] {
        if std::env::var_os(k).is_none() {
            // SAFETY: at start, before any thread that reads the environment.
            unsafe { std::env::set_var(k, v) };
        }
    }
    tracing::info!("portal: apps not ours are asked to use our file dialog ([apps] own-file-dialog)");
}

impl Server {
    /// The caller's window: an `x11:<id>` parent if given and found; else
    /// the window of its process with the keyboard; else its newest.
    fn caller_window(&self, conn: u32, parent_window: &str) -> Option<Win> {
        if let Some(id) = parent_window.strip_prefix("x11:").and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok())
            && let Some(w) = self.space.elements().find(|e| e.x11_surface().is_some_and(|x| x.window_id() == id))
        {
            return Some(w.clone());
        }
        let pid = self.dbus_conn_pid(conn)?;
        if let Some(f) = self.keyboard_window().filter(|f| self.window_pid(f) == Some(pid)) {
            return Some(f);
        }
        self.space.elements().filter(|e| self.window_pid(e) == Some(pid)).last().cloned()
    }

    /// A FileChooser call: the dialog started, the Request's path returned.
    pub fn portal_choose(&mut self, conn: u32, unique: &str, member: &str, args: &[Value]) -> Result<String, String> {
        if self.portal.choosers.len() >= MAX_CHOOSERS {
            return Err("too many file dialogs open".into());
        }
        let parent_window = args.first().and_then(Value::as_str).unwrap_or_default().to_string();
        let title = args.get(1).and_then(Value::as_str).unwrap_or_default().to_string();
        let empty = Value::Array("{sv}".into(), Vec::new());
        let options = args.get(2).unwrap_or(&empty);
        let token = opt_str(options, "handle_token").unwrap_or_else(|| {
            self.portal.serial += 1;
            format!("vergence{}", self.portal.serial)
        });
        let handle = request_path(unique, &token);
        let Some(helper) = helper() else { return Err("the file dialog (libexec/vergence/vergence-choose) is not installed".into()) };
        let mut cmd = std::process::Command::new(helper);
        let mut save_files = None;
        match member {
            "OpenFile" if opt_bool(options, "directory") => {
                cmd.arg("--folder");
            }
            "OpenFile" => {
                cmd.arg("--open");
                if opt_bool(options, "multiple") {
                    cmd.arg("--multiple");
                }
            }
            "SaveFile" => {
                cmd.arg("--save");
                // current_file: an existing file being saved again (its folder and name).
                let file = bytes_path(option(options, "current_file"));
                if let Some(name) = opt_str(options, "current_name").or_else(|| file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned())) {
                    cmd.args(["--name", &name]);
                }
                if let Some(dir) = file.as_ref().and_then(|f| f.parent()) {
                    cmd.arg("--in").arg(dir);
                }
            }
            "SaveFiles" => {
                // Several files into one folder: the folder is chosen.
                cmd.arg("--folder");
                let names: Vec<String> = match option(options, "files") {
                    Some(Value::Array(_, files)) => files.iter().filter_map(|f| bytes_path(Some(f))).filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect(),
                    _ => Vec::new(),
                };
                save_files = Some(names);
            }
            _ => return Err(format!("{member} is not a FileChooser method")),
        }
        if !title.is_empty() {
            cmd.args(["--title", &title]);
        }
        if let Some(word) = opt_str(options, "accept_label") {
            cmd.args(["--accept", &word]);
        }
        if let Some(dir) = bytes_path(option(options, "current_folder")) {
            cmd.arg("--in").arg(dir);
        }
        if let Some(Value::Array(_, filters)) = option(options, "filters") {
            let current = option(options, "current_filter").and_then(filter_arg);
            for (i, f) in filters.iter().filter_map(filter_arg).enumerate() {
                if current.as_deref() == Some(f.as_str()) {
                    cmd.args(["--current", &i.to_string()]);
                }
                cmd.args(["--filter", &f]);
            }
        }
        let app = self.dbus_conn_pid(conn).and_then(process_name).unwrap_or_else(|| "app".into());
        cmd.args(["--app", &app]);
        let parent = self.caller_window(conn, &parent_window);
        let sender = self.portal_channel();
        crate::no_terminal(&mut cmd);
        let mut child = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).spawn().map_err(|e| format!("the file dialog could not be started: {e}"))?;
        let pid = child.id();
        std::thread::spawn(move || {
            let mut out = String::new();
            if let Some(mut so) = child.stdout.take() {
                use std::io::Read;
                let _ = so.by_ref().take(1 << 20).read_to_string(&mut out);
            }
            let code = child.wait().ok().and_then(|s| s.code());
            let _ = sender.send((pid, code, out));
        });
        tracing::info!("portal: {member} for {app} (pid {pid}){}: {:?}", if parent.is_some() { ", over its window" } else { "" }, opts_seen(options));
        self.portal.choosers.insert(pid, Chooser { conn, handle: handle.clone(), parent, save_files });
        Ok(handle)
    }

    /// The channel the helpers' answers come back on, made once.
    fn portal_channel(&mut self) -> channel::Sender<(u32, Option<i32>, String)> {
        if let Some(s) = &self.portal.done {
            return s.clone();
        }
        let (tx, rx) = channel::channel::<(u32, Option<i32>, String)>();
        let inserted = self.loop_handle.insert_source(rx, |event, _, state| {
            if let channel::Event::Msg((pid, code, out)) = event {
                state.portal_answer(pid, code, &out);
            }
        });
        if let Err(e) = inserted {
            tracing::warn!("portal: {e}");
        }
        self.portal.done = Some(tx.clone());
        tx
    }

    /// A dialog closed: its Request's Response to the caller.
    fn portal_answer(&mut self, pid: u32, code: Option<i32>, out: &str) {
        let Some(c) = self.portal.choosers.remove(&pid) else { return };
        // Its window is gone or going: forget whose it was (and any that died).
        self.portal.adopted.retain(|w, parent| smithay::utils::IsAlive::alive(w) && Some(&*parent) != c.parent.as_ref());
        let chosen: Vec<PathBuf> = out.lines().filter(|l| l.starts_with('/')).map(PathBuf::from).collect();
        let (response, uris): (u32, Vec<String>) = match code {
            Some(0) if !chosen.is_empty() => {
                let paths: Vec<PathBuf> = match &c.save_files {
                    // SaveFiles: each name in the folder chosen.
                    Some(names) => names.iter().map(|n| chosen[0].join(n)).collect(),
                    None => chosen,
                };
                (0, paths.iter().map(|p| file_uri(p)).collect())
            }
            Some(1) => (1, Vec::new()),
            _ => (2, Vec::new()),
        };
        tracing::info!("portal: dialog {pid} answered {response} ({} file(s))", uris.len());
        let results = Value::Array("{sv}".into(), vec![Value::Entry(Box::new(Value::Str("uris".into())), Box::new(Value::Variant(Box::new(Value::strings(uris)))))]);
        self.dbus_request_response(c.conn, &c.handle, response, results);
    }

    /// Request.Close: the caller gave up; the dialog goes.
    pub fn portal_close(&mut self, handle: &str) {
        if let Some((&pid, _)) = self.portal.choosers.iter().find(|(_, c)| c.handle == handle) {
            unsafe extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            // SAFETY: kill(2) on our own helper's pid; no memory passed.
            unsafe { kill(pid as i32, 15) };
            self.portal.choosers.remove(&pid);
        }
    }

    /// OpenURI.OpenURI: the link opened by type; the Response sent at once.
    pub fn portal_open_uri(&mut self, conn: u32, unique: &str, args: &[Value]) -> Result<String, String> {
        let uri = args.get(1).and_then(Value::as_str).ok_or("OpenURI(ssa{sv})")?.to_string();
        let empty = Value::Array("{sv}".into(), Vec::new());
        let options = args.get(2).unwrap_or(&empty);
        let token = opt_str(options, "handle_token").unwrap_or_else(|| {
            self.portal.serial += 1;
            format!("vergence{}", self.portal.serial)
        });
        let handle = request_path(unique, &token);
        let response = match self.open_items(&[uri.clone()], "") {
            Ok(()) => 0,
            Err(e) => {
                tracing::info!("portal: OpenURI {uri}: {e}");
                2
            }
        };
        self.dbus_request_response(conn, &handle, response, Value::Array("{sv}".into(), Vec::new()));
        Ok(handle)
    }

    /// OpenURI.OpenFile / OpenDirectory: the file passed as descriptor
    /// `args[1]` (an index into `fds`), opened by type, or the folder it is
    /// in shown; the Response sent at once, as OpenURI's.
    pub fn portal_open_fd(&mut self, conn: u32, unique: &str, directory: bool, args: &[Value], fds: &[std::os::fd::OwnedFd]) -> Result<String, String> {
        let Some(Value::Fd(i)) = args.get(1) else { return Err("(sha{sv})".into()) };
        let fd = fds.get(*i as usize).ok_or("no such file descriptor")?;
        let path = fd_path(fd).ok_or("the file's path could not be found")?;
        let empty = Value::Array("{sv}".into(), Vec::new());
        let options = args.get(2).unwrap_or(&empty);
        let token = opt_str(options, "handle_token").unwrap_or_else(|| {
            self.portal.serial += 1;
            format!("vergence{}", self.portal.serial)
        });
        let handle = request_path(unique, &token);
        let target = if directory && !path.is_dir() { path.parent().map(PathBuf::from).unwrap_or(path) } else { path };
        let what = if directory { "OpenDirectory" } else { "OpenFile" };
        let response = match self.open_items(&[target.to_string_lossy().into_owned()], "") {
            Ok(()) => 0,
            Err(e) => {
                tracing::info!("portal: {what}: {e}");
                2
            }
        };
        self.dbus_request_response(conn, &handle, response, Value::Array("{sv}".into(), Vec::new()));
        Ok(handle)
    }

    /// A window just made by a bus client: if it is a portal dialog's (by
    /// the helper's pid), it belongs to the caller's window.
    pub fn portal_adopt(&mut self, w: &Win, pid: Option<u32>) {
        let Some(parent) = pid.and_then(|p| self.portal.choosers.get(&p)).and_then(|c| c.parent.clone()) else { return };
        self.portal.adopted.insert(w.clone(), parent);
    }
}

/// The path an open file descriptor is for, as the kernel knows it.
fn fd_path(fd: &std::os::fd::OwnedFd) -> Option<PathBuf> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    {
        let p = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()?;
        p.is_absolute().then_some(p)
    }
    #[cfg(target_os = "freebsd")]
    {
        // fcntl(F_KINFO) fills a struct kinfo_file (sys/user.h): its size
        // first, its path, PATH_MAX bytes, last.
        const F_KINFO: i32 = 22;
        const KINFO_FILE_SIZE: usize = 1392;
        const PATH_MAX: usize = 1024;
        unsafe extern "C" {
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        }
        let mut buf = vec![0u8; KINFO_FILE_SIZE];
        buf[..4].copy_from_slice(&(KINFO_FILE_SIZE as i32).to_ne_bytes());
        // SAFETY: a buffer of KINFO_FILE_SIZE bytes, its size in its first field, as F_KINFO asks.
        if unsafe { fcntl(fd.as_raw_fd(), F_KINFO, buf.as_mut_ptr()) } != 0 {
            return None;
        }
        let raw = &buf[KINFO_FILE_SIZE - PATH_MAX..];
        let end = raw.iter().position(|&b| b == 0)?;
        use std::os::unix::ffi::OsStrExt;
        let p = PathBuf::from(std::ffi::OsStr::from_bytes(&raw[..end]));
        p.is_absolute().then_some(p)
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = fd;
        None
    }
}

/// The option names a call carried, for the log (not their values: paths).
fn opts_seen(options: &Value) -> Vec<String> {
    let Value::Array(_, entries) = options else { return Vec::new() };
    entries.iter().filter_map(|e| if let Value::Entry(k, _) = e { Some(k.as_str().unwrap_or("?").to_string()) } else { None }).collect()
}

/// A process's name (its command), for the dialog's remembered folder.
pub(crate) fn process_name(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).stderr(std::process::Stdio::null()).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_gives_its_file_path() {
        let dir = std::env::temp_dir().join(format!("vergence-fdpath-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a file.txt");
        std::fs::write(&file, b"hi").unwrap();
        let f: std::os::fd::OwnedFd = std::fs::File::open(&file).unwrap().into();
        // The kernel's path may be the real one behind a link (/tmp on some systems).
        let want = std::fs::canonicalize(&file).unwrap();
        assert_eq!(fd_path(&f).map(|p| std::fs::canonicalize(p).unwrap()), Some(want));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn options_filters_and_paths_are_read() {
        let entry = |k: &str, v: Value| Value::Entry(Box::new(Value::Str(k.into())), Box::new(Value::Variant(Box::new(v))));
        let filter = |name: &str, rules: Vec<(u32, &str)>| Value::Struct(vec![Value::Str(name.into()), Value::Array("(us)".into(), rules.into_iter().map(|(k, p)| Value::Struct(vec![Value::U32(k), Value::Str(p.into())])).collect())]);
        let folder: Vec<Value> = b"/home/u/Pictures\0".iter().map(|b| Value::Byte(*b)).collect();
        let opts = Value::Array(
            "{sv}".into(),
            vec![
                entry("handle_token", Value::Str("t1".into())),
                entry("multiple", Value::Bool(true)),
                entry("filters", Value::Array("(sa(us))".into(), vec![filter("Images", vec![(0, "*.png"), (1, "image/jpeg")]), filter("All", vec![(0, "*")])])),
                entry("current_folder", Value::Array("y".into(), folder)),
            ],
        );
        assert_eq!(opt_str(&opts, "handle_token").as_deref(), Some("t1"));
        assert!(opt_bool(&opts, "multiple"));
        assert!(!opt_bool(&opts, "directory"));
        assert_eq!(bytes_path(option(&opts, "current_folder")), Some(PathBuf::from("/home/u/Pictures")));
        let Some(Value::Array(_, fs)) = option(&opts, "filters") else { panic!() };
        assert_eq!(filter_arg(&fs[0]).as_deref(), Some("Images|glob:*.png|mime:image/jpeg"));
        assert_eq!(request_path(":1.42", "t1"), "/org/freedesktop/portal/desktop/request/1_42/t1");
        assert_eq!(request_path(":1.42", "a-b"), "/org/freedesktop/portal/desktop/request/1_42/ab", "only letters, digits and _");
        assert_eq!(file_uri(std::path::Path::new("/home/u/a b#c.txt")), "file:///home/u/a%20b%23c.txt");
        assert_eq!(vergence_types::path_of_link(&file_uri(std::path::Path::new("/tmp/é x"))), Some(PathBuf::from("/tmp/é x")), "round trip");
    }
}
