//! The AAC helper program as the client starts it: confined, over its
//! pipes, answering D with the file's facts and tags, then playing.

use std::path::Path;
use vergence_hymie::helper::Sound;

const AAC44: &[u8] = include_bytes!("../../vergence-hymie/fixtures/aac44.m4a");

fn check(s: &mut Sound) {
    let i = &s.info;
    assert_eq!((i.rate, i.channels, i.bits, i.frames), (44_100, 2, 0, 66_150));
    assert_eq!(i.title.as_deref(), Some("AAC Stereo"));
    assert_eq!(i.artist.as_deref(), Some("Vergence"));
    assert_eq!(i.album.as_deref(), Some("Hymie Fixtures"));
    s.seek(10_000).unwrap();
    let samples = s.read(1000).unwrap();
    assert_eq!(samples.len(), 2000);
    let peak = samples.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!((0.4..0.6).contains(&peak), "peak {peak}");
}

#[test]
fn the_aac_helper_answers_for_an_aac_file() {
    let program = Path::new(env!("CARGO_BIN_EXE_vergence-hymie-aac"));
    let mut s = Sound::with_bytes(program, AAC44.to_vec()).unwrap();
    check(&mut s);
}

#[test]
fn the_main_helper_sends_aac_on_to_it() {
    // When the main helper was built beside this one (`cargo build`, as
    // install.sh's release build does), the client asks it first, is told
    // "AAC", and starts vergence-hymie-aac beside it with the same bytes.
    let main = Path::new(env!("CARGO_BIN_EXE_vergence-hymie-aac")).with_file_name("vergence-hymie");
    if !main.is_file() {
        eprintln!("{} not built: build -p vergence-hymie --bins to check the hand-over", main.display());
        return;
    }
    let mut s = Sound::with_bytes(&main, AAC44.to_vec()).unwrap();
    check(&mut s);
}
