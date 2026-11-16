// vtile: put up a live tile on the Vergence dock from a script (dock
// milestone 1). A tile lasts as long as the connection that put it up, so
// vtile stays running: end it (Ctrl+C, or the end of its input) and the
// tile goes.
//
//   vtile show <id> [options]   put the tile up and keep it
//   vtile run <id> [options]    the same, then each line read is a change,
//                               in the same options ("--value 24 m --state warning")
//
// Options:
//   --value V  --label L        what the dock shows (short: the dock is 48 points)
//   --state fine|warning|problem
//   --group G                   the drawer's group (System, Lab, Background apps)
//   --page P                    a settings page a click opens; without, the click is printed
//   --title T  --big B  --note N
//   --row Name=Value            a row of the card (repeat; given again, replaces them all)
//   --chart 1,2,3               the card's chart, oldest first
//   --push V                    add one value to the chart (the last 60 are kept)
//   --button id=Label  --switch id=Label=on|off  --slider id=Label=value,min,max,step
//                               the card's controls (given again, replace them all)
//
// What happens to the tile is printed, one line each: "card-opened",
// "card-closed", "clicked", "action <control> <value>".
//
// Finds the bus from $VERGENCE_BUS, else $WAYLAND_DISPLAY in $XDG_RUNTIME_DIR.

use std::io::{BufRead, Write};


use vergence_protocol::{
    Client,
    bus::{ClientMsg, ServerMsg},
    sock::poll_readable,
    tile::{Control, ControlKind, Tile, TileState},
};

/// How many chart values `--push` keeps.
const KEEP: usize = 60;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("vtile: {e}");
        std::process::exit(1);
    }
}

fn usage() -> String {
    "usage: vtile show|run <id> [--value V] [--label L] [--state fine|warning|problem] [--row Name=Value]... (see the top of vtile.rs)".into()
}

fn run(args: &[String]) -> Result<(), String> {
    let (Some(mode), Some(id)) = (args.first(), args.get(1)) else { return Err(usage()) };
    let lines = match mode.as_str() {
        "show" => false,
        "run" => true,
        _ => return Err(usage()),
    };
    let mut tile = Tile { id: id.clone(), ..Tile::default() };
    apply(&mut tile, &args[2..])?;
    let mut c = Client::connect("vtile").map_err(|e| format!("no Vergence session bus: {e}"))?;
    publish(&mut c, &tile)?;
    // Lines are read on a thread of their own (stdin is buffered, so poll
    // cannot say when the next line is already read) and taken here between
    // waits on the bus.
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    if lines {
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(l) = line else { break };
                if tx.send(Some(l)).is_err() {
                    break;
                }
            }
            let _ = tx.send(None);
        });
    }
    let mut out = std::io::stdout();
    loop {
        while let Some(m) = c.try_event().map_err(|e| e.to_string())? {
            event(&mut out, m);
        }
        let ready = poll_readable(&[Some(c.raw_fd())], Some(std::time::Duration::from_millis(100))).map_err(|e| e.to_string())?;
        if ready[0] && !c.read_ready().map_err(|e| e.to_string())? {
            return Err("the session ended".into());
        }
        while let Ok(line) = rx.try_recv() {
            // The end of the input ends the tile.
            let Some(line) = line else { return Ok(()) };
            let words = match split(line.trim()) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("vtile: {e}: {}", line.trim());
                    continue;
                }
            };
            if words.is_empty() {
                continue;
            }
            match apply(&mut tile, &words) {
                Ok(()) => publish(&mut c, &tile)?,
                Err(e) => eprintln!("vtile: {e}"),
            }
        }
    }
}

fn publish(c: &mut Client, tile: &Tile) -> Result<(), String> {
    match c.request(|serial| ClientMsg::SetTile { serial, tile: tile.clone() }).map_err(|e| e.to_string())? {
        ServerMsg::Done { .. } => Ok(()),
        ServerMsg::Error { message, .. } => Err(format!("refused: {message}")),
        other => Err(format!("unexpected {other:?}")),
    }
}

fn event(out: &mut impl Write, m: ServerMsg) {
    let _ = match m {
        ServerMsg::CardOpened { .. } => writeln!(out, "card-opened"),
        ServerMsg::CardClosed { .. } => writeln!(out, "card-closed"),
        ServerMsg::TileClicked { .. } => writeln!(out, "clicked"),
        ServerMsg::TileAction { control, value, .. } => writeln!(out, "action {control} {value}"),
        _ => Ok(()),
    };
    let _ = out.flush();
}

/// Change `tile` as the options say.
fn apply(tile: &mut Tile, args: &[String]) -> Result<(), String> {
    let mut rows: Option<Vec<(String, String)>> = None;
    let mut controls: Option<Vec<Control>> = None;
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args.get(i + 1).ok_or_else(|| format!("{flag} needs a value"))?.clone();
        let pair = |v: &str| v.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())).ok_or_else(|| format!("{flag} wants name=value, not {v:?}"));
        match flag {
            "--value" => tile.value = value,
            "--label" => tile.label = value,
            "--group" => tile.group = value,
            "--page" => tile.page = value,
            "--title" => tile.card.title = value,
            "--big" => tile.card.big = value,
            "--note" => tile.card.note = value,
            "--state" => {
                tile.state = match value.as_str() {
                    "fine" | "ok" => TileState::Fine,
                    "warning" | "warn" => TileState::Warning,
                    "problem" | "bad" => TileState::Problem,
                    other => return Err(format!("no state {other:?} (fine, warning, problem)")),
                }
            }
            "--row" => rows.get_or_insert_with(Vec::new).push(pair(&value)?),
            "--chart" => {
                tile.card.chart = value.split(',').filter(|v| !v.trim().is_empty()).map(|v| v.trim().parse::<f32>().map_err(|_| format!("not a number: {v:?}"))).collect::<Result<_, _>>()?;
            }
            "--push" => {
                tile.card.chart.push(value.trim().parse::<f32>().map_err(|_| format!("not a number: {value:?}"))?);
                let n = tile.card.chart.len();
                if n > KEEP {
                    tile.card.chart.drain(..n - KEEP);
                }
            }
            "--button" => {
                let (id, label) = pair(&value)?;
                controls.get_or_insert_with(Vec::new).push(Control { id, label, kind: ControlKind::Button });
            }
            "--switch" => {
                let (id, rest) = pair(&value)?;
                let (label, on) = rest.rsplit_once('=').ok_or("--switch wants id=Label=on|off")?;
                let on = matches!(on, "on" | "1" | "true");
                controls.get_or_insert_with(Vec::new).push(Control { id, label: label.into(), kind: ControlKind::Switch(on) });
            }
            "--slider" => {
                let (id, rest) = pair(&value)?;
                let (label, nums) = rest.rsplit_once('=').ok_or("--slider wants id=Label=value,min,max,step")?;
                let n: Vec<f64> = nums.split(',').map(|v| v.trim().parse::<f64>().map_err(|_| format!("not a number: {v:?}"))).collect::<Result<_, _>>()?;
                let [value, min, max, step] = n[..] else { return Err("--slider wants four numbers: value,min,max,step".into()) };
                controls.get_or_insert_with(Vec::new).push(Control { id, label: label.into(), kind: ControlKind::Slider { value, min, max, step } });
            }
            other => return Err(format!("no option {other:?}")),
        }
        i += 2;
    }
    if let Some(r) = rows {
        tile.card.rows = r;
    }
    if let Some(k) = controls {
        tile.card.controls = k;
    }
    tile.check()
}

/// A line split into words as a shell would: blanks between words, quotes
/// (single or double) keep blanks in, a backslash keeps the next character.
fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some('"') | None, '\\') => word.get_or_insert_with(String::new).push(chars.next().ok_or("a backslash at the end")?),
            (Some(_), c) => word.get_or_insert_with(String::new).push(c),
            (None, '"' | '\'') => {
                quote = Some(ch);
                word.get_or_insert_with(String::new);
            }
            (None, c) if c.is_whitespace() => words.extend(word.take()),
            (None, c) => word.get_or_insert_with(String::new).push(c),
        }
    }
    if quote.is_some() {
        return Err("an unclosed quote".into());
    }
    words.extend(word);
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_as_a_shell_would() {
        assert_eq!(split(r#"--value "23 m" --row 'Load=27 %' --note a\ b"#).unwrap(), ["--value", "23 m", "--row", "Load=27 %", "--note", "a b"]);
        assert_eq!(split(r#"--label """#).unwrap(), ["--label", ""]);
        assert!(split("--value \"open").is_err());
        assert!(split("  ").unwrap().is_empty());
    }

    #[test]
    fn options_change_the_tile() {
        let mut t = Tile { id: "ups".into(), ..Tile::default() };
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        apply(&mut t, &a(&["--value", "23 m", "--label", "UPS", "--state", "warning", "--row", "Load=27 %", "--row", "On=mains", "--switch", "beep=Beeper=on"])).unwrap();
        assert_eq!((t.value.as_str(), t.state), ("23 m", TileState::Warning));
        assert_eq!(t.card.rows.len(), 2);
        assert_eq!(t.card.controls[0].kind, ControlKind::Switch(true));
        // A later change keeps what it does not name; rows given again replace them.
        apply(&mut t, &a(&["--row", "Load=30 %", "--push", "1"])).unwrap();
        assert_eq!((t.value.as_str(), t.card.rows.len(), t.card.chart.len()), ("23 m", 1, 1));
        assert!(apply(&mut t, &a(&["--state", "awful"])).is_err());
        assert!(apply(&mut t, &a(&["--value"])).is_err());
        for _ in 0..70 {
            apply(&mut t, &a(&["--push", "2"])).unwrap();
        }
        assert_eq!(t.card.chart.len(), KEEP);
    }
}
