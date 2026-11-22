//! `vergence-open`: opens files and links with the applications chosen for
//! their types (files milestone 1), as File Viewer and the dock do. Also
//! says what a file is and which applications could open it.
//!
//!     vergence-open FILE|LINK...          open each with its default application
//!     vergence-open --with APP FILE...    open with that application (desktop id)
//!     vergence-open --type FILE...        print each one's type and what it is called
//!     vergence-open --apps FILE...        print the applications that open each, best first
//!     vergence-open --default TYPE APP    make APP the default for TYPE (Vergence's list)
//!
//! In a Vergence session the server opens them (the bus's `Open`), so the
//! dock shows each application starting; elsewhere this starts them
//! itself. Applications are desktop entries ("org.example.App" or with
//! ".desktop"); choices come from the mimeapps.list files, Vergence's own
//! first. An application that wants a terminal runs in ours.

use std::process::ExitCode;

use vergence_shell::{apps, open};

fn usage() -> ExitCode {
    eprintln!(
        "usage: vergence-open [--with APP] FILE|LINK...\n       vergence-open --type|--apps FILE|LINK...\n       vergence-open --default TYPE APP"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(first) = args.first() else { return usage() };
    match first.as_str() {
        "--type" => {
            for given in &args[1..] {
                let i = open::item(given);
                println!("{}\t{}\t{}", i.given, i.mime, vergence_types::describe(&i.mime));
            }
            ExitCode::SUCCESS
        }
        "--apps" => {
            let (entries, lists) = (apps::installed(), vergence_types::lists());
            for given in &args[1..] {
                let i = open::item(given);
                let found = open::apps_for(&i.mime, &entries, &lists);
                println!(
                    "{} ({}): {}",
                    i.given,
                    i.mime,
                    if found.is_empty() { "none".into() } else { found.join(", ") }
                );
            }
            ExitCode::SUCCESS
        }
        "--default" => {
            let [_, mime, app] = &args[..] else { return usage() };
            match vergence_types::choose_default(mime, &open::desktop_id(app)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("vergence-open: {}: {e}", vergence_types::own_list_path().display());
                    ExitCode::FAILURE
                }
            }
        }
        "--with" => match args.get(1) {
            Some(app) => run(&args[2..], Some(&open::desktop_id(app))),
            None => usage(),
        },
        _ if first.starts_with("--") => usage(),
        _ => run(&args, None),
    }
}

/// Opens the items: through the server in a Vergence session, else here.
fn run(given: &[String], app: Option<&str>) -> ExitCode {
    let items: Vec<open::Item> = given.iter().map(|g| open::item(g)).collect();
    if std::env::var_os(vergence_protocol::BUS_ENV).is_some()
        && let Ok(mut bus) = vergence_protocol::Client::connect("vergence-open")
    {
        let all: Vec<String> = items.iter().map(|i| i.arg.clone()).collect();
        let reply = bus.request(|serial| vergence_protocol::bus::ClientMsg::Open {
            serial,
            items: all,
            app: app.unwrap_or_default().to_string(),
        });
        return match reply {
            Ok(vergence_protocol::bus::ServerMsg::Done { .. }) => ExitCode::SUCCESS,
            Err(vergence_protocol::ClientError::Refused(_, message))
            | Ok(vergence_protocol::bus::ServerMsg::Error { message, .. }) => {
                eprintln!("vergence-open: {message}");
                ExitCode::FAILURE
            }
            other => {
                eprintln!("vergence-open: the server did not answer as expected: {other:?}");
                ExitCode::FAILURE
            }
        };
    }
    let (groups, failed) = open::plan(&items, app, &apps::installed(), &vergence_types::lists());
    for f in &failed {
        eprintln!("vergence-open: {f}");
    }
    let mut ok = failed.is_empty();
    for (entry, files) in groups {
        for argv in open::commands(&entry, &files) {
            let Some((program, rest)) = argv.split_first() else { continue };
            if let Err(e) = std::process::Command::new(program).args(rest).stdin(std::process::Stdio::null()).spawn() {
                eprintln!("vergence-open: {}: {e}", entry.name);
                ok = false;
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
