//! The real helper, confined (Capsicum or seccomp), through the client.

use std::path::PathBuf;
use vergence_pdf::helper::{Error, Viewer, tiny_pdf};

#[test]
fn viewer_through_the_confined_helper() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_vergence-pdf"));
    let mut v = Viewer::with_bytes(exe, tiny_pdf()).expect("the helper opens the file");
    assert_eq!(v.info.pages, vec![(100.0, 50.0)]);
    let t = v.render(0, 1.0, None).unwrap();
    assert_eq!((t.width, t.height), (100, 50));
    // The square is at x 10..30, y (from the top) 20..40.
    assert_eq!(t.rgba[(30 * 100 + 20) * 4], 0);
    assert_eq!(t.rgba[(5 * 100 + 50) * 4], 255);
    assert!(matches!(v.render(5, 1.0, None), Err(Error::Refused(_))));
    // A crash: the next request says so, the one after starts a new helper.
    v.kill_helper();
    assert!(matches!(v.render(0, 1.0, None), Err(Error::Crashed(_))));
    assert_eq!(v.render(0, 1.0, None).map(|t| t.width).ok(), Some(100));
}
