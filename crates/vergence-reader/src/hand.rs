//! One Reader process for all its windows (files milestone 1, as the
//! editor's hand.rs): the first `vergence-reader` listens on a socket of
//! its own (owner-only, in the runtime directory); starting it again hands
//! its files to that one, which opens them in tabs, and exits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The most files one request names, and the longest path.
const MAX_FILES: usize = 256;
const MAX_PATH: usize = 4096;

fn socket_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    Some(dir.join("vergence-reader.sock"))
}

/// A second start's files, as one line: "open", then each path, tab-separated.
pub fn encode(files: &[PathBuf]) -> String {
    let mut fields = vec!["open".to_string()];
    fields.extend(files.iter().map(|f| f.to_string_lossy().replace(['\t', '\n'], " ")));
    fields.join("\t") + "\n"
}

pub fn decode(line: &str) -> Option<Vec<PathBuf>> {
    let mut f = line.trim_end_matches('\n').split('\t');
    if f.next()? != "open" {
        return None;
    }
    Some(f.filter(|p| !p.is_empty() && p.len() <= MAX_PATH).take(MAX_FILES).map(PathBuf::from).collect())
}

/// Asks a running Reader to open `files` (none: a new window). True if one said yes.
pub fn hand_over(files: &[PathBuf]) -> bool {
    let Some(path) = socket_path() else { return false };
    let Ok(mut s) = UnixStream::connect(&path) else { return false };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    if s.write_all(encode(files).as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Becomes the Reader others hand over to: each request's files go to
/// `inbox` and `wake` is called. False if the socket could not be made.
pub fn listen(inbox: Arc<Mutex<Vec<Vec<PathBuf>>>>, wake: impl Fn() + Send + 'static) -> bool {
    let Some(path) = socket_path() else { return false };
    // A socket nobody answers on is left from one that went away.
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else { return false };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    std::thread::Builder::new()
        .name("handover".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                if BufReader::new(&stream).read_line(&mut line).is_ok()
                    && line.len() <= MAX_FILES * (MAX_PATH + 1) + 8
                    && let Some(files) = decode(&line)
                {
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(files);
                    let _ = (&stream).write_all(b"ok\n");
                    wake();
                }
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_survives_the_line() {
        let files = vec![PathBuf::from("/home/a/one.txt"), PathBuf::from("/tmp/with space.md")];
        assert_eq!(decode(&encode(&files)), Some(files));
        assert_eq!(decode(&encode(&[])), Some(vec![]));
        assert_eq!(decode("close\t/x\n"), None);
    }
}
