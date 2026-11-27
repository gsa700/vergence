//! `vergence-tls` (libexec): the TLS half of an HTTPS connection, through
//! the system's OpenSSL (see the crate's comment). Standard input is the
//! connected socket to the server; standard output is the program's end of
//! the clear stream. Arguments: `--host=NAME` (checked against the
//! certificate and sent as SNI; an IP address is checked as one), `--ca=FILE`
//! (trust this file instead of the system's certificates), and
//! `--check-confined` (for the tests: confine, then try to open a file).
//!
//! Before reading a byte from the server it loads OpenSSL and the system's
//! certificates and confines itself (vergence_decode::confine_allowing, with
//! poll and getpid allowed on Linux: OpenSSL's random generator asks for its
//! process id, to notice a fork): no file system, no network of its own, no
//! new descriptor. Then the handshake, one line to the program ("+" or "-why"),
//! and bytes both ways until either side closes.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_ulong, c_void};
use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;

#[link(name = "ssl")]
#[link(name = "crypto")]
unsafe extern "C" {
    fn OPENSSL_init_ssl(opts: u64, settings: *const c_void) -> c_int;
    fn TLS_client_method() -> *const c_void;
    fn SSL_CTX_new(method: *const c_void) -> *mut c_void;
    fn SSL_CTX_load_verify_locations(ctx: *mut c_void, file: *const c_char, path: *const c_char) -> c_int;
    fn SSL_CTX_set_verify(ctx: *mut c_void, mode: c_int, cb: *const c_void);
    fn SSL_CTX_ctrl(ctx: *mut c_void, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
    fn SSL_new(ctx: *mut c_void) -> *mut c_void;
    fn SSL_set_fd(ssl: *mut c_void, fd: c_int) -> c_int;
    fn SSL_ctrl(ssl: *mut c_void, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
    fn SSL_set1_host(ssl: *mut c_void, name: *const c_char) -> c_int;
    fn SSL_get0_param(ssl: *mut c_void) -> *mut c_void;
    fn X509_VERIFY_PARAM_set1_ip_asc(param: *mut c_void, ip: *const c_char) -> c_int;
    fn SSL_connect(ssl: *mut c_void) -> c_int;
    fn SSL_get_error(ssl: *mut c_void, ret: c_int) -> c_int;
    fn SSL_get_verify_result(ssl: *mut c_void) -> c_long;
    fn X509_verify_cert_error_string(n: c_long) -> *const c_char;
    fn SSL_read(ssl: *mut c_void, buf: *mut c_void, num: c_int) -> c_int;
    fn SSL_write(ssl: *mut c_void, buf: *const c_void, num: c_int) -> c_int;
    fn SSL_pending(ssl: *mut c_void) -> c_int;
    fn SSL_shutdown(ssl: *mut c_void) -> c_int;
    fn ERR_get_error() -> c_ulong;
    fn ERR_error_string_n(e: c_ulong, buf: *mut c_char, len: usize);
}

// nfds_t: unsigned long on Linux, unsigned int on FreeBSD.
#[cfg(target_os = "linux")]
type NFds = std::ffi::c_ulong;
#[cfg(not(target_os = "linux"))]
type NFds = std::ffi::c_uint;

unsafe extern "C" {
    fn poll(fds: *mut PollFd, n: NFds, timeout: c_int) -> c_int;
    fn tzset();
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x1;
const POLLERR: i16 = 0x8;
const POLLHUP: i16 = 0x10;
const SSL_VERIFY_PEER: c_int = 1;
const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
const SSL_CTRL_SET_MIN_PROTO_VERSION: c_int = 123;
const TLS1_2_VERSION: c_long = 0x0303;
const SSL_ERROR_WANT_READ: c_int = 2;
const SSL_ERROR_WANT_WRITE: c_int = 3;

/// The system's certificates: Void's, FreeBSD's, and the usual others.
const BUNDLES: [&str; 4] = [
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/ssl/cert.pem",
    "/etc/ssl/certs.pem",
    "/usr/local/share/certs/ca-root-nss.crt",
];

/// The clear stream to the program (standard output, a socket: both ways).
fn program() -> ManuallyDrop<std::fs::File> {
    // SAFETY: descriptor 1 is the socket pair's end the program gave us,
    // open for the process's life; ManuallyDrop keeps it from being closed twice.
    ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(1) })
}

fn say(line: &str) {
    let mut p = program();
    let _ = p.write_all(format!("{}\n", line.replace('\n', " ")).as_bytes());
}

fn ssl_error() -> String {
    let mut buf = [0 as c_char; 256];
    // SAFETY: ERR_get_error takes nothing; ERR_error_string_n writes at most len bytes, NUL-terminated.
    let e = unsafe { ERR_get_error() };
    if e == 0 {
        return "the connection failed".into();
    }
    unsafe { ERR_error_string_n(e, buf.as_mut_ptr(), buf.len()) };
    // SAFETY: NUL-terminated by ERR_error_string_n.
    unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

fn main() {
    // Certificates' dates are compared in UTC; a time zone given as a rule
    // needs no file, so checking them never opens /etc/localtime once
    // confined (it did, and the filter ended the helper).
    // SAFETY: set before any thread exists; tzset(3) takes nothing.
    unsafe {
        std::env::set_var("TZ", "UTC0");
        tzset();
    }
    let mut host = None;
    let mut ca = None;
    let mut check = false;
    for a in std::env::args().skip(1) {
        if let Some(h) = a.strip_prefix("--host=") {
            host = Some(h.to_string());
        } else if let Some(c) = a.strip_prefix("--ca=") {
            ca = Some(c.to_string());
        } else if a == "--check-confined" {
            check = true;
        }
    }
    if check {
        vergence_decode::confine_allowing(5, &[7, 39, 271]);
        // Confined, this is refused (FreeBSD) or ends the process (Linux).
        match std::fs::File::open("/etc/passwd") {
            Ok(_) => std::process::exit(1),
            Err(_) => std::process::exit(0),
        }
    }
    let Some(host) = host.filter(|h| !h.is_empty() && !h.contains('\0')) else {
        say("-no host given");
        std::process::exit(2);
    };
    // OpenSSL and the certificates, before confinement.
    // SAFETY: plain calls into OpenSSL with valid pointers; each result checked.
    let ctx = unsafe {
        OPENSSL_init_ssl(0, std::ptr::null());
        SSL_CTX_new(TLS_client_method())
    };
    if ctx.is_null() {
        say(&format!("-{}", ssl_error()));
        std::process::exit(3);
    }
    let bundle = ca.or_else(|| BUNDLES.iter().find(|b| std::path::Path::new(b).is_file()).map(|b| b.to_string()));
    let Some(bundle) = bundle.and_then(|b| CString::new(b).ok()) else {
        say("-no certificates to check the server against");
        std::process::exit(3);
    };
    unsafe {
        if SSL_CTX_load_verify_locations(ctx, bundle.as_ptr(), std::ptr::null()) != 1 {
            say(&format!("-the certificates could not be read: {}", ssl_error()));
            std::process::exit(3);
        }
        SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, std::ptr::null());
        SSL_CTX_ctrl(ctx, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, std::ptr::null_mut());
    }
    vergence_decode::confine_allowing(24 * 3600, &[7, 39, 271]);
    // The handshake, confined.
    let name = CString::new(host.clone()).unwrap_or_default();
    let is_ip = host.parse::<std::net::IpAddr>().is_ok();
    let ssl = unsafe { SSL_new(ctx) };
    if ssl.is_null() || unsafe { SSL_set_fd(ssl, 0) } != 1 {
        say(&format!("-{}", ssl_error()));
        std::process::exit(3);
    }
    unsafe {
        if is_ip {
            // An address is checked as one; no SNI for an address.
            X509_VERIFY_PARAM_set1_ip_asc(SSL_get0_param(ssl), name.as_ptr());
        } else {
            SSL_set1_host(ssl, name.as_ptr());
            SSL_ctrl(ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, 0, name.as_ptr() as *mut c_void);
        }
        if SSL_connect(ssl) != 1 {
            let v = SSL_get_verify_result(ssl);
            let why = if v != 0 {
                CStr::from_ptr(X509_verify_cert_error_string(v)).to_string_lossy().into_owned()
            } else {
                ssl_error()
            };
            say(&format!("-{why}"));
            std::process::exit(4);
        }
        if SSL_get_verify_result(ssl) != 0 {
            say("-the server's certificate could not be checked");
            std::process::exit(4);
        }
    }
    say("+");
    relay(ssl);
}

/// Bytes both ways until either side closes.
fn relay(ssl: *mut c_void) {
    let mut p = program();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        // SAFETY: ssl is the live connection.
        let pending = unsafe { SSL_pending(ssl) } > 0;
        let mut fds = [PollFd { fd: 0, events: POLLIN, revents: 0 }, PollFd { fd: 1, events: POLLIN, revents: 0 }];
        if !pending {
            // SAFETY: poll(2) on two valid pollfd structs, no time limit.
            if unsafe { poll(fds.as_mut_ptr(), 2, -1) } < 0 {
                return;
            }
        }
        let ready = |f: &PollFd| f.revents & (POLLIN | POLLERR | POLLHUP) != 0;
        if pending || ready(&fds[0]) {
            // SAFETY: buf has buf.len() bytes of room.
            let n = unsafe { SSL_read(ssl, buf.as_mut_ptr() as *mut c_void, buf.len() as c_int) };
            if n > 0 {
                if p.write_all(&buf[..n as usize]).is_err() {
                    return;
                }
            } else {
                let e = unsafe { SSL_get_error(ssl, n) };
                if e != SSL_ERROR_WANT_READ && e != SSL_ERROR_WANT_WRITE {
                    return;
                }
            }
        }
        if ready(&fds[1]) {
            match p.read(&mut buf) {
                Ok(0) | Err(_) => {
                    unsafe { SSL_shutdown(ssl) };
                    return;
                }
                Ok(n) => {
                    let mut at = 0;
                    while at < n {
                        // SAFETY: the bytes from at to n are initialized.
                        let w = unsafe { SSL_write(ssl, buf[at..n].as_ptr() as *const c_void, (n - at) as c_int) };
                        if w <= 0 {
                            return;
                        }
                        at += w as usize;
                    }
                }
            }
        }
    }
}
