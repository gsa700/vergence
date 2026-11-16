// PAM is the operating system's interface for logging in (Dave, 2026-11-16:
// not borrowing): linked from the system through our own declarations
// (src/pam.rs). Void ships only libpam.so.0, no development link, so Linux
// names the file itself; FreeBSD's base system has libpam.so.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-link-lib=dylib:+verbatim=libpam.so.0");
    } else {
        println!("cargo:rustc-link-lib=dylib=pam");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
