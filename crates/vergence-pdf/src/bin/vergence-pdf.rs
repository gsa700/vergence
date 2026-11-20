//! `vergence-pdf` (libexec): reads one PDF document from its pipe and
//! draws its pages on request, confined and apart from the app that asked
//! (vergence-pdf's helper.rs).

fn main() -> std::process::ExitCode {
    vergence_pdf::helper::helper_main()
}
