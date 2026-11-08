// FreeBSD installs packaged libraries (libxkbcommon, libwayland, Mesa's EGL...)
// in /usr/local/lib, which the system linker does not search by default; some
// of Smithay's bindings link by name without asking pkg-config. Linux needs
// nothing here.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("freebsd") {
        println!("cargo:rustc-link-search=native=/usr/local/lib");
    }
}
