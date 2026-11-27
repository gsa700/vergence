use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;

/// Songs the mock server has sent (the song tests count their fetches).
static STREAMS: AtomicUsize = AtomicUsize::new(0);

/// The mock server's song "so-0": 300 kB, not all one byte.
fn song() -> Vec<u8> {
    (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect()
}

/// A small Subsonic server on a local port: user "dave", password "sesame";
/// `albums` and `songs` in pages of 500; a cover for "c1" only.
fn server(albums: usize, songs_per: usize) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut req = Vec::new();
            let mut b = [0u8; 4096];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&b[..n]),
                }
            }
            let req = String::from_utf8_lossy(&req).to_string();
            let line = req.lines().next().unwrap_or("").to_string();
            let path = line.split(' ').nth(1).unwrap_or("/").to_string();
            let q = |k: &str| {
                path.split(['?', '&']).find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap_or("").to_string()
            };
            // The signature: t = md5("sesame" + s).
            let ok_sig = q("u") == "dave" && q("t") == crate::md5::hex(format!("sesame{}", q("s")).as_bytes());
            let json = |body: String| {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .into_bytes()
            };
            let reply = if !ok_sig {
                json(r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#.into())
            } else if path.starts_with("/rest/ping") {
                json(r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#.into())
            } else if path.starts_with("/rest/getAlbumList2") {
                let offset: usize = q("offset").parse().unwrap_or(0);
                let items: Vec<String> = (offset..albums.min(offset + 500))
                    .map(|i| {
                        let cover = if i == 1 { r#","coverArt":"c1""# } else { "" };
                        format!(
                            r#"{{"id":"al-{i}","name":"Album {i:04}","artist":"Artist {}","year":{}{cover}}}"#,
                            i / 3,
                            1970 + i % 40
                        )
                    })
                    .collect();
                json(format!(
                    r#"{{"subsonic-response":{{"status":"ok","albumList2":{{"album":[{}]}}}}}}"#,
                    items.join(",")
                ))
            } else if path.starts_with("/rest/search3") {
                let offset: usize = q("songOffset").parse().unwrap_or(0);
                let total = albums * songs_per;
                let items: Vec<String> = (offset..total.min(offset + 500))
                    .map(|i| {
                        let (a, n) = (i / songs_per, i % songs_per + 1);
                        format!(
                            r#"{{"id":"so-{i}","albumId":"al-{a}","title":"Song {n}","artist":"Artist {}","track":{n},"discNumber":1,"duration":200,"suffix":"flac","bitRate":900,"size":20000000,"samplingRate":44100,"bitDepth":16}}"#,
                            a / 3
                        )
                    })
                    .collect();
                json(format!(
                    r#"{{"subsonic-response":{{"status":"ok","searchResult3":{{"song":[{}]}}}}}}"#,
                    items.join(",")
                ))
            } else if path.starts_with("/rest/getCoverArt") && q("id") == "c1" {
                let png = b"\x89PNG\r\n\x1a\nfake";
                let mut r =
                    format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", png.len())
                        .into_bytes();
                r.extend_from_slice(png);
                r
            } else if path.starts_with("/rest/stream") && q("id") == "so-0" && q("format") == "raw" {
                STREAMS.fetch_add(1, Ordering::SeqCst);
                // Slow enough that a second ask comes while it is on its way.
                std::thread::sleep(std::time::Duration::from_millis(300));
                let body = song();
                let mut r =
                    format!("HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\n\r\n", body.len())
                        .into_bytes();
                r.extend_from_slice(&body);
                r
            } else if path.starts_with("/rest/getLyricsBySongId") && q("id") == "so-0" {
                json(r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[
                    {"synced":false,"lang":"eng","line":[{"value":"plain words"}]},
                    {"synced":true,"lang":"eng","line":[{"start":1000,"value":"Sung"},{"start":2500,"value":"words"}]}]}}}"#
                    .into())
            } else if path.starts_with("/rest/getLyricsBySongId") {
                json(r#"{"subsonic-response":{"status":"ok","lyricsList":{}}}"#.into())
            } else if path.starts_with("/rest/stream") {
                json(
                    r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"Song not found"}}}"#
                        .into(),
                )
            } else if path.starts_with("/rest/getCoverArt") {
                json(r#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"not found"}}}"#.into())
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
            };
            let _ = s.write_all(&reply);
        }
    });
    format!("127.0.0.1:{port}")
}

#[test]
fn addresses_are_made_whole() {
    assert_eq!(normalize("10.0.1.5:4533").unwrap(), "http://10.0.1.5:4533");
    assert_eq!(normalize("https://music.example.com/navidrome/rest/").unwrap(), "https://music.example.com/navidrome");
    assert!(normalize("http://h/?x=1").is_err());
}

#[test]
fn signing_in_keeps_no_password() {
    let a = server(2, 1);
    let s = Server::sign_in(&a, "dave", "sesame", None).unwrap();
    assert_eq!(s.login.salt.len(), 12);
    assert_eq!(s.login.token, crate::md5::hex(format!("sesame{}", s.login.salt).as_bytes()));
    assert!(!format!("{:?}", s.login).contains("sesame"));
    let e = Server::sign_in(&a, "dave", "wrong", None).err().unwrap();
    assert_eq!(e, Error::Refused { code: 40, message: "Wrong username or password".into() });
    assert!(e.to_string().contains("did not accept"));
    let off = Server::sign_in("127.0.0.1:1", "dave", "sesame", None).err().unwrap();
    assert!(matches!(off, Error::Unreachable(_)), "{off:?}");
}

#[test]
fn the_library_in_pages_its_covers_and_its_list_kept() {
    let a = server(700, 3);
    let s = Server::sign_in(&a, "dave", "sesame", None).unwrap();
    let mut seen = Vec::new();
    let l = s.listing(&AtomicBool::new(false), |n| seen.push(n)).unwrap();
    assert_eq!((l.albums.len(), l.songs.len()), (700, 2100), "two pages of albums, five of songs");
    assert_eq!(seen.last(), Some(&2100));
    let dir = std::env::temp_dir().join(format!("aw-covers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.fetch_covers(&l, &dir, &AtomicBool::new(false), |_, _| {});
    assert!(cover_file(&dir, "c1").is_some_and(|p| p.extension().is_some_and(|e| e == "png")));
    let key = s.login.key();
    assert!(is_server(&key));
    let albums = albums(&key, &l, &dir);
    assert_eq!(albums.len(), 700);
    let a1 = albums.iter().find(|a| a.title == "Album 0001").unwrap();
    assert!(a1.front.is_some(), "its cover from the kept file");
    assert_eq!(a1.tracks.len(), 3);
    assert_eq!(song_id(&a1.tracks[0].path).as_deref(), Some("so-3"));
    assert!(!a1.tracks[0].path.to_string_lossy().contains(&s.login.token), "no sign-in in a path");
    // Kept and read back.
    assert_eq!(Listing::from_text(&l.to_text()), Some(l.clone()));
    assert_eq!(Listing::from_text("# another\n"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_streams_address_is_made_when_needed() {
    let a = server(1, 1);
    let s = Server::sign_in(&a, "dave", "sesame", None).unwrap();
    let u = s.stream_url("so-0");
    assert!(u.path.starts_with("/rest/stream?id=so-0&format=raw&u=dave&t="), "{}", u.path);
    let r = vergence_http::get(&s.address("ping", ""), &Options::default()).unwrap();
    assert_eq!(r.status(), 200);
}

#[test]
fn a_song_is_fetched_once_for_two_asks_and_kept() {
    let a = server(1, 1);
    let s = Server::sign_in(&a, "dave", "sesame", None).unwrap();
    let key = s.login.key().to_string_lossy().to_string();
    register(vec![s.login.clone()]);
    let path = PathBuf::from(format!("{key}/song/so-0.flac"));
    // The player's opening ahead and its start, at once: one fetch.
    let p2 = path.clone();
    let other = std::thread::spawn(move || song_bytes(&p2, None));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mine = song_bytes(&path, None).unwrap().unwrap();
    let theirs = other.join().unwrap().unwrap().unwrap();
    assert_eq!(*mine, song());
    assert!(Arc::ptr_eq(&mine, &theirs));
    // Asked again (a seek opens the track again): kept, not fetched.
    let _ = song_bytes(&path, None).unwrap().unwrap();
    assert_eq!(STREAMS.load(Ordering::SeqCst), 1);
    // The server's own words when it will not play one.
    let gone = song_bytes(&PathBuf::from(format!("{key}/song/so-9.flac")), None).unwrap().unwrap_err();
    assert!(gone.contains("Song not found"), "{gone}");
    // Its lyrics, the server's timed set; a song without any has none.
    let words = song_lyrics(&path, None).expect("lyrics");
    assert!(words.timed);
    assert_eq!(words.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["Sung", "words"]);
    assert_eq!(song_lyrics(&PathBuf::from(format!("{key}/song/so-9.flac")), None), None);
    // A file is not a server's song; a server not signed in to says so.
    assert!(song_bytes(Path::new("/music/a.flac"), None).is_none());
    let stranger = song_bytes(Path::new("navidrome://eve@10.9.9.9:4533/song/x.flac"), None).unwrap().unwrap_err();
    assert!(stranger.contains("not signed in"), "{stranger}");
}
