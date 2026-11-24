//! Vergence Remote's viewer on Vergence (remote milestone 1, step 4;
//! docs/remote-milestone-1.md): another Vergence machine's desktop in a
//! window, used as if sitting at it, over SSH.
//!
//!     vergence-remote-viewer            the machine list
//!     vergence-remote-viewer HOST       straight to HOST (ssh HOST vergence-remote)
//!     vergence-remote-viewer localhost  this machine's own session
//!
//! session.rs is a session's window (the shell for Vergence over the
//! portable core, vergence-remote's client.rs); machines.rs the list.

mod machines;
mod session;

fn main() -> std::process::ExitCode {
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        Some("--help" | "-h") => {
            println!("usage: vergence-remote-viewer [HOST]   (ssh HOST vergence-remote; localhost for this machine)");
            return std::process::ExitCode::SUCCESS;
        }
        // For the tests: a stand-in for a host, as any command.
        Some("--command") => {
            let command: Vec<String> = std::env::args().skip(2).collect();
            vergence_ui::run(&mut session::Viewer::with_command("stand-in", command), (1280, 760))
        }
        Some(host) => vergence_ui::run(&mut session::Viewer::new(host), (1280, 760)),
        None => vergence_widgets::run(machines::Machines::new(), (420, 480)),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vergence-remote-viewer: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
