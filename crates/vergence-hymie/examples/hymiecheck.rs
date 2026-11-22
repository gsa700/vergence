//! `hymiecheck FILE...`: each file through the confined helper (the one
//! cargo built beside this example), its facts, tags and waveform; FLACs also
//! decoded here and checked against the MD5 they keep. For trying the helper
//! on a real machine: `cargo run --example hymiecheck -- ~/Music/x.flac`.

fn main() {
    let helper = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("vergence-hymie"))).filter(|p| p.is_file());
    for arg in std::env::args().skip(1) {
        let path = std::path::Path::new(&arg);
        let t = std::time::Instant::now();
        let opened = match &helper {
            Some(h) => vergence_hymie::read_file(path).and_then(|d| vergence_hymie::helper::Sound::with_bytes(h, d)),
            None => vergence_hymie::helper::Sound::open(path),
        };
        let mut s = match opened {
            Ok(s) => s,
            Err(e) => {
                println!("{arg}: {e}");
                continue;
            }
        };
        let i = &s.info;
        println!("{arg}: {} Hz, {} ch, {} bit, {:.2} s; {:?} / {:?} / {:?}; cover {}", i.rate, i.channels, i.bits, i.seconds(), i.title, i.artist, i.album, i.cover.as_ref().map_or("none".into(), |(w, h, _)| format!("{w}x{h}")));
        let mut slices = 0;
        loop {
            match s.peaks(1000) {
                Ok((_, p)) if p.is_empty() => break,
                Ok((_, p)) => slices += p.len(),
                Err(e) => {
                    println!("  waveform: {e}");
                    break;
                }
            }
        }
        let check = if arg.to_ascii_lowercase().ends_with(".flac") { vergence_hymie::read_file(path).map(|d| format!("{:?}", vergence_hymie::flac_check(&d))).unwrap_or_default() } else { String::new() };
        println!("  waveform {slices} slices in {:.0} ms {check}", t.elapsed().as_secs_f64() * 1000.0);
    }
}
