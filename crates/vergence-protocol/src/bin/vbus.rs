// vbus: talk to a running vergence-server's session bus (whitepaper §6.8).
//
//   vbus ping                 round trip to the server
//   vbus outputs              the monitors, once
//   vbus watch <topic>        subscribe and print every event (Ctrl+C ends)
//
// Finds the bus from $VERGENCE_BUS, else $WAYLAND_DISPLAY in $XDG_RUNTIME_DIR.
// --socket <path> names it directly.

use vergence_protocol::{
    Client,
    bus::{ClientMsg, OutputInfo, ServerMsg, Topic},
};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let socket = match args.iter().position(|a| a == "--socket") {
        Some(i) if i + 1 < args.len() => {
            let p = args.remove(i + 1);
            args.remove(i);
            Some(std::path::PathBuf::from(p))
        }
        _ => None,
    };
    if let Err(e) = run(socket, &args) {
        eprintln!("vbus: {e}");
        std::process::exit(1);
    }
}

fn run(socket: Option<std::path::PathBuf>, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut c = match socket {
        Some(p) => Client::connect_to(&p, "vbus")?,
        None => Client::connect("vbus")?,
    };
    match args.first().map(String::as_str) {
        Some("ping") => {
            let t = std::time::Instant::now();
            c.request(|serial| ClientMsg::Ping { serial })?;
            println!("pong in {:.2} ms (client {})", t.elapsed().as_secs_f64() * 1000.0, c.client_id);
        }
        Some("outputs") => match c.request(|serial| ClientMsg::GetOutputs { serial })? {
            ServerMsg::Outputs { outputs, .. } => print_outputs(&outputs),
            other => return Err(format!("unexpected {other:?}").into()),
        },
        Some("watch") => {
            let name = args.get(1).map(String::as_str).unwrap_or("");
            let topic = Topic::from_name(name).ok_or_else(|| format!("no topic {name:?} (topics: outputs)"))?;
            c.request(|serial| ClientMsg::Subscribe { serial, topic })?;
            loop {
                match c.next_event()? {
                    ServerMsg::Outputs { outputs, .. } => {
                        println!("-- outputs");
                        print_outputs(&outputs);
                    }
                    other => println!("{other:?}"),
                }
            }
        }
        _ => return Err("usage: vbus [--socket <path>] ping | outputs | watch <topic>".into()),
    }
    Ok(())
}

fn print_outputs(outputs: &[OutputInfo]) {
    for o in outputs {
        println!("{} {:?} at {},{} size {}x{} scale {:.4}", o.name, o.description, o.x, o.y, o.width, o.height, o.scale);
    }
}
