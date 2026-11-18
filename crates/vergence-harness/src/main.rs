//! vergence-harness: test tool, separate from the daemon (milestone 1, test
//! automation). The harness may need root; vergd never does.
//!
//!   vergence-harness parse <edid-db>
//!       Parse every EDID in a linuxhw/EDID checkout (github.com/linuxhw/EDID)
//!       and report what kinds of monitors and faults it holds. No X needed.
//!
//!   vergence-harness live <edid-db> --conn HDMI-A-1 --output HDMI-1
//!                         [--sample N] [--vergd PATH]
//!       Root, X display in DISPLAY/XAUTHORITY. For N EDIDs spread evenly over
//!       the database: write it to the connector's debugfs edid_override,
//!       force the connector on, make X re-probe, and check that X shows the
//!       same bytes and that `vergd probe` prints the expected profile key.
//!       Then unplug the fake again.
//!
//!   vergence-harness fake --name N --product 0x9902 --serial 2
//!                         --mode 4k30|1080p60 --size-mm WxH --out FILE
//!       Write a fake monitor's EDID (milestone 4: fakes of other densities).
//!
//!   vergence-harness measure <window title>...
//!   vergence-harness seams <shot.png> --scale S --window X Y W H [--bar H]
//!   vergence-harness same <a.png> <b.png> [--at X Y] [--corners R] [--top N] [--tolerance T]
//!   vergence-harness cursors <theme directory>   (Vergence's pointers as an Xcursor theme)
//!       Find each size box (tools/sizebox) and print its size on the panel
//!       in millimeters (milestone 4, acceptance 3).

mod cursors;
mod db;
mod fake;
mod live;
mod measure;
mod parse;
mod same;
mod seams;

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: vergence-harness parse <edid-db>\n       \
         vergence-harness live <edid-db> --conn <debugfs connector> --output <X output> \
         [--sample N] [--vergd PATH]\n       \
         vergence-harness fake --name N --product 0xNNNN --serial N --mode 4k30|1080p60 \
         --size-mm WxH --out FILE\n       \
         vergence-harness measure <window title>...\n       \
         vergence-harness seams <shot.png> --scale S --window X Y W H [--bar H]\n       \
         vergence-harness same <a.png> <b.png> [--at X Y] [--corners R] [--top N] [--tolerance T]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("parse") if args.len() == 2 => parse::run(&PathBuf::from(&args[1])),
        Some("live") if args.len() >= 2 => match live::Options::from_args(&args[1..]) {
            Some(o) => live::run(&o),
            None => return usage(),
        },
        Some("fake") => fake::run(&args[1..]),
        Some("measure") if args.len() >= 2 => measure::run(&args[1..]),
        Some("seams") if args.len() >= 2 => seams::run(&args[1..]),
        Some("same") if args.len() >= 3 => same::run(&args[1..]),
        Some("cursors") if args.len() == 2 => cursors::run(&args[1..]),
        // `grid <scale>`: exit 0 if the scale has the server's pixel grid (step and all).
        Some("grid") if args.len() == 2 => match args[1].parse::<f64>().ok().and_then(seams::grid_step) {
            Some(q) => {
                println!("grid step {q} points");
                Ok(true)
            }
            None => {
                println!("off the grid");
                Ok(false)
            }
        },
        _ => return usage(),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("vergence-harness: {e}");
            ExitCode::FAILURE
        }
    }
}
