//! `vergence-remote-probe`: a viewer without a window, for tests and for
//! looking at a connection: it runs a command that reaches a host's
//! `vergence-remote` (`vergence-remote` itself on the same machine, or `ssh
//! hubble vergence-remote`), says Hello, applies and acknowledges frames, and
//! reports what came: frames, bytes on the wire, the picture's size. With
//! `--out FILE` the last picture is written as a PPM.
//!
//! vergence-remote-probe [--size WxH] [--frames N] [--seconds S] [--name N]
//!     [--screen NAME] [--out FILE] [--script "CMD; CMD; ..."] -- COMMAND [ARGS...]
//!
//! The script runs after the first frame, as a viewer's input: `pointer X Y`
//! (the view's pixels), `press|release left|right|middle`, `key CODE` (an evdev
//! code, down and up), `type TEXT` (small letters, digits and spaces), `scroll
//! DX DY`, `privacy on|off` (blank the desk), `locks CAPS NUM` (0 or 1 each),
//! `down CODE` and `up CODE` (a key held, for chords), `wait MS`.

use std::io::{BufReader, BufWriter, Read, Write};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use vergence_remote::proto::{self, ToHost, ToViewer};

/// Counts the bytes read through it (what came over the wire).
struct Counted<R> {
    inner: R,
    n: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let k = self.inner.read(buf)?;
        self.n.fetch_add(k as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(k)
    }
}

/// What came: frames, the picture's size, its pixels, the scale.
type Got = (u32, (u32, u32), Vec<u8>, f32);

/// evdev codes for typing small letters, digits and spaces.
fn code_of(c: char) -> Option<u32> {
    const LETTERS: &[u32; 26] =
        &[30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44];
    match c {
        'a'..='z' => Some(LETTERS[c as usize - 'a' as usize]),
        '1'..='9' => Some(2 + c as u32 - '1' as u32),
        '0' => Some(11),
        ' ' => Some(57),
        _ => None,
    }
}

/// One script command as the messages it sends (and a pause).
fn script_step(cmd: &str) -> Result<(Vec<ToHost>, u64), String> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let num = |i: usize| -> Result<f32, String> {
        words.get(i).and_then(|w| w.parse().ok()).ok_or_else(|| format!("script: {cmd:?}"))
    };
    let button = |w: Option<&&str>| match w.copied() {
        Some("left") | None => Ok(0x110),
        Some("right") => Ok(0x111),
        Some("middle") => Ok(0x112),
        Some(other) => Err(format!("script: no button {other}")),
    };
    Ok(match words.first().copied() {
        Some("pointer") => (vec![ToHost::Pointer { x: num(1)?, y: num(2)? }], 30),
        Some("press") => (vec![ToHost::Button { code: button(words.get(1))?, pressed: true }], 30),
        Some("release") => (vec![ToHost::Button { code: button(words.get(1))?, pressed: false }], 30),
        Some("key") => {
            let code = num(1)? as u32;
            (vec![ToHost::Key { code, pressed: true }, ToHost::Key { code, pressed: false }], 30)
        }
        Some("type") => {
            let text = cmd.trim_start().strip_prefix("type").unwrap_or("").trim_start();
            let mut m = Vec::new();
            for c in text.chars() {
                let code = code_of(c).ok_or_else(|| format!("script: cannot type {c:?}"))?;
                m.push(ToHost::Key { code, pressed: true });
                m.push(ToHost::Key { code, pressed: false });
            }
            (m, 30)
        }
        Some("scroll") => (vec![ToHost::Scroll { dx: num(1)?, dy: num(2)? }], 30),
        Some("down") => (vec![ToHost::Key { code: num(1)? as u32, pressed: true }], 30),
        Some("up") => (vec![ToHost::Key { code: num(1)? as u32, pressed: false }], 30),
        Some("wait") => (Vec::new(), num(1)? as u64),
        Some("privacy") => (vec![ToHost::Privacy { on: words.get(1) == Some(&"on") }], 30),
        Some("locks") => (vec![ToHost::Locks { caps: num(1)? != 0.0, num: num(2)? != 0.0 }], 60),
        _ => return Err(format!("script: {cmd:?}")),
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let (width, height) = flag("--size")
        .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
        .unwrap_or((1280u32, 720u32));
    let frames: u32 = flag("--frames").and_then(|v| v.parse().ok()).unwrap_or(1);
    let seconds: u64 = flag("--seconds").and_then(|v| v.parse().ok()).unwrap_or(10);
    let name = flag("--name").unwrap_or_else(|| "Probe".into());
    let screen = flag("--screen").unwrap_or_default();
    let script = flag("--script");
    let Some(cmd_at) = args.iter().position(|a| a == "--") else {
        eprintln!("usage: vergence-remote-probe [--size WxH] [--frames N] [--seconds S] [--out FILE] -- COMMAND ...");
        return ExitCode::from(2);
    };
    let cmd = &args[cmd_at + 1..];
    let Some((prog, rest)) = cmd.split_first() else { return ExitCode::from(2) };
    let mut child = match Command::new(prog).args(rest).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("probe: {prog}: {e}");
            return ExitCode::from(1);
        }
    };
    let wire = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut r =
        BufReader::with_capacity(1 << 20, Counted { inner: child.stdout.take().expect("piped"), n: wire.clone() });
    let mut w = BufWriter::new(child.stdin.take().expect("piped"));
    let start = Instant::now();
    let result = (|| -> Result<Got, String> {
        proto::write_opening(&mut w).map_err(|e| e.to_string())?;
        proto::read_opening(&mut r).map_err(|e| e.to_string())?;
        ToHost::Hello { name, width, height, screen }
            .write(&mut w)
            .and_then(|_| w.flush())
            .map_err(|e| e.to_string())?;
        let (mut picture, mut size, mut got, mut scale) = (Vec::new(), (0u32, 0u32), 0u32, 0f32);
        while got < frames {
            if start.elapsed() > Duration::from_secs(seconds) {
                return Err(format!("only {got} frame(s) in {seconds} s"));
            }
            match ToViewer::read(&mut r).map_err(|e| e.to_string())? {
                ToViewer::Welcome { host, screens } => println!("welcome: {host}, screens {screens:?}"),
                ToViewer::Frame(f) => {
                    f.apply(&mut picture, &mut size).map_err(|e| e.to_string())?;
                    got += 1;
                    scale = f.scale;
                    println!(
                        "frame {}: {}x{} scale {:.3}, {} rectangle(s)",
                        f.seq,
                        f.width,
                        f.height,
                        f.scale,
                        f.rects.len()
                    );
                    ToHost::Ack { seq: f.seq }.write(&mut w).and_then(|_| w.flush()).map_err(|e| e.to_string())?;
                    if got == 1
                        && let Some(script) = &script
                    {
                        for cmd in script.split(';').map(str::trim).filter(|c| !c.is_empty()) {
                            let (msgs, pause) = script_step(cmd)?;
                            for m in msgs {
                                m.write(&mut w).map_err(|e| e.to_string())?;
                                w.flush().map_err(|e| e.to_string())?;
                                std::thread::sleep(Duration::from_millis(8));
                            }
                            std::thread::sleep(Duration::from_millis(pause));
                        }
                        println!("script: done");
                    }
                }
                ToViewer::Notice { text } => println!("notice: {text}"),
                ToViewer::Privacy { on } => println!("privacy: {}", if on { "on" } else { "off" }),
                ToViewer::Bye { reason } => return Err(format!("bye: {reason}")),
            }
        }
        Ok((got, size, picture, scale))
    })();
    let _ = ToHost::Bye.write(&mut w).and_then(|_| w.flush());
    drop(w);
    let _ = child.wait();
    match result {
        Ok((got, size, picture, scale)) => {
            let bytes = wire.load(std::sync::atomic::Ordering::Relaxed);
            let raw = size.0 as u64 * size.1 as u64 * 4;
            println!(
                "probe: {got} frame(s), {}x{} scale {scale:.3}, {bytes} bytes on the wire ({:.1}% of one raw picture), {} ms",
                size.0,
                size.1,
                bytes as f64 * 100.0 / raw.max(1) as f64,
                start.elapsed().as_millis()
            );
            if let Some(out) = flag("--out") {
                let mut ppm = format!("P6\n{} {}\n255\n", size.0, size.1).into_bytes();
                for px in picture.as_chunks::<4>().0 {
                    ppm.extend_from_slice(&px[..3]);
                }
                if let Err(e) = std::fs::write(&out, ppm) {
                    eprintln!("probe: {out}: {e}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("probe: {e}");
            ExitCode::from(1)
        }
    }
}
