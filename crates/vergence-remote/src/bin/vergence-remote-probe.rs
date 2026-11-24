//! `vergence-remote-probe`: a viewer without a window, for tests and for
//! looking at a connection: it runs a command that reaches a host's
//! `vergence-remote` (`vergence-remote` itself on the same machine, or `ssh
//! hubble vergence-remote`), says Hello, applies and acknowledges frames, and
//! reports what came: frames, bytes on the wire, the picture's size. With
//! `--out FILE` the last picture is written as a PPM.
//!
//! vergence-remote-probe [--size WxH] [--frames N] [--seconds S] [--name N]
//!     [--screen NAME] [--out FILE] -- COMMAND [ARGS...]

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
                }
                ToViewer::Notice { text } => println!("notice: {text}"),
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
