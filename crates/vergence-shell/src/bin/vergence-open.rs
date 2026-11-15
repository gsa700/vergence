//! `vergence-open`: opens files and links with the applications chosen for
//! their types (files milestone 1, step 1), as File Viewer and the dock
//! will. Also says what a file is and which applications could open it.
//!
//!     vergence-open FILE|LINK...          open each with its default application
//!     vergence-open --with APP FILE...    open with that application (desktop id)
//!     vergence-open --type FILE...        print each one's type and what it is called
//!     vergence-open --apps FILE...        print the applications that open each, best first
//!     vergence-open --default TYPE APP    make APP the default for TYPE (Vergence's list)
//!
//! Applications are desktop entries ("org.example.App" or with ".desktop");
//! choices come from the mimeapps.list files, Vergence's own first. An
//! application that wants a terminal runs in ours (`vergence-term -e`).

use std::path::PathBuf;
use std::process::ExitCode;

use vergence_shell::{apps, desktop};

/// One thing to open: as given, what to pass the application, and its type.
struct Item {
    given: String,
    arg: String,
    mime: String,
}

fn item(given: &str) -> Item {
    if let Some(mime) = vergence_types::of_link(given) {
        return Item { given: given.into(), arg: given.into(), mime };
    }
    let path = vergence_types::path_of_link(given).unwrap_or_else(|| PathBuf::from(given));
    let path = std::path::absolute(&path).unwrap_or(path);
    let mime = vergence_types::of_file(&path).to_string();
    Item { given: given.into(), arg: path.to_string_lossy().into_owned(), mime }
}

fn desktop_id(id: &str) -> String {
    if id.ends_with(".desktop") { id.to_string() } else { format!("{id}.desktop") }
}

fn usage() -> ExitCode {
    eprintln!("usage: vergence-open [--with APP] FILE|LINK...\n       vergence-open --type|--apps FILE|LINK...\n       vergence-open --default TYPE APP");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(first) = args.first() else { return usage() };
    match first.as_str() {
        "--type" => {
            for given in &args[1..] {
                let i = item(given);
                println!("{}\t{}\t{}", i.given, i.mime, vergence_types::describe(&i.mime));
            }
            ExitCode::SUCCESS
        }
        "--default" => {
            let [_, mime, app] = &args[..] else { return usage() };
            match vergence_types::choose_default(mime, &desktop_id(app)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("vergence-open: {}: {e}", vergence_types::own_list_path().display());
                    ExitCode::FAILURE
                }
            }
        }
        "--apps" | "--with" => {
            let installed = apps::installed();
            let ids: Vec<String> = installed.iter().map(|e| desktop_id(&e.id)).collect();
            let known: Vec<vergence_types::App> = installed.iter().zip(&ids).map(|(e, id)| vergence_types::App { id, types: &e.mime_types }).collect();
            let lists = vergence_types::lists();
            if first == "--apps" {
                for given in &args[1..] {
                    let i = item(given);
                    let found = vergence_types::apps_for(&i.mime, &known, &lists);
                    println!("{} ({}): {}", i.given, i.mime, if found.is_empty() { "none".into() } else { found.join(", ") });
                }
                return ExitCode::SUCCESS;
            }
            let Some(app) = args.get(1) else { return usage() };
            let Some(entry) = installed.iter().find(|e| desktop_id(&e.id) == desktop_id(app)) else {
                eprintln!("vergence-open: no application {app}");
                return ExitCode::FAILURE;
            };
            let files: Vec<String> = args[2..].iter().map(|g| item(g).arg).collect();
            if start(entry, &files) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        _ if first.starts_with("--") => usage(),
        _ => {
            let installed = apps::installed();
            let ids: Vec<String> = installed.iter().map(|e| desktop_id(&e.id)).collect();
            let known: Vec<vergence_types::App> = installed.iter().zip(&ids).map(|(e, id)| vergence_types::App { id, types: &e.mime_types }).collect();
            let lists = vergence_types::lists();
            // Files of one type go to one application together, in the order given.
            let mut groups: Vec<(String, Vec<String>)> = Vec::new();
            let mut ok = true;
            for given in &args {
                let i = item(given);
                match vergence_types::default_for(&i.mime, &known, &lists) {
                    Some(app) => match groups.iter_mut().find(|(a, _)| *a == app) {
                        Some((_, files)) => files.push(i.arg),
                        None => groups.push((app, vec![i.arg])),
                    },
                    None => {
                        eprintln!("vergence-open: {}: no application opens {} ({})", i.given, vergence_types::describe(&i.mime), i.mime);
                        ok = false;
                    }
                }
            }
            for (app, files) in groups {
                let Some(entry) = installed.iter().find(|e| desktop_id(&e.id) == app) else { continue };
                ok &= start(entry, &files);
            }
            if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
    }
}

/// Starts an application with files, as its Exec line says; not waited for.
fn start(entry: &desktop::Entry, files: &[String]) -> bool {
    let mut ok = true;
    for argv in desktop::command_for(entry, files) {
        let Some((program, rest)) = argv.split_first() else { continue };
        let mut command = if entry.terminal {
            let mut c = std::process::Command::new("vergence-term");
            c.arg("-e").args(&argv);
            c
        } else {
            let mut c = std::process::Command::new(program);
            c.args(rest);
            c
        };
        command.stdin(std::process::Stdio::null());
        if let Err(e) = command.spawn() {
            eprintln!("vergence-open: {}: {e}", entry.name);
            ok = false;
        }
    }
    ok
}
