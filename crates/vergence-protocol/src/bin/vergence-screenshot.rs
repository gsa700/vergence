// Screenshot (screenshots milestone 1; Dave, 2026-11-21: "How will users
// know about all this? Maybe we should make a little utility that calls
// the bar directly"): the app in Apps (and on the dock, if pinned) that
// opens Super+Shift+5's bar on the screen under the pointer, as the Mac's
// Screenshot app does. The bar shows the keys under it, so it teaches
// them too. Asks the session's server over the bus (v32 ScreenshotBar)
// and ends.

use vergence_protocol::{
    Client,
    bus::{ClientMsg, ServerMsg},
};

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("vergence-screenshot {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let result = Client::connect("vergence-screenshot").map_err(|e| format!("no Vergence session: {e}")).and_then(|mut c| {
        match c.request(|serial| ClientMsg::ScreenshotBar { serial }).map_err(|e| e.to_string())? {
            ServerMsg::Done { .. } => Ok(()),
            ServerMsg::Error { message, .. } => Err(message),
            other => Err(format!("unexpected {other:?}")),
        }
    });
    if let Err(e) = result {
        eprintln!("vergence-screenshot: {e}");
        std::process::exit(1);
    }
}
