// Only one System Settings at a time (see docs/apps-roadmap.md). The first
// vergence-settings listens on a socket of its own (owner-only, in the
// runtime directory, as the editor's hand.rs); a second start hands its
// arguments (--page, --app, --forget) to it and exits; the running one turns
// to the page asked for and comes forward.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

const MAX_LINE: usize = 4096;

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("vergence-settings.sock"))
}

/// The arguments as one line: "args", then each, tab-separated.
pub fn encode(args: &[String]) -> String {
    let mut f = vec!["args".to_string()];
    f.extend(args.iter().map(|a| a.replace(['\t', '\n'], " ")));
    f.join("\t") + "\n"
}

pub fn decode(line: &str) -> Option<Vec<String>> {
    if line.len() > MAX_LINE {
        return None;
    }
    let mut f = line.trim_end_matches('\n').split('\t');
    (f.next()? == "args").then(|| f.take(16).map(str::to_string).collect())
}

/// Hands `args` to a running System Settings. True if one took them.
pub fn hand_over(args: &[String]) -> bool {
    let Some(path) = socket_path() else { return false };
    let Ok(mut s) = UnixStream::connect(&path) else { return false };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    if s.write_all(encode(args).as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Becomes the one others hand over to: each start's arguments are sent as
/// `to_message(args)` through `send`.
pub fn listen(send: impl Fn(Vec<String>) + Send + 'static) {
    let Some(path) = socket_path() else { return };
    // A socket nobody answers on is left from one that went away.
    if UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(listener) = UnixListener::bind(&path) else { return };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let _ = std::thread::Builder::new().name("handover".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            if BufReader::new((&stream).take(MAX_LINE as u64 + 1)).read_line(&mut line).is_ok()
                && let Some(args) = decode(&line)
            {
                let _ = (&stream).write_all(b"ok\n");
                send(args);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_go_over_whole() {
        let a = vec!["--page".to_string(), "date & time".to_string()];
        assert_eq!(decode(&encode(&a)), Some(a));
        assert_eq!(decode("open\tx\n"), None);
        assert_eq!(decode(&encode(&[])), Some(vec![]));
    }
}
