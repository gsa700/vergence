//! `vergence-remote`: the host's side of Vergence Remote, run by SSH for a
//! viewer (`ssh <host> vergence-remote`). Its standard input and output are
//! the channel to the viewer; it connects to the running session's remote
//! socket (crates/vergence-server/src/remote.rs) and relays between the two
//! in Vergence Remote's protocol (docs/remote-protocol.md), compressing
//! frames on their way out (the server sends them uncompressed on the local
//! leg, so its loop never spends time on that).
//!
//! It checks both sides' messages as it relays (each is decoded and written
//! again), so nothing the viewer sends reaches the server unchecked.

use std::io::{BufReader, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use vergence_remote::proto::{self, Rect, ToHost, ToViewer};

fn say_bye(why: &str) -> ExitCode {
    let mut out = BufWriter::new(std::io::stdout().lock());
    let _ = proto::write_opening(&mut out);
    let _ = ToViewer::Bye { reason: why.to_string() }.write(&mut out);
    let _ = out.flush();
    eprintln!("vergence-remote: {why}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("vergence-remote, protocol version {}", proto::VERSION);
        return ExitCode::SUCCESS;
    }
    let path = match vergence_remote::find_socket() {
        Ok(p) => p,
        Err(e) => return say_bye(&e),
    };
    let local = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => return say_bye(&format!("{}: {e}", path.display())),
    };
    let Ok(local_w) = local.try_clone() else { return say_bye("no second handle on the socket") };
    let mut local_r = BufReader::with_capacity(1 << 20, local);
    let mut local_w = BufWriter::new(local_w);
    if let Err(e) =
        proto::write_opening(&mut local_w).map_err(proto::Error::Io).and_then(|_| proto::read_opening(&mut local_r))
    {
        return say_bye(&format!("the session: {e}"));
    }
    let mut viewer_r = BufReader::new(std::io::stdin());
    let mut viewer_w = BufWriter::with_capacity(1 << 20, std::io::stdout());
    if proto::write_opening(&mut viewer_w).is_err() {
        return ExitCode::from(1);
    }
    if let Err(e) = proto::read_opening(&mut viewer_r) {
        eprintln!("vergence-remote: the viewer: {e}");
        return ExitCode::from(1);
    }
    // Viewer to session, on a thread of its own.
    let up = std::thread::spawn(move || {
        while let Ok(m) = ToHost::read(&mut viewer_r) {
            let bye = m == ToHost::Bye;
            if m.write(&mut local_w).and_then(|_| local_w.flush()).is_err() || bye {
                break;
            }
        }
        let _ = local_w.flush();
        if let Ok(s) = local_w.into_inner() {
            let _ = s.shutdown(std::net::Shutdown::Write);
        }
    });
    // Session to viewer: frames compressed on the way.
    let code = loop {
        let m = match ToViewer::read(&mut local_r) {
            Ok(m) => m,
            Err(_) => break ExitCode::SUCCESS,
        };
        let m = match m {
            ToViewer::Frame(mut f) => {
                f.rects = f.rects.into_iter().map(|r| Rect::new(r.x, r.y, r.w, r.h, &r.data)).collect();
                ToViewer::Frame(f)
            }
            other => other,
        };
        let bye = matches!(m, ToViewer::Bye { .. });
        if m.write(&mut viewer_w).and_then(|_| viewer_w.flush()).is_err() {
            break ExitCode::from(1);
        }
        if bye {
            break ExitCode::SUCCESS;
        }
    };
    drop(up);
    code
}
