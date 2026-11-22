//! Asks the session's desktop portal for a screenshot or a color, as a
//! program would, and waits for the answer (screenshots milestone 1, step
//! 7's checks): `portal-shot [screenshot|interactive|color]`. Prints the
//! Request's path, then the Response's code and results.

use std::time::Duration;

use vergence_dbus::{Value, client::Client, message::Message};

fn main() {
    let what = std::env::args().nth(1).unwrap_or_else(|| "screenshot".into());
    let addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    let Some(path) = addr.split(',').find_map(|p| p.strip_prefix("unix:path=")) else {
        eprintln!("portal-shot: no unix:path= session bus ({addr:?})");
        std::process::exit(2);
    };
    let mut c = Client::connect(std::path::Path::new(path)).expect("session bus");
    c.add_match("type='signal',interface='org.freedesktop.portal.Request',member='Response'").expect("match");
    let entry = |k: &str, v: Value| Value::Entry(Box::new(Value::Str(k.into())), Box::new(Value::Variant(Box::new(v))));
    let mut opts = vec![entry("handle_token", Value::Str("check1".into()))];
    if what == "interactive" {
        opts.push(entry("interactive", Value::Bool(true)));
    }
    let member = if what == "color" { "PickColor" } else { "Screenshot" };
    let m = Message::method_call(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
        member,
        &[Value::Str(String::new()), Value::Array("{sv}".into(), opts)],
    );
    let reply = c.call(m, Duration::from_secs(10)).expect("call");
    println!("handle {:?}", reply.args());
    // Other signals (the bus's NameAcquired) are not the answer.
    let until = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < until {
        match c.next_signal(Duration::from_secs(60)) {
            Ok(Some(sig)) if sig.member.as_deref() == Some("Response") => {
                println!("response {:?}", sig.args());
                return;
            }
            Ok(_) => {}
            Err(e) => {
                println!("no response: {e}");
                return;
            }
        }
    }
    println!("no response in 60 s");
}
