//! `vshell apps`: what the shelf will see on this machine, for checking one
//! over SSH (docs/shelf-milestone-1.md, step 2). One application a line:
//! its name, its id, what is run, and the icon file that would be drawn
//! (at 32 points on a screen of scale 1; `--pixels N` asks for another
//! size, `--theme NAME` for an icon theme).

use vergence_shell::{apps, icons};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.first().map(String::as_str) != Some("apps") {
        eprintln!("vshell apps [--theme NAME] [--pixels N] [--all]");
        std::process::exit(2);
    }
    let theme = value("--theme").unwrap_or_default();
    let pixels = value("--pixels").and_then(|v| v.parse().ok()).unwrap_or(32);
    let all = args.iter().any(|a| a == "--all");
    let found = apps::installed();
    for app in found.iter().filter(|a| all || !a.no_display) {
        let icon = match icons::find(&theme, &app.icon, pixels) {
            Some(file) => file.display().to_string(),
            None if app.icon.is_empty() => "(no icon named)".to_string(),
            None => format!("(no file for \"{}\")", app.icon),
        };
        let notes = [app.terminal.then_some("in a terminal"), app.no_display.then_some("not listed")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{}\t{}\t{}\t{}{}",
            app.name,
            app.id,
            app.exec.join(" "),
            icon,
            if notes.is_empty() { String::new() } else { format!("\t({notes})") }
        );
    }
    eprintln!("{} applications ({} not listed)", found.len(), found.iter().filter(|a| a.no_display).count());
}
