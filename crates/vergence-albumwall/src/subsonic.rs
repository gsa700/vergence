//! A Navidrome server as a library (milestone 2, step 2): the Subsonic API
//! it serves, through our own HTTP client (vergence-http) and, for HTTPS,
//! the confined OpenSSL helper. Ported from the Avalonia AlbumWall's
//! Navidrome.cs, whose rules hold:
//!
//! - **Read-only, all of it.** Nothing is written to the server.
//! - **The password is never kept** (question 3): Subsonic signs each request
//!   with a salt and md5(password + salt); those two are made once at sign-in
//!   and kept, and the password is forgotten.
//! - **A track's path is not its address**: `navidrome://user@host:port/song/
//!   <id>.<ext>` says which song without saying how to sign in, because a path
//!   goes where a sign-in must not (the session file, logs, other programs
//!   asking what is playing). [`Server::stream_url`] makes the address at the
//!   one moment the player needs it.
//! - **What is kept on this machine** (question 4): the album and song list
//!   ([`Listing`], our own small file) so the wall opens at once and is then
//!   checked against the server; the covers as files, named by their ids (a
//!   cover id changes when its picture does, so a file is never stale).
//!
//! The library comes in pages of 500: every album (getAlbumList2), then every
//! song through an empty search (search3), thirty-odd requests for a library
//! that would be a thousand asked album by album; then the covers not yet
//! here, six at a time.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use vergence_http::json::Value;
use vergence_http::{Options, Url};

use crate::library::{Album, Art, Track};

/// Albums and songs asked for at a time.
const PAGE: usize = 500;
/// What one answer may be (the Avalonia app's security review, 2026-10-10):
/// a page of 500 albums is well under 1 MB; a 3000-pixel JPEG a few MB.
const MAX_JSON: u64 = 64 * 1024 * 1024;
const MAX_COVER: u64 = 20 * 1024 * 1024;
/// The API version asked for, and who asks.
const API: &str = "1.16.1";
const CLIENT: &str = "vergence-albumwall";

/// A server signed in to, as the library list keeps it: no password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    /// The server's address as given, normalized ("http://10.0.1.5:4533",
    /// a path kept for a server behind a proxy: "https://example.com/music").
    pub address: String,
    pub user: String,
    pub salt: String,
    pub token: String,
    /// One of the server's own libraries ("music folders"), or all of them.
    pub folder: Option<String>,
}

impl Login {
    /// The library's key on the wall (its "folder"): which server and which
    /// of its libraries, never the sign-in.
    pub fn key(&self) -> PathBuf {
        let a = self.address.split_once("://").map_or(self.address.as_str(), |(_, r)| r);
        let mut k = format!("navidrome://{}@{a}", self.user);
        if let Some(f) = &self.folder {
            k.push_str(&format!("/folder/{f}"));
        }
        PathBuf::from(k)
    }
}

/// Whether a library's key is a server's.
pub fn is_server(key: &Path) -> bool {
    key.to_string_lossy().starts_with("navidrome://")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not answering (off, asleep, wrong address): looked for again later.
    Unreachable(String),
    /// Answering with a refusal (code 40: a wrong user or password).
    Refused { code: i64, message: String },
    /// An answer that could not be read.
    Bad(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable(s) => write!(f, "{s}"),
            Error::Refused { code: 40, .. } => write!(f, "the server did not accept that user name and password"),
            Error::Refused { message, .. } => write!(f, "the server said no: {message}"),
            Error::Bad(s) => write!(f, "the server's answer could not be read: {s}"),
        }
    }
}

/// An address as typed, made whole: "10.0.1.5:4533" is http://10.0.1.5:4533;
/// a trailing slash and "/rest" are taken off.
pub fn normalize(address: &str) -> Result<String, Error> {
    let a = address.trim();
    let a = if a.contains("://") { a.to_string() } else { format!("http://{a}") };
    let a = a.trim_end_matches('/');
    let a = a.strip_suffix("/rest").unwrap_or(a).trim_end_matches('/');
    let url = Url::parse(a).map_err(|e| Error::Unreachable(e.to_string()))?;
    if url.path.contains('?') {
        return Err(Error::Unreachable("an address with a query in it".into()));
    }
    Ok(url.to_text().trim_end_matches('/').to_string())
}

/// A salt: 12 hex characters from the system's random source.
fn salt() -> String {
    use std::io::Read;
    let mut b = [0u8; 6];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // No random source: the time, which still varies the token per sign-in.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        b.copy_from_slice(&t.to_le_bytes()[..6]);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// One album as the server lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteAlbum {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub year: i32,
    pub cover: Option<String>,
}

/// One song as the server lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteSong {
    pub id: String,
    pub album: String,
    pub title: String,
    pub artist: String,
    pub track: u32,
    pub disc: u32,
    pub seconds: u64,
    pub suffix: String,
    pub bitrate: u32,
    pub size: u64,
    pub rate: u32,
    pub bits: u16,
}

/// What the server holds: its albums and songs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub albums: Vec<RemoteAlbum>,
    pub songs: Vec<RemoteSong>,
}

/// A server to ask.
pub struct Server {
    pub login: Login,
    base: Url,
    opts: Options,
}

impl Server {
    pub fn new(login: Login, tls: Option<Arc<dyn vergence_http::Connector>>) -> Result<Server, Error> {
        let base = Url::parse(&login.address).map_err(|e| Error::Unreachable(e.to_string()))?;
        let opts = Options { agent: CLIENT.into(), tls, max_body: MAX_JSON, ..Options::default() };
        Ok(Server { login, base, opts })
    }

    /// Signs in with a password, which is used for the token and forgotten.
    pub fn sign_in(
        address: &str,
        user: &str,
        password: &str,
        tls: Option<Arc<dyn vergence_http::Connector>>,
    ) -> Result<Server, Error> {
        let address = normalize(address)?;
        let salt = salt();
        let token = crate::md5::hex(format!("{password}{salt}").as_bytes());
        let login = Login { address, user: user.to_string(), salt, token, folder: None };
        let s = Server::new(login, tls)?;
        s.ping()?;
        Ok(s)
    }

    /// Whether this server's address is on the local network, as connected
    /// to (Dave, 2026-11-26: no warning for HTTP there), and whether it is HTTPS.
    pub fn https(&self) -> bool {
        self.base.https
    }

    /// The address of a call: the server's path, /rest/<call>, the query and
    /// the signature.
    fn address(&self, call: &str, query: &str) -> Url {
        let sep = if query.is_empty() { "" } else { "&" };
        let path = format!(
            "{}/rest/{call}?{query}{sep}u={}&t={}&s={}&v={API}&c={CLIENT}&f=json",
            self.base.path.trim_end_matches('/'),
            vergence_http::encode(&self.login.user),
            self.login.token,
            self.login.salt
        );
        self.base.with_path(&path)
    }

    /// A call's answer, its "subsonic-response" checked: ok, or the server's refusal.
    fn call(&self, call: &str, query: &str) -> Result<Value, Error> {
        let r = vergence_http::get(&self.address(call, query), &self.opts).map_err(unreachable)?;
        if r.status() != 200 {
            return Err(Error::Unreachable(format!("the server answered {} to {call}", r.status())));
        }
        let body = r.bytes().map_err(unreachable)?;
        let v = vergence_http::json::parse(&body).map_err(|e| Error::Bad(e.to_string()))?;
        let resp = v.get("subsonic-response").ok_or_else(|| Error::Bad("not a Subsonic answer".into()))?;
        if resp.get("status").and_then(Value::text).as_deref() == Some("ok") {
            return Ok(resp.clone());
        }
        let e = resp.get("error");
        Err(Error::Refused {
            code: e.and_then(|e| e.get("code")).and_then(Value::number).unwrap_or(0.0) as i64,
            message: e.and_then(|e| e.get("message")).and_then(Value::text).unwrap_or_default(),
        })
    }

    pub fn ping(&self) -> Result<(), Error> {
        self.call("ping", "").map(|_| ())
    }

    /// The address the server was reached at (a ping), for the warning rule:
    /// plain HTTP to an address outside the local network is warned about.
    pub fn reached(&self) -> Result<std::net::SocketAddr, Error> {
        let r = vergence_http::get(&self.address("ping", ""), &self.opts).map_err(unreachable)?;
        Ok(r.peer)
    }

    /// Whether to warn: HTTP (not HTTPS) to an address not on the local network.
    pub fn warn_plain(&self) -> bool {
        !self.base.https && self.reached().is_ok_and(|a| !vergence_http::is_local(a.ip()))
    }

    /// The server's own libraries: (id, name).
    pub fn folders(&self) -> Result<Vec<(String, String)>, Error> {
        let v = self.call("getMusicFolders", "")?;
        let list = v.get("musicFolders").and_then(|f| f.get("musicFolder")).map(Value::items).unwrap_or_default();
        Ok(list
            .into_iter()
            .filter_map(|f| Some((f.get("id")?.text()?, f.get("name").and_then(Value::text).unwrap_or_default())))
            .collect())
    }

    fn only(&self) -> String {
        self.login.folder.as_ref().map_or(String::new(), |f| format!("&musicFolderId={}", vergence_http::encode(f)))
    }

    /// Every album and song, a page at a time; `progress(songs so far)`.
    pub fn listing(&self, cancel: &AtomicBool, mut progress: impl FnMut(usize)) -> Result<Listing, Error> {
        let mut l = Listing::default();
        let stop = || cancel.load(Ordering::Relaxed);
        for offset in (0..).step_by(PAGE) {
            if stop() {
                return Err(Error::Unreachable("stopped".into()));
            }
            let v = self.call(
                "getAlbumList2",
                &format!("type=alphabeticalByName&size={PAGE}&offset={offset}{}", self.only()),
            )?;
            let page = v.get("albumList2").and_then(|a| a.get("album")).map(Value::items).unwrap_or_default();
            let n = page.len();
            l.albums.extend(page.into_iter().filter_map(album_of));
            if n < PAGE {
                break;
            }
        }
        for offset in (0..).step_by(PAGE) {
            if stop() {
                return Err(Error::Unreachable("stopped".into()));
            }
            let v = self.call(
                "search3",
                &format!("query=&artistCount=0&albumCount=0&songCount={PAGE}&songOffset={offset}{}", self.only()),
            )?;
            let page = v.get("searchResult3").and_then(|a| a.get("song")).map(Value::items).unwrap_or_default();
            let n = page.len();
            l.songs.extend(page.into_iter().filter_map(song_of));
            progress(l.songs.len());
            if n < PAGE {
                break;
            }
        }
        Ok(l)
    }

    /// A cover's picture and its file extension; None if the server has
    /// none for it (it answers with an error, not a picture).
    pub fn cover(&self, id: &str) -> Result<Option<(Vec<u8>, &'static str)>, Error> {
        let opts = Options { max_body: MAX_COVER, ..self.opts.clone() };
        let r = vergence_http::get(&self.address("getCoverArt", &format!("id={}", vergence_http::encode(id))), &opts)
            .map_err(unreachable)?;
        let kind = r.head.header("content-type").unwrap_or("").to_ascii_lowercase();
        let ext = match kind.split(';').next().unwrap_or("").trim() {
            "image/png" => "png",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "image/jpeg" | "image/jpg" => "jpg",
            _ => return Ok(None),
        };
        if r.status() != 200 {
            return Ok(None);
        }
        Ok(Some((r.bytes().map_err(unreachable)?, ext)))
    }

    /// The covers of `listing` not yet in `dir`, fetched six at a time;
    /// `progress(done, of)`. One cover failing is not the library.
    pub fn fetch_covers(
        &self,
        listing: &Listing,
        dir: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(usize, usize) + Sync,
    ) {
        let _ = std::fs::create_dir_all(dir);
        let mut wanted: Vec<&str> = listing.albums.iter().filter_map(|a| a.cover.as_deref()).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let missing: Vec<&str> = wanted.into_iter().filter(|id| cover_file(dir, id).is_none()).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| {
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(id) = missing.get(i) else { return };
                        if let Ok(Some((bytes, ext))) = self.cover(id) {
                            let file = dir.join(format!("{}.{ext}", safe(id)));
                            let tmp = file.with_extension("part");
                            if std::fs::write(&tmp, &bytes).is_ok() {
                                let _ = std::fs::rename(&tmp, &file);
                            }
                        }
                        progress(done.fetch_add(1, Ordering::Relaxed) + 1, missing.len());
                    }
                });
            }
        });
    }

    /// The address a song is played from, as it is (`format=raw`: the file
    /// itself, bit-perfect and gapless), made at the moment it is needed.
    pub fn stream_url(&self, song_id: &str) -> Url {
        self.address("stream", &format!("id={}&format=raw", vergence_http::encode(song_id)))
    }
}

fn unreachable(e: vergence_http::Error) -> Error {
    Error::Unreachable(e.to_string())
}

fn album_of(a: &Value) -> Option<RemoteAlbum> {
    Some(RemoteAlbum {
        id: a.get("id")?.text()?,
        name: a.get("name").and_then(Value::text).unwrap_or_default(),
        artist: a.get("artist").and_then(Value::text).unwrap_or_default(),
        year: a.get("year").and_then(Value::number).unwrap_or(0.0) as i32,
        cover: a.get("coverArt").and_then(Value::text).filter(|c| !c.is_empty()),
    })
}

fn song_of(s: &Value) -> Option<RemoteSong> {
    let n = |k: &str| s.get(k).and_then(Value::number).unwrap_or(0.0);
    Some(RemoteSong {
        id: s.get("id")?.text()?,
        album: s.get("albumId").and_then(Value::text).unwrap_or_default(),
        title: s.get("title").and_then(Value::text).unwrap_or_default(),
        artist: s.get("artist").and_then(Value::text).unwrap_or_default(),
        track: n("track") as u32,
        disc: n("discNumber") as u32,
        seconds: n("duration") as u64,
        suffix: s.get("suffix").and_then(Value::text).unwrap_or_default(),
        bitrate: n("bitRate") as u32,
        size: n("size") as u64,
        rate: n("samplingRate") as u32,
        bits: n("bitDepth") as u16,
    })
}

/// A cover id as a file name: only letters, digits, '-' and '_'.
fn safe(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// A cover's file in `dir`, if it is there.
pub fn cover_file(dir: &Path, id: &str) -> Option<PathBuf> {
    ["jpg", "png", "webp", "gif"].iter().map(|e| dir.join(format!("{}.{e}", safe(id)))).find(|p| p.is_file())
}

/// Where a server library's list and covers are kept: beside the folders'
/// indexes, named by its key.
pub fn cache_paths(key: &Path) -> Option<(PathBuf, PathBuf)> {
    let index = crate::index::path_for(key)?;
    Some((index.with_extension("server"), index.with_extension("covers")))
}

/// The library's albums, as the wall knows them: each song a track whose
/// path names it under the library's key; covers from `cover_dir`.
pub fn albums(key: &Path, l: &Listing, cover_dir: &Path) -> Vec<Album> {
    let key = key.to_string_lossy();
    let mut out: Vec<Album> = l
        .albums
        .iter()
        .map(|a| Album {
            album_artist: a.artist.clone(),
            title: a.name.clone(),
            year: a.year,
            tracks: Vec::new(),
            front: a.cover.as_deref().and_then(|c| cover_file(cover_dir, c)).map(|path| {
                let size = std::fs::metadata(&path).map_or(0, |m| m.len());
                Art { path, embedded: false, width: 0, height: 0, size, modified: 0 }
            }),
            back: None,
            folders: Default::default(),
            search: format!("{}\n{}", a.artist, a.name).to_lowercase(),
        })
        .collect();
    let place: std::collections::HashMap<&str, usize> =
        l.albums.iter().enumerate().map(|(i, a)| (a.id.as_str(), i)).collect();
    for s in &l.songs {
        let Some(&i) = place.get(s.album.as_str()) else { continue };
        let ext = if s.suffix.is_empty() { "audio".to_string() } else { safe(&s.suffix) };
        out[i].tracks.push(Track {
            disc: s.disc.max(1),
            number: s.track,
            title: s.title.clone(),
            artist: s.artist.clone(),
            millis: s.seconds * 1000,
            path: PathBuf::from(format!("{key}/song/{}.{ext}", safe(&s.id))),
            rate: s.rate,
            bits: s.bits,
            bitrate: s.bitrate,
            size: s.size,
        });
    }
    out.retain(|a| !a.tracks.is_empty());
    for a in &mut out {
        a.tracks.sort_by_key(|t| (t.disc, t.number));
    }
    out.sort_by_cached_key(|a| a.sort_key().to_uppercase());
    out
}

/// A track's song id, if its path is a server's song.
pub fn song_id(path: &Path) -> Option<String> {
    let p = path.to_string_lossy();
    if !p.starts_with("navidrome://") {
        return None;
    }
    let (_, file) = p.rsplit_once("/song/")?;
    Some(file.rsplit_once('.').map_or(file, |(id, _)| id).to_string())
}

// The kept listing: our own text, one record a line, fields by tabs (a tab
// or a newline in a name is a space).

const HEADER: &str = "# AlbumWall for Vergence: a music server's albums and songs (version 1)";

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

impl Listing {
    pub fn to_text(&self) -> String {
        let mut s = format!("{HEADER}\n");
        for a in &self.albums {
            s.push_str(&format!(
                "a\t{}\t{}\t{}\t{}\t{}\n",
                clean(&a.id),
                clean(&a.name),
                clean(&a.artist),
                a.year,
                a.cover.as_deref().map(clean).unwrap_or_default()
            ));
        }
        for t in &self.songs {
            s.push_str(&format!(
                "s\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                clean(&t.id),
                clean(&t.album),
                clean(&t.title),
                clean(&t.artist),
                t.track,
                t.disc,
                t.seconds,
                clean(&t.suffix),
                t.bitrate,
                t.size,
                t.rate,
                t.bits
            ));
        }
        s
    }

    /// Read back; None for another version or a file that is not one.
    pub fn from_text(text: &str) -> Option<Listing> {
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let mut l = Listing::default();
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            match f.first().copied() {
                Some("a") if f.len() == 6 => l.albums.push(RemoteAlbum {
                    id: f[1].into(),
                    name: f[2].into(),
                    artist: f[3].into(),
                    year: f[4].parse().unwrap_or(0),
                    cover: Some(f[5].to_string()).filter(|c| !c.is_empty()),
                }),
                Some("s") if f.len() == 13 => l.songs.push(RemoteSong {
                    id: f[1].into(),
                    album: f[2].into(),
                    title: f[3].into(),
                    artist: f[4].into(),
                    track: n(5) as u32,
                    disc: n(6) as u32,
                    seconds: n(7),
                    suffix: f[8].into(),
                    bitrate: n(9) as u32,
                    size: n(10),
                    rate: n(11) as u32,
                    bits: n(12) as u16,
                }),
                _ => {}
            }
        }
        Some(l)
    }

    pub fn load(path: &Path) -> Option<Listing> {
        Listing::from_text(&std::fs::read_to_string(path).ok()?)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("server-new");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

/// The time a call may take to answer at all (a server asleep is not waited on long).
pub const CONNECT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests;
