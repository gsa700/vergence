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

mod db;
mod live;
mod parse;

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: vergence-harness parse <edid-db>\n       \
         vergence-harness live <edid-db> --conn <debugfs connector> --output <X output> \
         [--sample N] [--vergd PATH]"
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
