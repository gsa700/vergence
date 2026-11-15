//! `vergence-choose` (libexec): the Open and Save dialog for apps that are
//! not ours (files milestone 1, step 3). The server starts it for each call
//! to the desktop portal's FileChooser, places its window over the calling
//! app's window as a modal dialog, and reads the answer from its output:
//! one chosen path per line, then exit 0; nothing and exit 1 if cancelled.
//!
//!     vergence-choose --open|--save|--folder [--title T] [--accept WORD]
//!         [--multiple] [--name NAME] [--in DIR] [--app APP]
//!         [--filter "NAME|glob:*.txt|mime:text/plain"]... [--current N]
//!
//! `--app` is the calling app (its last folder is remembered apart from
//! ours); filters are a name and its patterns and types, `|` between them.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;

use vergence_widgets::Theme;
use vergence_widgets::filedialog::{FileDialog, Filter};

/// desktop.toml's text, for the theme.
fn desktop_text() -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(config.join("vergence/desktop.toml")).unwrap_or_default()
}

/// A filter from its argument: "Text files|glob:*.txt|mime:text/plain".
fn filter(arg: &str) -> Option<Filter> {
    let mut parts = arg.split('|');
    let name = parts.next()?.trim();
    let (mut globs, mut types) = (Vec::new(), Vec::new());
    for p in parts {
        if let Some(g) = p.strip_prefix("glob:") {
            globs.push(g.to_string());
        } else if let Some(t) = p.strip_prefix("mime:") {
            types.push(t.to_string());
        }
    }
    let mut f = Filter::new(if name.is_empty() { "Files" } else { name }, &[]);
    f.patterns = globs;
    f.types = types;
    Some(f)
}

fn usage() -> ExitCode {
    eprintln!("usage: vergence-choose --open|--save|--folder [--title T] [--accept WORD] [--multiple] [--name NAME] [--in DIR] [--app APP] [--filter F]... [--current N]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut mode, mut title, mut accept, mut multiple, mut name, mut dir, mut app, mut filters, mut current) = (None, None, None, false, String::new(), None, String::from("portal"), Vec::new(), 0usize);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--open" | "--save" | "--folder" => mode = Some(a.clone()),
            "--multiple" => multiple = true,
            "--title" => title = it.next().cloned(),
            "--accept" => accept = it.next().cloned(),
            "--name" => name = it.next().cloned().unwrap_or_default(),
            "--in" => dir = it.next().map(PathBuf::from),
            "--app" => app = it.next().cloned().unwrap_or_default(),
            "--filter" => filters.extend(it.next().and_then(|f| filter(f))),
            "--current" => current = it.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            _ => return usage(),
        }
    }
    let Some(mode) = mode else { return usage() };
    let theme = Theme::from_desktop(&desktop_text(), false, "[file dialog]");
    let chosen: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let out = chosen.clone();
    let done = move |files: Vec<PathBuf>| *out.borrow_mut() = files;
    // Remembered folders per calling app, apart from our own apps'.
    let id = format!("portal-{}", app.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).take(64).collect::<String>());
    let mut d = match mode.as_str() {
        "--save" => FileDialog::save(&id, title.as_deref().unwrap_or("Save"), &name, theme, done),
        "--folder" => FileDialog::choose_folder(&id, title.as_deref().unwrap_or("Choose a folder"), theme, done),
        _ => FileDialog::open(&id, title.as_deref().unwrap_or("Open"), theme, done).multiple(multiple),
    };
    if let Some(w) = &accept {
        d = d.accept(w);
    }
    if let Some(dir) = &dir {
        d = d.folder(dir);
    }
    if !filters.is_empty() {
        // The app's filters, the one it asked for first, then "All files" last.
        let mut list = filters;
        let first = list.remove(current.min(list.len() - 1));
        list.insert(0, first);
        list.push(Filter::new("All files", &[]));
        d = d.filters(list);
    }
    if let Err(e) = vergence_widgets::run(d, (860, 540)) {
        eprintln!("vergence-choose: {e}");
        return ExitCode::FAILURE;
    }
    let files = chosen.borrow();
    if files.is_empty() {
        return ExitCode::from(1);
    }
    for f in files.iter() {
        println!("{}", f.display());
    }
    ExitCode::SUCCESS
}
