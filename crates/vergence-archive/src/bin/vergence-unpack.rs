//! `vergence-unpack` (libexec): reads one archive from its standard input
//! and passes its entries on, confined and apart from the app that asked
//! (vergence-archive's helper.rs).

fn main() -> std::process::ExitCode {
    vergence_archive::helper::helper_main()
}
