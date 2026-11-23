//! Archives in Vergence (archive milestone 1, docs/archive-milestone-1.md):
//! extracted through a confined helper, made as .zip, both by libarchive
//! (borrowed with Dave's agreement, 2026-11-23: the library bsdtar is
//! built on, already on every machine Vergence runs on).
//!
//! An archive is untrusted input, as a picture or a PDF is (whitepaper
//! §6.10): its formats are read only in the helper `vergence-unpack`
//! (libexec), confined before it reads a byte (helper.rs), which passes its
//! entries on a simple stream of our own (stream.rs). The app's side
//! (unpack.rs) makes the files and decides every path: nothing outside the
//! folder it extracts into, links last and only inside, no setuid bits, no
//! devices, no more bytes than the listing said.
//!
//! Start reading: [`unpack::list`] and [`unpack::extract`] (what FileView
//! calls), then helper.rs, then [`make::compress`].
//!
//! Rule: libarchive reads an archive only inside the helper, after
//! `vergence_decode::confine`; never call its read side in an app.

mod ffi;
pub mod helper;
pub mod make;
pub mod stream;
pub mod unpack;

pub use unpack::{Error, Extracted, Listing, Progress, extract, list};
