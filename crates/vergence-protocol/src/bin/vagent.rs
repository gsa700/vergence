// vagent: the agent interface from a shell (meaning milestone 1, step 3):
// read the windows' meaning and act on widgets by name, as an agent would.
// For tests and scripts; vergence-agent-mcp (step 6) speaks the same
// interface for agents themselves.
//
//   vagent windows                         every window (native ones have meaning)
//   vagent tree <client> <window>          a window's widgets, on the screens
//   vagent find "<path>"                   widgets by path: "Calculator > 7",
//                                          "Windows > switch:Minimize", "* > button:Save"
//   vagent focused                         the focused widget and the caret
//   vagent do <action> "<path>" [value...] find exactly one widget, then act on it:
//                                          press, focus, reveal, set <value>, choose <item>
//   vagent act <client> <window> <node> <action> [value...]
//   vagent changes [since]                 what changed since a number
//
// --name N says who the agent is (default "vagent"). Finds the bus from
// $VERGENCE_BUS, else $WAYLAND_DISPLAY in $XDG_RUNTIME_DIR.

use vergence_protocol::{
    Client,
    agent::{AgentEvent, AgentFound, AgentRequest},
    bus::{ClientMsg, ServerMsg},
    window::{Action, Node},
};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut name = "vagent".to_string();
    if let Some(i) = args.iter().position(|a| a == "--name")
        && let Some(n) = args.get(i + 1).cloned()
    {
        name = n;
        args.drain(i..i + 2);
    }
    match run(&name, &args) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("vagent: {e}");
            std::process::exit(1);
        }
    }
}

fn usage() -> String {
    "usage: vagent windows | tree <client> <window> | find <path> | focused | do <action> <path> [value] | act <client> <window> <node> <action> [value] | changes [since]".into()
}

fn ask(c: &mut Client, make: impl FnOnce(u32) -> AgentRequest) -> Result<AgentEvent, String> {
    match c.request(|serial| ClientMsg::Agent(make(serial))).map_err(|e| e.to_string())? {
        ServerMsg::Agent(e) => Ok(e),
        other => Err(format!("unexpected {other:?}")),
    }
}

fn action(verb: &str, value: String) -> Result<Action, String> {
    Ok(match verb {
        "press" => Action::Press,
        "focus" => Action::Focus,
        "reveal" => Action::Reveal,
        "set" => Action::Set(value),
        "choose" if !value.is_empty() => Action::Choose(value),
        _ => return Err(format!("no action {verb:?} (press, focus, reveal, set <value>, choose <item>)")),
    })
}

fn node_line(n: &Node) -> String {
    let value = n.value.as_ref().map(|v| format!(" = {v:?}")).unwrap_or_default();
    let states = if n.states.is_empty() { String::new() } else { format!(" [{}]", n.states.join(", ")) };
    let items = if n.items.is_empty() { String::new() } else { format!(" items {:?}", n.items) };
    format!("#{} {} {:?}{value}{states}{items} at {:.0},{:.0} {:.0}x{:.0}", n.id, n.role, n.name, n.x, n.y, n.w, n.h)
}

fn found_line(f: &AgentFound) -> String {
    format!("{}/{} {:?} {} on {}", f.client, f.window, f.title, node_line(&f.node), f.screen)
}

fn acted(e: AgentEvent) -> Result<String, String> {
    match e {
        AgentEvent::Acted { ok: true, .. } => Ok("ok".into()),
        AgentEvent::Acted { why, .. } => Err(format!("refused: {why}")),
        other => Err(format!("unexpected {other:?}")),
    }
}

fn run(name: &str, args: &[String]) -> Result<String, String> {
    let Some(cmd) = args.first() else { return Err(usage()) };
    let mut c = Client::connect("vagent").map_err(|e| format!("no Vergence session bus: {e}"))?;
    match c
        .request(|serial| ClientMsg::Agent(AgentRequest::Hello { serial, name: name.into() }))
        .map_err(|e| e.to_string())?
    {
        ServerMsg::Done { .. } => {}
        other => return Err(format!("unexpected {other:?}")),
    }
    let num = |i: usize| -> Result<u64, String> { args.get(i).and_then(|a| a.parse().ok()).ok_or_else(usage) };
    let rest = |i: usize| args.get(i..).map(|v| v.join(" ")).unwrap_or_default();
    match cmd.as_str() {
        "windows" => match ask(&mut c, |serial| AgentRequest::Windows { serial })? {
            AgentEvent::Windows { windows, .. } => Ok(windows
                .iter()
                .map(|w| {
                    let mark = if w.keyboard { "> " } else { "  " };
                    let meaning = if w.meaning { "" } else { " (no meaning)" };
                    let allowed = match (w.allowed.as_str(), w.meaning) {
                        (_, false) | ("", _) => String::new(),
                        ("timed", _) => format!(", allowed for {} more min", w.minutes_left),
                        (a, _) => format!(", allowed {a}"),
                    };
                    format!(
                        "{mark}{}/{} {} {:?} ({}) at {:.0},{:.0} {:.0}x{:.0} on {}{meaning}{allowed}",
                        w.client, w.window, w.kind, w.title, w.app_id, w.at.0, w.at.1, w.at.2, w.at.3, w.screen
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")),
            other => Err(format!("unexpected {other:?}")),
        },
        "tree" => {
            let (client, window) = (num(1)? as u32, num(2)? as u32);
            match ask(&mut c, |serial| AgentRequest::Tree { serial, client, window })? {
                AgentEvent::Tree { title, focus, caret, nodes, .. } => {
                    let mut out = vec![format!("{title:?}, focus #{focus}")];
                    out.extend(
                        nodes.iter().map(|n| format!("{}{}", if n.id == focus { "> " } else { "  " }, node_line(n))),
                    );
                    if let Some((x, y, w, h)) = caret {
                        out.push(format!("  caret at {x:.0},{y:.0} {w:.0}x{h:.0}"));
                    }
                    Ok(out.join("\n"))
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "find" => match ask(&mut c, |serial| AgentRequest::Find { serial, path: rest(1) })? {
            AgentEvent::Found { found, .. } if found.is_empty() => Err("nothing there".into()),
            AgentEvent::Found { found, .. } => Ok(found.iter().map(found_line).collect::<Vec<_>>().join("\n")),
            other => Err(format!("unexpected {other:?}")),
        },
        "focused" => match ask(&mut c, |serial| AgentRequest::Focused { serial })? {
            AgentEvent::Focused { found: None, .. } => Ok("nothing focused in a native window".into()),
            AgentEvent::Focused { found: Some(f), caret, .. } => Ok(format!(
                "{}{}",
                found_line(&f),
                caret.map(|(x, y, w, h)| format!("\ncaret at {x:.0},{y:.0} {w:.0}x{h:.0}")).unwrap_or_default()
            )),
            other => Err(format!("unexpected {other:?}")),
        },
        "do" => {
            let (Some(verb), Some(path)) = (args.get(1), args.get(2)) else { return Err(usage()) };
            let found = match ask(&mut c, |serial| AgentRequest::Find { serial, path: path.clone() })? {
                AgentEvent::Found { found, .. } => found,
                other => return Err(format!("unexpected {other:?}")),
            };
            let f = match found.as_slice() {
                [one] => one.clone(),
                [] => return Err(format!("nothing at {path:?}")),
                many => {
                    return Err(format!(
                        "{} widgets at {path:?}; name one more exactly:\n{}",
                        many.len(),
                        many.iter().map(found_line).collect::<Vec<_>>().join("\n")
                    ));
                }
            };
            let action = action(verb, rest(3))?;
            acted(ask(&mut c, |serial| AgentRequest::Act {
                serial,
                client: f.client,
                window: f.window,
                node: f.node.id,
                action,
                role: f.node.role.clone(),
                name: f.node.name.clone(),
            })?)
        }
        "act" => {
            let (client, window, node) = (num(1)? as u32, num(2)? as u32, num(3)?);
            let action = action(args.get(4).ok_or_else(usage)?, rest(5))?;
            acted(ask(&mut c, |serial| AgentRequest::Act {
                serial,
                client,
                window,
                node,
                action,
                role: String::new(),
                name: String::new(),
            })?)
        }
        "changes" => {
            let since = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0);
            match ask(&mut c, |serial| AgentRequest::Changes { serial, since })? {
                AgentEvent::Changes { latest, changes, .. } => {
                    let mut out: Vec<String> = changes.iter().map(|(n, ch)| format!("{n} {ch:?}")).collect();
                    out.push(format!("latest {latest}"));
                    Ok(out.join("\n"))
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
        _ => Err(usage()),
    }
}
