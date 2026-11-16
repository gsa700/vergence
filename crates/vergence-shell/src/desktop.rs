//! Desktop entries: the files by which a system says what applications it
//! has (`*.desktop` under `applications/` in the XDG data directories).
//!
//! A file format read at the edge (whitepaper principle 4) by our own code.
//! The files are the user's or a package's, trusted as much as the programs
//! they start, and still read as untrusted input: a bad one must not take
//! the session down. Sizes are limited, nothing panics, and what cannot be
//! understood is left out, never guessed at.
//!
//! Read from an entry: Type, Name, Exec, Icon, TryExec, Terminal,
//! NoDisplay, Hidden, OnlyShowIn, NotShowIn, StartupWMClass, Keywords,
//! GenericName, Comment, Categories (the last three for the launcher, shelf
//! milestone 2), MimeType (the file types it opens, files milestone 1).
//! Translated names
//! (`Name[de]`) are not used yet.

/// The most of a file that is read, and the longest line and argument list
/// an entry may have.
pub const MAX_FILE: usize = 256 * 1024;
const MAX_VALUE: usize = 4096;
const MAX_ARGS: usize = 256;
/// The most categories and keywords kept from one entry.
pub const MAX_LIST: usize = 32;
/// The most file types kept from one entry (an office suite lists ~150).
pub const MAX_TYPES: usize = 512;

/// This desktop's name, as OnlyShowIn and NotShowIn name desktops (and as
/// the session's XDG_CURRENT_DESKTOP says it).
pub const DESKTOP: &str = "Vergence";

/// One application, as its desktop entry describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    /// The file's name without `.desktop`, subdirectories joined with `-`
    /// ("firefox", "org.gnome.Nautilus"): what pins and window matching use.
    pub id: String,
    pub name: String,
    /// The program and its arguments, field codes removed. Run as it is,
    /// with no shell between.
    pub exec: Vec<String>,
    /// An icon's name in the icon themes, or a file's path.
    pub icon: String,
    /// A program that must exist for the entry to be shown.
    pub try_exec: String,
    /// The program wants a terminal to run in.
    pub terminal: bool,
    /// Not to be shown in a list of applications (it can still own windows).
    pub no_display: bool,
    /// As good as deleted: a user's entry of this kind hides the system's.
    pub hidden: bool,
    /// For other desktops only (OnlyShowIn without Vergence, or NotShowIn
    /// with it): autostart leaves it alone (autostart.rs).
    pub elsewhere: bool,
    /// The class its windows carry, where that is not its id.
    pub startup_wm_class: String,
    pub keywords: Vec<String>,
    /// What kind of program it is ("Web Browser"), a sentence about it, and
    /// the freedesktop categories it declares (the launcher's tabs).
    pub generic_name: String,
    pub comment: String,
    pub categories: Vec<String>,
    /// The file types it says it opens (MimeType), as written.
    pub mime_types: Vec<String>,
    /// The Exec line itself, field codes kept, for opening files with it
    /// ([`command_for`]).
    pub exec_line: String,
}

/// A value as the file means it: `\s`, `\n`, `\t`, `\r` and `\\` are a
/// space, a line end, a tab, a return and a backslash.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // An escape the format does not have is kept as it was written.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// An Exec line as arguments, by the specification's rules: arguments are
/// parted by spaces; one in double quotes may hold spaces, and in it a
/// backslash before `"`, `` ` ``, `$` or `\` gives that character. Field
/// codes (`%f`, `%U`, `%i`, `%c`, `%k` and the rest) are dropped, an
/// argument that was only a field code with them; `%%` is a percent sign.
/// None if a quote is left open, there is no program, or there are more
/// arguments than any real entry has.
pub fn split_exec(exec: &str) -> Option<Vec<String>> {
    split_filled(exec, &[])
}

/// The commands that open `files` (paths or URLs) with an entry, by its
/// Exec line's field codes: `%F` and `%U` take them all in one command, `%f`
/// and `%u` one each, so a program taking one file at a time is started once
/// per file; with no file code the files are left off (the specification's
/// rule) and the program is started once. Files are passed as they are given
/// (a path for %u too, which programs accept). Empty if the line cannot be
/// split.
pub fn command_for(entry: &Entry, files: &[String]) -> Vec<Vec<String>> {
    let line = if entry.exec_line.is_empty() { return vec![entry.exec.clone()] } else { &entry.exec_line };
    let codes = field_codes(line);
    let many = codes.iter().any(|c| matches!(c, 'F' | 'U'));
    let one = codes.iter().any(|c| matches!(c, 'f' | 'u'));
    let runs: Vec<&[String]> = if files.is_empty() || many || !one { vec![files] } else { files.chunks(1).collect() };
    runs.into_iter().filter_map(|f| split_filled(line, f)).collect()
}

/// The field codes an Exec line has (outside `%%`).
fn field_codes(line: &str) -> Vec<char> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') | None => {}
                Some(code) => out.push(code),
            }
        }
    }
    out
}

/// An Exec line split as [`split_exec`] does, with `%f`/`%u` given the first
/// of `files` and `%F`/`%U` all of them, each file its own argument.
fn split_filled(exec: &str, files: &[String]) -> Option<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    let mut chars = exec.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut arg = String::new();
        // An argument that was a field code and nothing else is left out.
        let (mut only_code, mut quoted) = (false, false);
        while let Some(&c) = chars.peek() {
            match c {
                ' ' | '\t' => break,
                '"' => {
                    chars.next();
                    quoted = true;
                    loop {
                        match chars.next()? {
                            '"' => break,
                            '\\' => match chars.next()? {
                                c @ ('"' | '`' | '$' | '\\') => arg.push(c),
                                other => {
                                    arg.push('\\');
                                    arg.push(other);
                                }
                            },
                            c => arg.push(c),
                        }
                    }
                }
                '%' => {
                    chars.next();
                    match chars.next() {
                        Some('%') => arg.push('%'),
                        // A file or files: put in where the code stands.
                        Some('f' | 'u') if !files.is_empty() => {
                            arg.push_str(&files[0]);
                            only_code = false;
                        }
                        Some('F' | 'U') if !files.is_empty() => {
                            // All the files, each its own argument; the code stood alone (else the first file only).
                            if arg.is_empty() && chars.peek().is_none_or(|c| *c == ' ' || *c == '\t') {
                                args.extend(files[..files.len() - 1].iter().cloned());
                                arg.push_str(&files[files.len() - 1]);
                            } else {
                                arg.push_str(&files[0]);
                            }
                            only_code = false;
                        }
                        // A field code: a file, a URL, the icon, the name, the entry's own path.
                        Some(_) => only_code = arg.is_empty(),
                        None => {}
                    }
                }
                c => {
                    chars.next();
                    arg.push(c);
                    only_code = false;
                }
            }
        }
        if !(arg.is_empty() && (only_code || !quoted)) {
            args.push(arg);
        }
        if args.len() > MAX_ARGS {
            return None;
        }
    }
    (!args.is_empty() && !args[0].is_empty()).then_some(args)
}

/// Reads an entry's text. None if it is not an application's entry: no
/// `[Desktop Entry]` group, a Type other than Application, no Name, or an
/// Exec that cannot be split. `id` is given by the caller, from the file's
/// name.
pub fn parse(id: &str, text: &str) -> Option<Entry> {
    let text = if text.len() > MAX_FILE { text.get(..MAX_FILE).or_else(|| text.get(..text.floor_char_boundary(MAX_FILE)))? } else { text };
    let mut entry = Entry { id: id.to_string(), ..Entry::default() };
    let (mut in_group, mut seen_group, mut application, mut exec) = (false, false, false, None);
    let names = |v: &str| v.split(';').any(|d| d.trim() == DESKTOP);
    let (mut only_elsewhere, mut not_here) = (false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // Only the first [Desktop Entry] group counts; actions and the rest are skipped.
            in_group = line == "[Desktop Entry]" && !seen_group;
            seen_group |= in_group;
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim_end(), value.trim_start());
        if value.len() > MAX_VALUE {
            continue;
        }
        let flag = value == "true";
        match key {
            "Type" => application = value == "Application",
            "Name" => entry.name = unescape(value),
            "Exec" => exec = Some(unescape(value)),
            "Icon" => entry.icon = unescape(value),
            "TryExec" => entry.try_exec = unescape(value),
            "Terminal" => entry.terminal = flag,
            "NoDisplay" => entry.no_display = flag,
            "Hidden" => entry.hidden = flag,
            "OnlyShowIn" => only_elsewhere = !names(value),
            "NotShowIn" => not_here = names(value),
            "StartupWMClass" => entry.startup_wm_class = unescape(value),
            "Keywords" => entry.keywords = unescape(value).split(';').map(str::trim).filter(|k| !k.is_empty()).take(MAX_LIST).map(str::to_string).collect(),
            "GenericName" => entry.generic_name = unescape(value),
            "Comment" => entry.comment = unescape(value),
            "MimeType" => entry.mime_types = unescape(value).split(';').map(str::trim).filter(|k| !k.is_empty()).take(MAX_TYPES).map(str::to_string).collect(),
            "Categories" => entry.categories = unescape(value).split(';').map(str::trim).filter(|k| !k.is_empty()).take(MAX_LIST).map(str::to_string).collect(),
            _ => {}
        }
    }
    // An entry for other desktops only is kept but not listed: it may still own windows.
    entry.elsewhere = only_elsewhere || not_here;
    entry.no_display |= entry.elsewhere;
    // A hidden entry is kept (it hides the system's of its id) whatever else it lacks.
    if entry.hidden {
        return seen_group.then_some(entry);
    }
    if !application || entry.name.trim().is_empty() {
        return None;
    }
    let exec = exec?;
    entry.exec = split_exec(&exec)?;
    entry.exec_line = exec;
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(exec: &str) -> Option<Vec<String>> {
        split_exec(exec)
    }

    fn list(items: &[&str]) -> Option<Vec<String>> {
        Some(items.iter().map(|s| s.to_string()).collect())
    }

    /// The launcher's keys (shelf milestone 2): what kind of program, a
    /// comment, its categories; long lists are cut.
    #[test]
    fn generic_name_comment_and_categories_are_read() {
        let many: String = (0..100).map(|i| format!("C{i};")).collect();
        let text = format!("[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\nGenericName=Web Browser\nComment=Browse the\\sWeb\nCategories=Network;WebBrowser;\nKeywords={many}\n");
        let e = parse("firefox", &text).expect("an entry");
        assert_eq!(e.generic_name, "Web Browser");
        assert_eq!(e.comment, "Browse the Web");
        assert_eq!(e.categories, ["Network", "WebBrowser"]);
        assert_eq!(e.keywords.len(), MAX_LIST);
        let text = format!("[Desktop Entry]\nType=Application\nName=X\nExec=x\nCategories={many}\n");
        assert_eq!(parse("x", &text).expect("an entry").categories.len(), MAX_LIST);
    }

    #[test]
    fn exec_lines_become_arguments() {
        assert_eq!(args("firefox %u"), list(&["firefox"]));
        assert_eq!(args("/usr/bin/htop"), list(&["/usr/bin/htop"]));
        assert_eq!(args("  env   FOO=1  prog  --flag  "), list(&["env", "FOO=1", "prog", "--flag"]));
        // Quotes hold spaces; a backslash in them gives the four characters it may.
        assert_eq!(args(r#"sh -c "echo \"hi there\" \$HOME \\ \`x\`""#), list(&["sh", "-c", r#"echo "hi there" $HOME \ `x`"#]));
        assert_eq!(args(r#""/opt/My App/run" --name "A B""#), list(&["/opt/My App/run", "--name", "A B"]));
        // An empty quoted argument is an argument.
        assert_eq!(args(r#"prog "" x"#), list(&["prog", "", "x"]));
        // Field codes go; one inside an argument leaves the rest of it; %% is a percent sign.
        assert_eq!(args("prog --file=%f %U --pct=100%% %i %c %k"), list(&["prog", "--file=", "--pct=100%"]));
        assert_eq!(args("prog %"), list(&["prog"]));
    }

    /// Opening files: %F and %U take them all, %f and %u one per command, a
    /// line with no file code gets none; MimeType and the raw line are kept.
    #[test]
    fn files_are_put_where_the_exec_line_says() {
        let entry = |exec: &str| parse("x", &format!("[Desktop Entry]\nType=Application\nName=X\nExec={exec}\nMimeType=image/png;image/jpeg;\n")).unwrap();
        let files = vec!["/a b.png".to_string(), "/c.png".to_string()];
        let e = entry("view %F");
        assert_eq!(e.mime_types, vec!["image/png", "image/jpeg"]);
        assert_eq!(e.exec_line, "view %F");
        assert_eq!(command_for(&e, &files), vec![vec!["view", "/a b.png", "/c.png"]]);
        assert_eq!(command_for(&entry("view --open %U --new"), &files), vec![vec!["view", "--open", "/a b.png", "/c.png", "--new"]]);
        assert_eq!(command_for(&entry("view %f"), &files), vec![vec!["view", "/a b.png"], vec!["view", "/c.png"]]);
        assert_eq!(command_for(&entry("view --file=%u"), &files[..1]), vec![vec!["view", "--file=/a b.png"]]);
        assert_eq!(command_for(&entry("view"), &files), vec![vec!["view"]]);
        assert_eq!(command_for(&entry("view %F"), &[]), vec![vec!["view"]]);
        assert_eq!(command_for(&entry("pct --at=100%% %f"), &files[..1]), vec![vec!["pct", "--at=100%", "/a b.png"]]);
    }

    #[test]
    fn exec_lines_that_are_not_commands_are_refused() {
        assert_eq!(args(""), None);
        assert_eq!(args("   "), None);
        assert_eq!(args("%u"), None);
        assert_eq!(args(r#"prog "unclosed"#), None);
        assert_eq!(args(r#"prog "ends in a backslash\"#), None);
        assert_eq!(args(r#""" x"#), None, "no program");
        assert_eq!(args(&"a ".repeat(MAX_ARGS + 2)), None);
    }

    const FIREFOX: &str = "\
# a comment
[Desktop Entry]
Version=1.0
Name=Firefox
Name[de]=Feuerfuchs
GenericName=Web Browser
Exec=firefox %u
Icon=firefox
Terminal=false
Type=Application
StartupWMClass=firefox
Keywords=web;browser;internet;
Actions=new-window;

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
";

    #[test]
    fn an_entry_is_read() {
        let e = parse("firefox", FIREFOX).unwrap();
        assert_eq!((e.id.as_str(), e.name.as_str(), e.icon.as_str()), ("firefox", "Firefox", "firefox"));
        assert_eq!(e.exec, ["firefox"], "the entry's own Exec, not its action's");
        assert_eq!(e.keywords, ["web", "browser", "internet"]);
        assert_eq!(e.startup_wm_class, "firefox");
        assert!(!e.terminal && !e.no_display && !e.hidden);
        let htop = parse("htop", "[Desktop Entry]\nType=Application\nName=Htop\nExec=htop\nTerminal=true\nIcon=htop\nNoDisplay=false\n").unwrap();
        assert!(htop.terminal && !htop.no_display);
        // Escapes in values.
        let e = parse("x", "[Desktop Entry]\nType=Application\nName=Two\\sWords\\\\\nExec=prog\n").unwrap();
        assert_eq!(e.name, "Two Words\\");
    }

    #[test]
    fn what_is_not_an_application_is_not_one() {
        assert_eq!(parse("x", ""), None);
        assert_eq!(parse("x", "Name=No group\nExec=x\nType=Application\n"), None);
        assert_eq!(parse("x", "[Desktop Entry]\nType=Link\nName=A link\nURL=http://example.org\n"), None);
        assert_eq!(parse("x", "[Desktop Entry]\nType=Application\nExec=x\n"), None, "no name");
        assert_eq!(parse("x", "[Desktop Entry]\nType=Application\nName=X\n"), None, "nothing to run");
        assert_eq!(parse("x", "[Desktop Entry]\nType=Application\nName=X\nExec=\"open\n"), None);
        // Keys of another group are not the entry's.
        assert_eq!(parse("x", "[Other]\nType=Application\nName=X\nExec=x\n"), None);
        // Another desktop's entries are kept but not listed; ours and everyone's are.
        let app = "[Desktop Entry]\nType=Application\nName=A\nExec=a\n";
        assert!(parse("g", &format!("{app}OnlyShowIn=GNOME;Unity;\n")).unwrap().no_display);
        assert!(!parse("v", &format!("{app}OnlyShowIn=GNOME;Vergence;\n")).unwrap().no_display);
        assert!(parse("n", &format!("{app}NotShowIn=KDE;Vergence;\n")).unwrap().no_display);
        assert!(!parse("k", &format!("{app}NotShowIn=KDE;\n")).unwrap().no_display);
        // Hidden is kept, to hide the system's entry of the same id, with or without the rest.
        let h = parse("x", "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(h.hidden && h.exec.is_empty());
        let n = parse("x", "[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n").unwrap();
        assert!(n.no_display);
    }

    #[test]
    fn oversized_input_is_cut_not_trusted() {
        let mut text = String::from("[Desktop Entry]\nType=Application\nName=Big\nExec=big\n");
        text.push_str(&"# padding é\n".repeat(MAX_FILE / 8));
        assert!(text.len() > MAX_FILE);
        assert_eq!(parse("big", &text).unwrap().name, "Big");
        // A value longer than any real one is skipped, not kept.
        let long = format!("[Desktop Entry]\nType=Application\nName={}\nExec=x\n", "n".repeat(MAX_VALUE + 1));
        assert_eq!(parse("long", &long), None);
    }
}
