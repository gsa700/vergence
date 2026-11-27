//! The helper against a real TLS server: `openssl s_server` with a
//! certificate made here, by a throwaway authority of our own. Skipped
//! (and said so) where the openssl command is not installed.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

fn openssl() -> bool {
    Command::new("openssl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A certificate for `localhost` only (no IP address in it), self-signed.
fn certificate(dir: &Path) -> (PathBuf, PathBuf) {
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    let ok = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=localhost"])
        .args(["-addext", "subjectAltName=DNS:localhost"])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(ok.success());
    (cert, key)
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn server(cert: &Path, key: &Path) -> (Server, u16) {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new("openssl")
        .args(["s_server", "-quiet", "-www", "-accept"])
        .arg(port.to_string())
        .arg("-cert")
        .arg(cert)
        .arg("-key")
        .arg(key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    (Server(child), port)
}

fn tls(ca: Option<PathBuf>) -> vergence_http::Options {
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_vergence-tls"));
    vergence_http::Options { tls: Some(Arc::new(vergence_tls::Tls { helper, ca })), ..Default::default() }
}

#[test]
fn https_with_a_trusted_certificate_and_its_name() {
    if !openssl() {
        eprintln!("skipped: no openssl command");
        return;
    }
    let dir = std::env::temp_dir().join(format!("vergence-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (cert, key) = certificate(&dir);
    let (_server, port) = server(&cert, &key);
    // The name in the certificate, trusted: s_server's status page.
    let url = vergence_http::Url::parse(&format!("https://localhost:{port}/")).unwrap();
    let r = vergence_http::get(&url, &tls(Some(cert.clone()))).expect("a trusted certificate");
    assert_eq!(r.status(), 200);
    let body = String::from_utf8_lossy(&r.bytes().expect("the status page")).to_string();
    assert!(body.contains("s_server") || body.contains("Ciphers"), "{body}");
    // Not the name in it (an address the certificate does not hold).
    let url = vergence_http::Url::parse(&format!("https://127.0.0.1:{port}/")).unwrap();
    let e = vergence_http::get(&url, &tls(Some(cert.clone()))).err().expect("a name the certificate does not hold");
    eprintln!("refused, as it should be: {e}");
    // Not trusted: the system's certificates do not know this authority.
    let url = vergence_http::Url::parse(&format!("https://localhost:{port}/")).unwrap();
    let e = vergence_http::get(&url, &tls(None)).err().expect("a certificate nobody vouches for");
    eprintln!("refused, as it should be: {e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_helper_is_confined() {
    let ok = Command::new(env!("CARGO_BIN_EXE_vergence-tls")).arg("--check-confined").status().unwrap();
    // Linux kills it at the open (a signal, no exit code); FreeBSD refuses the open (exit 0).
    assert!(ok.code().is_none_or(|c| c == 0), "{ok:?}");
}
