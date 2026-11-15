//! The FreeBSD eject helper (setuid root, the video group only): see
//! vergence_media::helper_main.

fn main() {
    std::process::exit(vergence_media::helper_main(std::env::args_os().skip(1).collect()));
}
