// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
#![allow(irrefutable_let_patterns)]

mod handlers;

mod grabs;
mod input;
mod state;
mod winit;
mod drm;
mod cursor;
mod focus;
mod decor;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::Server;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let mut event_loop: EventLoop<Server> = EventLoop::try_new()?;

    let display: Display<Server> = Display::new()?;

    let mut state = Server::new(&mut event_loop, display);

    // --drm: the real monitors (from a text console, through libseat); else nested.
    if std::env::args().any(|a| a == "--drm") {
        crate::drm::run(&mut event_loop, &mut state)?;
    } else {
        crate::winit::init_winit(&mut event_loop, &mut state)?;
    }

    // Set WAYLAND_DISPLAY to our socket name, so child processes connect to Server rather
    // than the host compositor
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    // X11 apps through Xwayland; the first app starts once it is ready (or failed),
    // so it inherits DISPLAY.
    state.start_xwayland(spawn_client);

    event_loop.run(None, &mut state, move |_| {
        // Server is running
    })?;

    Ok(())
}

/// A scale rounded to 120ths, the unit wp-fractional-scale-v1 gives apps: outputs use
/// exactly what apps draw at, so nothing is resampled (server milestone 1, stage 2).
pub fn scale_in_120ths(exact: f64) -> f64 {
    ((exact * 120.0).round() / 120.0).max(1.0 / 120.0)
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}

fn spawn_client() {
    let mut args = std::env::args().skip(1).filter(|a| a != "--drm");
    let flag = args.next();
    let arg = args.next();

    match (flag.as_deref(), arg) {
        (Some("-c") | Some("--command"), Some(command)) => {
            std::process::Command::new(command).spawn().ok();
        }
        _ => {
            std::process::Command::new("foot").spawn().ok();
        }
    }
}
