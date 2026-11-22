// The drive list (drives milestone 1; Dave, 2026-11-21: the desktop's icons
// and FileView disagreed about a USB stick, two processes each reading the
// mounts on its own; "2": the server keeps one list). A thread looks at the
// mount table and the automounter's /media (vergence-media drives.rs) every
// two seconds, and at once when poked (after an action); the list goes to
// the subscribers of the bus topic `Drives` whenever it changes. Eject,
// Mount and Mount Disk Image are done here on a worker, so one "Safe to
// remove" comes from the server whichever app asked (a disk image put away
// says nothing). `vctl drives` prints the list.

use std::sync::mpsc;
use std::time::Duration;

use smithay::reexports::calloop::channel::{self, Event};
use vergence_protocol::bus::{DriveAction, DriveInfo, DriveKind, ErrorCode, ServerMsg, Topic};

use crate::Server;

/// How often the drives are looked at when nothing pokes.
const EVERY: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct Drives {
    pub list: Vec<DriveInfo>,
    /// Pokes the watching thread to look again now.
    poke: Option<mpsc::Sender<()>>,
    /// Sends a finished action's answer back to the event loop.
    done: Option<channel::Sender<Heard>>,
}

/// What the threads tell the event loop.
pub enum Heard {
    List(Vec<DriveInfo>),
    /// An action done: the client and its serial, and where it is mounted (or why not);
    /// the drive's name and kind, for the notice.
    Done { client: u32, serial: u32, result: Result<String, String>, action: DriveAction, name: String, kind: Option<DriveKind> },
}

impl Server {
    /// Starts watching the drives (any session but the login screen's).
    pub fn drives_start(&mut self) {
        if crate::greeter() {
            return;
        }
        let (tx, rx) = channel::channel::<Heard>();
        let res = self.loop_handle.insert_source(rx, |event, _, state| {
            if let Event::Msg(heard) = event {
                state.drives_heard(heard);
            }
        });
        if let Err(e) = res {
            return tracing::warn!("drives: {e}");
        }
        let (poke, poked) = mpsc::channel::<()>();
        let list_tx = tx.clone();
        let started = std::thread::Builder::new().name("drives".into()).spawn(move || {
            let mut last: Vec<DriveInfo> = Vec::new();
            let mut first = true;
            loop {
                // A drive ejected and since pulled out is forgotten (plugged in again, it shows).
                vergence_media::drives::prune_ejected();
                // A drive pulled out while mounted is let go of (FreeBSD keeps its mount).
                for stale in vergence_media::drives::stale_mounts() {
                    match vergence_media::eject(&stale) {
                        Ok(_) => tracing::info!("drives: {} was pulled out while mounted; unmounted", stale.display()),
                        Err(e) => tracing::warn!("drives: {} was pulled out while mounted and could not be unmounted: {e}", stale.display()),
                    }
                }
                let now = vergence_media::drives::scan();
                if now != last || first {
                    first = false;
                    last = now.clone();
                    if list_tx.send(Heard::List(now)).is_err() {
                        return;
                    }
                }
                match poked.recv_timeout(EVERY) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        if let Err(e) = started {
            return tracing::warn!("drives: can't start watching: {e}");
        }
        // FreeBSD: devd tells when a disk's node comes or goes; looked at at
        // once (Dave, 2026-11-21: "a pretty long delay when inserting a drive").
        #[cfg(target_os = "freebsd")]
        {
            let p = poke.clone();
            let _ = std::thread::Builder::new().name("drives devd".into()).spawn(move || devd_disks(p));
        }
        self.drives.poke = Some(poke);
        self.drives.done = Some(tx);
    }

    fn drives_heard(&mut self, heard: Heard) {
        match heard {
            Heard::List(list) => {
                tracing::info!("drives: {}", list.iter().map(|d| format!("{} ({:?}{})", d.path, d.kind, if d.mounted { "" } else { ", not mounted" })).collect::<Vec<_>>().join(", "));
                self.drives.list = list;
                let m = ServerMsg::Drives { serial: 0, drives: self.drives.list.clone() };
                for id in self.bus_subscribers(Topic::Drives) {
                    self.bus_send(id, m.clone());
                }
            }
            Heard::Done { client, serial, result, action, name, kind } => {
                match &result {
                    Ok(path) => {
                        self.bus_send(client, ServerMsg::DriveDone { serial, path: path.clone() });
                        // One notice, from here (Dave: "yes" to the server's one "Safe to
                        // remove"); a disk image or a network mount let go of says nothing.
                        if action == DriveAction::Eject && matches!(kind, Some(DriveKind::Removable | DriveKind::Optical)) {
                            let req = crate::notify::Request {
                                app: String::new(),
                                replaces: 0,
                                summary: format!("Safe to remove “{name}”"),
                                body: "The drive can be pulled out now.".into(),
                                urgency: 1,
                                timeout_ms: 0,
                                actions: Vec::new(),
                            };
                            self.notify(crate::notify::Origin::Server, req);
                        }
                    }
                    Err(why) => self.bus_send(client, ServerMsg::Error { serial, code: ErrorCode::Failed, message: why.clone() }),
                }
                if let Some(p) = &self.drives.poke {
                    let _ = p.send(());
                }
            }
        }
    }

    /// The topic's state for a new subscriber.
    pub fn bus_drives(&self) -> ServerMsg {
        ServerMsg::Drives { serial: 0, drives: self.drives.list.clone() }
    }

    /// A client's drive action: done on a worker, answered when it is.
    pub fn drive_action(&mut self, client: u32, serial: u32, action: DriveAction, path: String) {
        let Some(done) = self.drives.done.clone() else {
            return self.bus_send(client, ServerMsg::Error { serial, code: ErrorCode::Failed, message: "the drives are not watched in this session".into() });
        };
        let known = self.drives.list.iter().find(|d| d.path == path).cloned();
        // Only drives the list has are ejected or mounted; an image is any file
        // (vergence-media checks it, and the FreeBSD helper again as root).
        if action != DriveAction::MountImage && known.is_none() {
            return self.bus_send(client, ServerMsg::Error { serial, code: ErrorCode::Failed, message: format!("{path} is not a drive this session knows") });
        }
        let name = known.as_ref().map_or_else(|| std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), |d| d.name.clone());
        let kind = known.as_ref().map(|d| d.kind);
        let started = std::thread::Builder::new().name("drive action".into()).spawn(move || {
            let p = std::path::Path::new(&path);
            let result = match action {
                DriveAction::Eject => eject(p).map(|()| String::new()),
                // Looked into, the automounter mounts it.
                DriveAction::Mount => std::fs::read_dir(p).map(|_| path.clone()).map_err(|e| e.to_string()),
                DriveAction::MountImage => vergence_media::mount_image(p).map(|at| at.to_string_lossy().into_owned()),
            };
            let _ = done.send(Heard::Done { client, serial, result, action, name, kind });
        });
        if let Err(e) = started {
            tracing::warn!("drives: can't start the action: {e}");
        }
    }
}

/// FreeBSD: reads devd's pipe (as drm.rs does for monitors; each reader has
/// every event) and pokes the drive thread when a disk's or a label's node
/// is made or taken away.
#[cfg(target_os = "freebsd")]
fn devd_disks(poke: mpsc::Sender<()>) {
    use std::io::BufRead;
    loop {
        if let Ok(stream) = std::os::unix::net::UnixStream::connect("/var/run/devd.pipe") {
            for line in std::io::BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if is_disk_node(&line) && poke.send(()).is_err() {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// devd's "!system=DEVFS subsystem=CDEV type=CREATE cdev=da0s1" and the like,
/// for disks, cards, discs, md devices and the labels the automounter uses.
#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn is_disk_node(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('!') else { return false };
    if !rest.contains("system=DEVFS") || !rest.contains("subsystem=CDEV") {
        return false;
    }
    rest.split_whitespace().filter_map(|w| w.strip_prefix("cdev=")).any(|c| {
        let base: String = c.chars().take_while(|x| x.is_ascii_alphabetic()).collect();
        matches!(base.as_str(), "da" | "cd" | "mmcsd" | "sdda" | "md") || ["msdosfs/", "iso9660/", "label/", "ext2fs/", "ntfs/", "exfat/", "ufs/"].iter().any(|d| c.starts_with(d))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn devd_disk_nodes_are_told_apart() {
        assert!(super::is_disk_node("!system=DEVFS subsystem=CDEV type=CREATE cdev=da0s1"));
        assert!(super::is_disk_node("!system=DEVFS subsystem=CDEV type=DESTROY cdev=msdosfs/USB"));
        assert!(!super::is_disk_node("!system=DEVFS subsystem=CDEV type=CREATE cdev=ttyU0"));
        assert!(!super::is_disk_node("!system=DRM subsystem=CONNECTOR type=HOTPLUG"));
    }
}

/// Lets go of a drive: through vergence-media (udisks2, the FreeBSD helper),
/// else `umount` (a network mount); a drive under /media noted as ejected,
/// so the list leaves it out while it stays plugged in.
fn eject(path: &std::path::Path) -> Result<(), String> {
    if vergence_media::eject(path)? {
        if path.starts_with("/media/") {
            vergence_media::drives::note_ejected(path);
        }
        return Ok(());
    }
    let kind = vergence_media::drives::mounts().into_iter().find(|m| m.at == path).map(|m| m.kind).unwrap_or_default();
    let mut tries: Vec<(&str, Vec<&str>)> = Vec::new();
    if kind.starts_with("fuse.") && cfg!(target_os = "linux") {
        tries.push(("fusermount3", vec!["-u"]));
        tries.push(("fusermount", vec!["-u"]));
    }
    // By its full path where there is one: the session's PATH may not have /sbin.
    tries.push((if std::path::Path::new("/sbin/umount").exists() { "/sbin/umount" } else { "umount" }, vec![]));
    let mut why = String::new();
    for (program, args) in tries {
        match std::process::Command::new(program).args(&args).arg(path).stdin(std::process::Stdio::null()).output() {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => why = String::from_utf8_lossy(&o.stderr).trim().to_string(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => why = e.to_string(),
        }
    }
    Err(if why.is_empty() { "it could not be unmounted.".into() } else { why })
}
