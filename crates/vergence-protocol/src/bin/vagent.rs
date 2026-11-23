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
//   vagent read <client> <window> <node> [offset] [len]
//                                          a widget's text: one page from offset
//                                          (characters), or the whole of it in pages
//   vagent terminal open [cwd]             a terminal of the agent's own (bus v38):
//                                          the person is asked; prints its window and tab
//   vagent type <client> <window> <node> <text...>
//                                          type into a terminal's pane ("\n" ends a line)
//   vagent key <client> <window> <node> <name>
//                                          a key: Return, Tab, Escape, BackSpace, Up,
//                                          Down, Left, Right, Home, End, ctrl+c, ...
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
    "usage: vagent windows | tree <client> <window> | find <path> | focused | do <action> <path> [value] | act <client> <window> <node> <action> [value] | changes [since] | read <client> <window> <node> [offset] [len] | terminal open [cwd] | type <client> <window> <node> <text> | key <client> <window> <node> <name>".into()
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
        // A terminal's pane (bus v38): "\n" in the text is a line's end.
        "type" if !value.is_empty() => Action::Type(value.replace("\\n", "\n")),
        "key" if vergence_protocol::window::key_name_ok(&value) => Action::Key(value),
        "key" => return Err(format!("no key {value:?} (Return, Tab, Escape, BackSpace, Up, Down, Left, Right, Home, End, ctrl+<letter>, ...)")),
        _ => return Err(format!("no action {verb:?} (press, focus, reveal, set <value>, choose <item>, type <text>, key <name>)")),
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
        // A widget's whole text, past the tree's 512-character cut (bus v37).
        "read" => {
            let (client, window, node) = (num(1)? as u32, num(2)? as u32, num(3)?);
            let page = |c: &mut _, offset: u32, len: u32| -> Result<(u32, String), String> {
                match ask(c, |serial| AgentRequest::Read {
                    serial,
                    client,
                    window,
                    node,
                    offset,
                    len,
                    role: String::new(),
                    name: String::new(),
                })? {
                    AgentEvent::Text { total, text, .. } => Ok((total, text)),
                    other => Err(format!("unexpected {other:?}")),
                }
            };
            match args.get(4) {
                // One page, and where it ends of the whole.
                Some(o) => {
                    let offset: u32 = o.parse().map_err(|_| usage())?;
                    let len = args.get(5).and_then(|l| l.parse().ok()).unwrap_or(u32::MAX);
                    let (total, text) = page(&mut c, offset, len)?;
                    let end = offset + text.chars().count() as u32;
                    Ok(format!("{text}\n[{offset}..{end} of {total}]"))
                }
                // The whole text, page after page.
                None => {
                    let mut all = String::new();
                    let mut offset = 0u32;
                    loop {
                        let (total, text) = page(&mut c, offset, u32::MAX)?;
                        let got = text.chars().count() as u32;
                        all.push_str(&text);
                        offset += got;
                        if got == 0 || offset >= total {
                            break Ok(format!("{all}\n[{offset} characters]"));
                        }
                    }
                }
            }
        }
        // A terminal of the agent's own (bus v38): the reply comes once the
        // person has answered and its window has said what it means.
        "terminal" if args.get(1).map(String::as_str) == Some("open") => {
            let cwd = args.get(2).cloned().unwrap_or_default();
            match ask(&mut c, |serial| AgentRequest::OpenTerminal { serial, cwd })? {
                AgentEvent::Terminal { client, window, tab, .. } => {
                    Ok(format!("terminal {client} {window} tab {tab} (its pane is the tab's \"terminal\" child: vagent tree {client} {window})"))
                }
                other => Err(format!("unexpected {other:?}")),
            }
        }
        "type" | "key" => {
            let (client, window, node) = (num(1)? as u32, num(2)? as u32, num(3)?);
            let action = action(cmd, rest(4))?;
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
