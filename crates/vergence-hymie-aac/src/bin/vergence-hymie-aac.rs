//! `vergence-hymie-aac` (libexec): the sound helper for AAC .m4a files. The
//! same program and protocol as `vergence-hymie` (vergence-hymie's
//! helper.rs), confined the same way, with an open that plays AAC through
//! this crate's decoder (FAAD2's, GPL-2.0-or-later). The client starts it
//! when the main helper says a file is AAC.

fn main() -> std::process::ExitCode {
    vergence_hymie::helper::helper_main_with(vergence_hymie_aac::open)
}
