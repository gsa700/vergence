// vdm-try: ask vdm to check this account's password from a terminal, as the
// lock screen will; for trying vdm by hand. Each of PAM's prompts is shown and
// answered (without echo where PAM says so: stty -echo).
//
//   vdm-try [--socket PATH]

use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;

use vergence_dm::paths;
use vergence_dm::proto::{self, FromVdm, Service, ToVdm};
use vergence_dm::secret::Secret;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let socket = args.iter().position(|a| a == "--socket").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| paths::UNLOCK_SOCKET.to_string());
    let mut s = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vdm-try: {socket}: {e} (is vdm running?)");
            std::process::exit(2);
        }
    };
    if proto::send_to_vdm(&mut s, &ToVdm::Hello { version: proto::VERSION, service: Service::Unlock }).is_err() {
        eprintln!("vdm-try: vdm closed the connection");
        std::process::exit(2);
    }
    let stdin = std::io::stdin();
    loop {
        match proto::read_from_vdm(&mut s) {
            Ok(FromVdm::Prompt { echo, text }) => {
                print!("{text}");
                let _ = std::io::stdout().flush();
                if !echo {
                    let _ = std::process::Command::new("stty").arg("-echo").status();
                }
                let mut line = String::new();
                let _ = stdin.lock().read_line(&mut line);
                if !echo {
                    let _ = std::process::Command::new("stty").arg("echo").status();
                    println!();
                }
                let mut answer = Secret::new().expect("memory for the answer");
                answer.push_str(line.trim_end_matches(['\n', '\r']));
                line.clear();
                if proto::send_to_vdm(&mut s, &ToVdm::Answer(answer)).is_err() {
                    eprintln!("vdm-try: vdm closed the connection");
                    std::process::exit(2);
                }
            }
            Ok(FromVdm::Info(t)) => println!("{t}"),
            Ok(FromVdm::Error(t)) => println!("! {t}"),
            Ok(FromVdm::Done { ok, wait }) => {
                if ok {
                    println!("yes");
                    std::process::exit(0);
                }
                if wait > 0 {
                    println!("no (wait {wait} s)");
                } else {
                    println!("no");
                }
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("vdm-try: {e}");
                std::process::exit(2);
            }
        }
    }
}
