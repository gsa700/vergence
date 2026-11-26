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
    // The tone's embedded picture, decoded in the helper.
    let (w, h, rgba) = s.cover(&dir.join("big.flac"), 32).unwrap().expect("a cover");
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    assert!(w >= 32 && h >= 32);
    assert_eq!(s.cover(&dir.join("tone.mp3"), 32).unwrap(), None);
    let aac = s.facts(&dir.join("aac.m4a")).unwrap();
    assert_eq!(aac.rate, 44100);
    assert!(matches!(s.facts(&dir.join("gone.flac")), Err(vergence_hymie::Error::Io(_))));
    // A sidecar measured by the helper: a PNG header of 640 x 480.
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    png.extend_from_slice(&640u32.to_be_bytes());
    png.extend_from_slice(&480u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(dir.join("cover.png"), &png).unwrap();
    assert_eq!(s.image_size(&dir.join("cover.png")), Some((640, 480)));
    std::fs::write(dir.join("notes.txt"), b"not a picture").unwrap();
    assert_eq!(s.image_size(&dir.join("notes.txt")), None);
    std::fs::remove_dir_all(&dir).unwrap();
}
