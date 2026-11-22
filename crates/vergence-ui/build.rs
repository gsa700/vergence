// Where the system keeps libxkbcommon (src/keyboard.rs links it). Linux has
// it on the linker's own path; FreeBSD keeps packages' libraries under
// /usr/local/lib, which the linker does not search by itself (found on
// canary, 2026-11-09). pkg-config knows either way; without it, FreeBSD's
// usual place is added.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let from_pkg_config = std::process::Command::new("pkg-config")
        .args(["--libs-only-L", "xkbcommon"])
        .output()
        .ok()
        .filter(|o| o.status.success());
    match from_pkg_config {
        Some(out) => {
            for dir in String::from_utf8_lossy(&out.stdout).split_whitespace().filter_map(|a| a.strip_prefix("-L")) {
                println!("cargo:rustc-link-search=native={dir}");
            }
        }
        None => {
            if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("freebsd") {
                println!("cargo:rustc-link-search=native=/usr/local/lib");
            }
        }
    }
}
