// vbus: talk to a running vergence-server's session bus (whitepaper §6.8).
//
//   vbus ping                 round trip to the server
//   vbus window [title]       ask for a native window (native milestone 1)
//   vbus outputs              the monitors, once
//   vbus settings             the desktop settings, once
//   vbus watch <topic>        subscribe and print every event (Ctrl+C ends)
//   vbus notify <summary> [body] [--critical] [--action key=Label]...
//                             show a notification and wait until it closes
//
// Finds the bus from $VERGENCE_BUS, else $WAYLAND_DISPLAY in $XDG_RUNTIME_DIR.
// --socket <path> names it directly.

use vergence_protocol::{
    Client,
    bus::{ClientMsg, OutputInfo, ServerMsg, Setting, SettingValue, Topic},
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
        Some("settings") => match c.request(|serial| ClientMsg::GetSettings { serial })? {
            ServerMsg::Settings { settings, .. } => print_settings(&settings),
            other => return Err(format!("unexpected {other:?}").into()),
        },
        Some("watch") => {
            let name = args.get(1).map(String::as_str).unwrap_or("");
            let topic = Topic::from_name(name).ok_or_else(|| format!("no topic {name:?} (topics: outputs, settings)"))?;
            c.request(|serial| ClientMsg::Subscribe { serial, topic })?;
            loop {
                match c.next_event()? {
                    ServerMsg::Outputs { outputs, .. } => {
                        println!("-- outputs");
                        print_outputs(&outputs);
                    }
                    ServerMsg::Settings { complete, settings, .. } => {
                        println!("-- settings{}", if complete { " (all)" } else { " (changed)" });
                        print_settings(&settings);
                    }
                    other => println!("{other:?}"),
                }
            }
        }
        Some("notify") => {
            let mut words = Vec::new();
            let mut actions = Vec::new();
            let mut urgency = 1;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--critical" => urgency = 2,
                    "--action" => {
                        let kv = it.next().ok_or("--action key=Label")?;
                        let (k, l) = kv.split_once('=').ok_or("--action key=Label")?;
                        actions.push((k.to_string(), l.to_string()));
                    }
                    _ => words.push(a.clone()),
                }
            }
            let summary = words.first().cloned().ok_or("vbus notify <summary> [body]")?;
            let body = words.get(1).cloned().unwrap_or_default();
            let id = match c.request(|serial| ClientMsg::Notify { serial, replaces: 0, summary, body, urgency, timeout_ms: -1, actions })? {
                ServerMsg::Notified { id, .. } => id,
                other => return Err(format!("unexpected {other:?}").into()),
            };
            println!("notification {id}");
            loop {
                match c.next_event()? {
                    ServerMsg::ActionInvoked { id: i, key } if i == id => println!("action {key:?}"),
                    ServerMsg::NotificationClosed { id: i, reason } if i == id => {
                        println!("closed ({})", match reason { 1 => "expired", 2 => "dismissed", 3 => "closed", _ => "?" });
                        break;
                    }
                    _ => {}
                }
            }
        }
        // Ask for a native window (native milestone 1) and say what the server answers.
        Some("window") => {
            let title = args.get(1).cloned().unwrap_or_else(|| "vbus".into());
            let req = |serial| {
                ClientMsg::Window(vergence_protocol::window::WinRequest::Create {
                    serial,
                    window: 1,
                    role: "test".into(),
                    token: String::new(),
                    title: title.clone(),
                    width: 400.0,
                    height: 300.0,
                    own_frame: false,
                    parent: 0,
                    place: vergence_protocol::window::Place::Anywhere,
                })
            };
            match c.request(req) {
                Ok(m) => println!("{m:?}"),
                Err(e) => println!("the server answered: {e}"),
            }
        }
        _ => return Err("usage: vbus [--socket <path>] ping | outputs | settings | watch <topic> | notify <summary> [body] | window [title]".into()),
    }
    Ok(())
}

fn print_outputs(outputs: &[OutputInfo]) {
    for o in outputs {
        println!("{} {:?} at {},{} size {}x{} scale {:.4} ({}) id {}", o.name, o.description, o.x, o.y, o.width, o.height, o.scale, o.note, o.id);
    }
}

fn print_settings(settings: &[Setting]) {
    for s in settings {
        let v = match &s.value {
            SettingValue::Bool(b) => b.to_string(),
            SettingValue::U32(x) => x.to_string(),
            SettingValue::I32(x) => x.to_string(),
            SettingValue::F64(x) => x.to_string(),
            SettingValue::Str(x) => format!("{x:?}"),
            SettingValue::Rgb(r, g, b) => format!("rgb({r:.3}, {g:.3}, {b:.3})"),
        };
        println!("{} {} = {v}", s.namespace, s.key);
    }
}
