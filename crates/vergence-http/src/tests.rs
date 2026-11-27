use super::*;
use std::net::TcpListener;

#[test]
fn addresses_are_read() {
    let u = Url::parse("http://10.0.1.5:4533/rest/ping?u=dave&f=json").unwrap();
    assert_eq!(
        (u.https, u.host.as_str(), u.port, u.path.as_str()),
        (false, "10.0.1.5", 4533, "/rest/ping?u=dave&f=json")
    );
    let u = Url::parse("https://Music.Example.com").unwrap();
    assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "music.example.com", 443, "/"));
    let u = Url::parse("http://[fd00::5]:4533/x#frag").unwrap();
    assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("fd00::5", 4533, "/x"));
    assert_eq!(u.to_text(), "http://[fd00::5]:4533/x");
    assert_eq!(Url::parse("http://h?x=1").unwrap().path, "/?x=1");
    for bad in [
        "ftp://h/",
        "http://",
        "http://user:pw@h/",
        "http://h:0/",
        "http://h:99999/",
        "http://h /",
        "http://[nope]/",
        "http://h/a b",
        "h/",
    ] {
        assert!(Url::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
}

#[test]
fn the_local_network() {
    let local = [
        "10.0.1.5",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.10",
        "169.254.3.4",
        "127.0.0.1",
        "::1",
        "fd12::1",
        "fe80::1",
        "::ffff:192.168.0.2",
    ];
    for a in local {
        assert!(is_local(a.parse().unwrap()), "{a}");
    }
    let public = ["8.8.8.8", "172.32.0.1", "100.64.0.1", "100.127.255.1", "2001:4860::8888", "::ffff:8.8.8.8"];
    for a in public {
        assert!(!is_local(a.parse().unwrap()), "{a}");
    }
}

#[test]
fn heads_are_read_or_refused() {
    let (h, used) =
        parse_head(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nX-A:  b \r\n\r\nhello").unwrap().unwrap();
    assert_eq!((h.status, used), (206, 61));
    assert_eq!(h.header("content-length"), Some("5"));
    assert_eq!(h.header("x-a"), Some("b"));
    assert!(parse_head(b"HTTP/1.1 200 OK\r\nA: b\r\n").unwrap().is_none(), "not all there yet");
    for bad in [
        &b"HTTP/2 200 OK\r\n\r\n"[..],
        b"HTTP/1.1 20 OK\r\n\r\n",
        b"HTTP/1.1 999 X\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nno colon\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nA\x01: b\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nA: b\x00c\r\n\r\n",
        b"HTTP/1.1 200 OK\r\n\xff: b\r\n\r\n",
    ] {
        assert!(parse_head(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
    let many = format!("HTTP/1.1 200 OK\r\n{}\r\n", "A: b\r\n".repeat(MAX_HEADERS + 1));
    assert!(parse_head(many.as_bytes()).is_err());
    assert!(parse_head(&vec![b'a'; MAX_HEAD + 1]).is_err(), "a head that never ends");
}

#[test]
fn chunks_are_decoded_across_any_split() {
    let wire = b"5;ext=1\r\nhello\r\n7\r\n, world\r\n0\r\nTrailer: x\r\n\r\n";
    for split in 0..wire.len() {
        let mut c = Chunked::default();
        let mut out = Vec::new();
        c.feed(&wire[..split], &mut out).unwrap();
        c.feed(&wire[split..], &mut out).unwrap();
        assert!(c.done, "split at {split}");
        assert_eq!(out, b"hello, world", "split at {split}");
    }
    // One byte at a time.
    let mut c = Chunked::default();
    let mut out = Vec::new();
    for b in wire {
        c.feed(&[*b], &mut out).unwrap();
    }
    assert_eq!((c.done, out.as_slice()), (true, &b"hello, world"[..]));
    for bad in [&b"zz\r\n"[..], b"5\r\nhelloXX\r\n", b"10000000000000000\r\n"] {
        let mut c = Chunked::default();
        assert!(c.feed(bad, &mut Vec::new()).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}

/// Random bytes never make the parsers panic.
#[test]
fn random_bytes_are_errors_not_panics() {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let pieces: [&[u8]; 8] = [b"HTTP/1.1 ", b"200", b" OK", b"\r\n", b": ", b"Content-Length", b"\r\n\r\n", b"0\r\n"];
    for _ in 0..20_000 {
        let n = (next() % 64) as usize;
        let mut buf = Vec::new();
        for _ in 0..n {
            if next() % 3 == 0 {
                buf.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
            } else {
                buf.push(next() as u8);
            }
        }
        let _ = parse_head(&buf);
        let _ = Chunked::default().feed(&buf, &mut Vec::new());
        let _ = Url::parse(&String::from_utf8_lossy(&buf));
    }
}

/// A one-request server on a local port: what it answers, by the request.
fn serve(answer: impl Fn(&str) -> Vec<u8> + Send + 'static) -> Url {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut req = Vec::new();
            let mut b = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&b[..n]),
                }
            }
            let reply = answer(&String::from_utf8_lossy(&req));
            if reply == b"STALL" {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
            let _ = s.write_all(&reply);
        }
    });
    Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap()
}

#[test]
fn a_fixed_length_answer() {
    let u = serve(|_| b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec());
    let r = get(&u.with_path("/rest/ping"), &Options::default()).unwrap();
    assert_eq!((r.status(), r.length()), (200, Some(5)));
    assert!(is_local(r.peer.ip()));
    assert_eq!(r.bytes().unwrap(), b"hello");
}

#[test]
fn a_chunked_answer_and_one_read_to_close() {
    let u =
        serve(|_| b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n".to_vec());
    assert_eq!(get(&u, &Options::default()).unwrap().bytes().unwrap(), b"abcde");
    let u = serve(|_| b"HTTP/1.0 200 OK\r\n\r\nuntil the end".to_vec());
    assert_eq!(get(&u, &Options::default()).unwrap().bytes().unwrap(), b"until the end");
}

#[test]
fn a_range_is_asked_for() {
    let u = serve(|req| {
        let range = req.lines().find(|l| l.starts_with("Range:")).unwrap_or("").to_string();
        let body = range.into_bytes();
        let mut r = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        r.extend_from_slice(&body);
        r
    });
    let opts = Options { range: Some((1000, None)), ..Options::default() };
    let r = get(&u, &opts).unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.bytes().unwrap(), b"Range: bytes=1000-");
}

#[test]
fn redirects_within_the_server_only() {
    let u = serve(|req| {
        if req.starts_with("GET /old ") {
            b"HTTP/1.1 302 Found\r\nLocation: /new\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else if req.starts_with("GET /away ") {
            b"HTTP/1.1 302 Found\r\nLocation: http://elsewhere.example/\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()
        }
    });
    let r = get(&u.with_path("/old"), &Options::default()).unwrap();
    assert_eq!(r.url.path, "/new");
    assert_eq!(r.bytes().unwrap(), b"ok");
    assert!(matches!(get(&u.with_path("/away"), &Options::default()), Err(Error::Redirect(_))));
}

#[test]
fn too_large_and_too_slow_are_refused() {
    let u = serve(|_| b"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\n\r\n".to_vec());
    let small = Options { max_body: 1000, ..Options::default() };
    assert!(matches!(get(&u, &small), Err(Error::TooLarge)));
    // No length said: the limit holds while reading.
    let u = serve(|_| {
        let mut r = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
        r.extend(std::iter::repeat_n(b'x', 5000));
        r
    });
    assert!(matches!(get(&u, &small).unwrap().bytes(), Err(Error::TooLarge)));
    // A server that never answers: the whole answer's deadline.
    let u = serve(|_| b"STALL".to_vec());
    let quick = Options { answer: Duration::from_millis(300), ..Options::default() };
    let t = Instant::now();
    assert!(matches!(get(&u, &quick), Err(Error::Timeout)));
    assert!(t.elapsed() < Duration::from_secs(2));
    // An answer cut short.
    let u = serve(|_| b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc".to_vec());
    assert!(matches!(get(&u, &Options::default()).unwrap().bytes(), Err(Error::BadAnswer(_))));
}

#[test]
fn https_without_tls_says_so() {
    assert!(matches!(get(&Url::parse("https://127.0.0.1:1/").unwrap(), &Options::default()), Err(Error::NoTls)));
}
