//! `vergence-decode` (libexec): decodes one picture from standard input,
//! limited and apart from the program that asked (vergence-decode's lib.rs).

fn main() -> std::process::ExitCode {
    vergence_decode::helper_main()
}
