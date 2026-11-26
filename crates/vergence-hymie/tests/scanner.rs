//! The library scan through the real helper (cargo's build of it): files
//! read by their ends, a big one never sent whole, a damaged one an error
//! that does not stop the next.

use std::path::{Path, PathBuf};

use vergence_hymie::helper::Scanner;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)).unwrap()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hymie-scan-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_library_is_scanned_by_one_helper() {
    let dir = scratch("lib");
    let mut big = fixture("tone16.flac");
    let frames = vergence_hymie::open(big.clone()).unwrap().frames();
    big.resize(big.len() + (20 << 20), 0);
    std::fs::write(dir.join("big.flac"), &big).unwrap();
    std::fs::write(dir.join("tone.mp3"), fixture("tone44.mp3")).unwrap();
    std::fs::write(dir.join("aac.m4a"), fixture("aac44.m4a")).unwrap();
    std::fs::write(dir.join("broken.flac"), b"fLaC and then nothing that makes sense").unwrap();

    let mut s = Scanner::with_program(Path::new(env!("CARGO_BIN_EXE_vergence-hymie")));
    let f = s.facts(&dir.join("big.flac")).unwrap();
    assert_eq!(f.frames, frames);
    assert!(s.facts(&dir.join("broken.flac")).is_err());
    let mp3 = s.facts(&dir.join("tone.mp3")).unwrap();
    assert!(mp3.rate > 0 && mp3.frames > 0);
    let aac = s.facts(&dir.join("aac.m4a")).unwrap();
    assert_eq!(aac.rate, 44100);
    assert!(matches!(s.facts(&dir.join("gone.flac")), Err(vergence_hymie::Error::Io(_))));
    std::fs::remove_dir_all(&dir).unwrap();
}
