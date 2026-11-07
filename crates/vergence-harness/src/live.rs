//! Live sweep: EDIDs from the database as debugfs fake monitors.
//!
//! Needs root (debugfs) and an X display. Found on Hubble (2026-11-06):
//! forcing a connector sends no hot-plug event, so the harness makes X
//! re-probe itself after each change, and EDID files must be written in one
//! write (`cat`'s copy path fails on debugfs).

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

use vergence_profile::profile_key;

use crate::db;

const DEBUGFS: &str = "/sys/kernel/debug/dri/0";

pub struct Options {
    pub db: PathBuf,
    /// debugfs connector name, e.g. HDMI-A-1.
    pub conn: String,
    /// The same connector as X names it, e.g. HDMI-1.
    pub output: String,
    pub sample: usize,
    pub vergd: PathBuf,
    /// A file listing database paths (relative), one per line, to run
    /// instead of the even sample.
    pub only: Option<PathBuf>,
}

impl Options {
    pub fn from_args(args: &[String]) -> Option<Options> {
        let mut o = Options {
            db: PathBuf::from(args.first()?),
            conn: String::new(),
            output: String::new(),
            sample: 200,
            vergd: PathBuf::from("target/debug/vergd"),
            only: None,
        };
        let mut it = args[1..].iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--conn" => o.conn = it.next()?.clone(),
                "--output" => o.output = it.next()?.clone(),
                "--sample" => o.sample = it.next()?.parse().ok()?,
                "--vergd" => o.vergd = PathBuf::from(it.next()?),
                "--only" => o.only = Some(PathBuf::from(it.next()?)),
                _ => return None,
            }
        }
        if o.conn.is_empty() || o.output.is_empty() || o.sample == 0 {
            return None;
        }
        Some(o)
    }
}

/// Outcome of one EDID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// X shows the same bytes and vergd's key matches.
    Ok,
    /// The kernel refused the override (EINVAL): drm validates every block
    /// and the declared length, as it does for a real monitor's EDID.
    KernelRefused,
    /// X shows the output connected but with no EDID.
    KernelDropped,
    /// X shows different bytes (kernel or X changed the EDID).
    KernelChanged,
    /// The output did not come up as connected at all.
    NotConnected,
    /// vergd's key differs from the key of what X shows.
    VergdWrongKey,
    /// vergd failed or printed no key.
    VergdFailed,
    /// The harness itself failed (debugfs write, X request); see the message.
    HarnessError,
}

struct Fake<'a> {
    dir: PathBuf,
    _o: &'a Options,
}

impl<'a> Fake<'a> {
    fn new(o: &'a Options) -> Self {
        Fake { dir: Path::new(DEBUGFS).join(&o.conn), _o: o }
    }

    fn write(&self, file: &str, data: &[u8]) -> std::io::Result<()> {
        let mut f = OpenOptions::new().write(true).open(self.dir.join(file))?;
        // One write call: debugfs takes the whole blob at once.
        f.write_all(data)
    }

    fn plug(&self, edid: &[u8]) -> std::io::Result<()> {
        self.write("edid_override", edid)?;
        self.write("force", b"on")
    }

    fn unplug(&self) -> std::io::Result<()> {
        self.write("force", b"unspecified")?;
        self.write("edid_override", b"reset")
    }
}

struct X {
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
}

impl X {
    fn connect() -> Result<X, Box<dyn std::error::Error>> {
        let (conn, n) = x11rb::connect(None)?;
        let root = conn.setup().roots[n].root;
        Ok(X { conn, root })
    }

    /// Make X re-probe every output.
    fn reprobe(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.conn.randr_get_screen_resources(self.root)?.reply()?;
        Ok(())
    }

    /// (name, connected, edid) for every output, without re-probing.
    fn outputs(&self) -> Result<Vec<(String, bool, Option<Vec<u8>>)>, Box<dyn std::error::Error>> {
        let atom = self.conn.intern_atom(true, b"EDID")?.reply()?.atom;
        let res = self.conn.randr_get_screen_resources_current(self.root)?.reply()?;
        let mut out = Vec::new();
        for &o in &res.outputs {
            let info = self.conn.randr_get_output_info(o, res.config_timestamp)?.reply()?;
            let connected = info.connection == randr::Connection::CONNECTED;
            let edid = if connected && atom != 0 {
                let any: u32 = AtomEnum::ANY.into();
                let p = self
                    .conn
                    .randr_get_output_property(o, atom, any, 0, 256, false, false)?
                    .reply()?;
                if p.format == 8 && !p.data.is_empty() { Some(p.data) } else { None }
            } else {
                None
            };
            out.push((String::from_utf8_lossy(&info.name).into_owned(), connected, edid));
        }
        Ok(out)
    }
}

/// The key `vergd probe` prints, or None.
fn vergd_key(vergd: &Path) -> Option<String> {
    let out = Command::new(vergd).arg("probe").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("profile key ").map(|r| r.split_whitespace().next().unwrap_or("").to_string()))
}

fn output_off(name: &str) {
    let _ = Command::new("xrandr").args(["--output", name, "--off"]).output();
}

pub fn run(o: &Options) -> Result<bool, Box<dyn std::error::Error>> {
    let fake = Fake::new(o);
    if !fake.dir.join("force").exists() {
        return Err(format!(
            "{} not found: run as root with lindebugfs mounted on /sys/kernel/debug",
            fake.dir.display()
        )
        .into());
    }
    let x = X::connect()?;
    if x.outputs()?.iter().any(|(n, c, _)| n == &o.output && *c) {
        return Err(format!("{} is already connected; pick a free connector", o.output).into());
    }

    let all: Vec<PathBuf> = match &o.only {
        Some(list) => std::fs::read_to_string(list)?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| o.db.join(l))
            .collect(),
        None => db::files(&o.db)?,
    };
    let step = if o.only.is_some() { 1 } else { (all.len() / o.sample).max(1) };
    let picks: Vec<&PathBuf> = all.iter().step_by(step).take(o.sample).collect();
    println!("{} of {} EDIDs, every {}th, on {} / {}", picks.len(), all.len(), step, o.conn, o.output);

    let started = Instant::now();
    let mut results: Vec<(Outcome, &PathBuf)> = Vec::new();
    for (i, path) in picks.iter().enumerate() {
        let Some(raw) = db::read_edid(path)? else { continue };
        // Inject what a monitor sends: the blocks its base block declares.
        // Database files often carry more (wrap-around reads, a second
        // EDID), which drm rejects with EINVAL.
        let edid = vergence_profile::canonical(&raw).to_vec();
        let outcome = (|| -> Result<Outcome, Box<dyn std::error::Error>> {
            match fake.plug(&edid) {
                Err(e) if e.raw_os_error() == Some(22) => return Ok(Outcome::KernelRefused),
                r => r?,
            }
            x.reprobe()?;
            let outs = x.outputs()?;
            let Some((_, connected, shown)) = outs.iter().find(|(n, _, _)| n == &o.output) else {
                return Ok(Outcome::NotConnected);
            };
            if !connected {
                return Ok(Outcome::NotConnected);
            }
            let first = match shown {
                None => Outcome::KernelDropped,
                Some(b) if b != &edid => Outcome::KernelChanged,
                Some(_) => Outcome::Ok,
            };
            // Whatever X shows, vergd must agree with it.
            let expected: Vec<&[u8]> =
                outs.iter().filter(|(_, c, _)| *c).filter_map(|(_, _, e)| e.as_deref()).collect();
            let want = profile_key(&expected);
            match vergd_key(&o.vergd) {
                None => Ok(Outcome::VergdFailed),
                Some(k) if k != want => Ok(Outcome::VergdWrongKey),
                Some(_) => Ok(first),
            }
        })();
        // Always unplug, whatever happened.
        let _ = fake.unplug();
        let _ = x.reprobe();
        output_off(&o.output);

        let rel = path.strip_prefix(&o.db).unwrap_or(path);
        let outcome = match outcome {
            Ok(oc) => oc,
            Err(e) => {
                println!("  HarnessError: {} ({} B): {e}", rel.display(), edid.len());
                Outcome::HarnessError
            }
        };
        if !matches!(outcome, Outcome::Ok | Outcome::HarnessError) {
            println!("  {:?}: {} ({} B)", outcome, rel.display(), edid.len());
        }
        results.push((outcome, path));
        if (i + 1) % 50 == 0 {
            println!("  ... {} done, {:.0} s", i + 1, started.elapsed().as_secs_f64());
        }
    }

    // The fake must be gone at the end.
    x.reprobe()?;
    let left = x.outputs()?.iter().any(|(n, c, _)| n == &o.output && *c);

    let mut counts = std::collections::BTreeMap::new();
    for (oc, _) in &results {
        *counts.entry(*oc).or_insert(0usize) += 1;
    }
    println!("done in {:.0} s:", started.elapsed().as_secs_f64());
    for (oc, n) in &counts {
        println!("  {oc:?}: {n}");
    }
    if left {
        println!("WARNING: {} still shows connected after the run", o.output);
    }
    let vergd_bad = results.iter().any(|(oc, _)| {
        matches!(oc, Outcome::VergdWrongKey | Outcome::VergdFailed | Outcome::HarnessError)
    });
    Ok(!vergd_bad && !left)
}
