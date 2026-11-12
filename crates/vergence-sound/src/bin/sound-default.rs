//! sound-default: FreeBSD's root helper for the default sound output (sound
//! milestone 1, Decided 1). `sound-default N` sets `hw.snd.default_unit` to
//! N, a device that plays, and does nothing else; the checks are in
//! `vergence_sound::oss::helper_main`. tools/install.sh puts it in
//! PREFIX/libexec/vergence, owned by root, group video, mode 4750: setuid
//! root, runnable only by the group the session's users are in. The settings
//! app and `vsound default` run it when the system refuses them.

fn main() {
    #[cfg(target_os = "freebsd")]
    std::process::exit(vergence_sound::oss::helper_main(std::env::args_os().skip(1).collect()));
    #[cfg(not(target_os = "freebsd"))]
    {
        eprintln!("sound-default: only FreeBSD needs this helper");
        std::process::exit(1);
    }
}
