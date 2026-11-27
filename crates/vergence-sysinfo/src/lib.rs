//! What the machine is doing: processors, memory, file systems, disks, temperatures, its name and
//! system.
//!
//! Tools milestone 1, the Hardware app. Used by the Hardware app (vergence-hw), the server and
//! the terminal. Our own code, no dependencies, read the way each system offers it: Linux from
//! /proc and /sys, FreeBSD from sysctls (sysctlbyname, from the C library every program links);
//! how full each file system is from `df -kP`, which both systems have.
//!
//! Start reading: the types at the top ([`Cpu`], [`Memory`], [`FileSystem`], [`DiskIo`]), then
//! the `os` module for your system.
//!
//! Rule: the parsers are plain functions of the text, tested here against samples of it; keep
//! reading and parsing apart so they stay testable.

use std::path::Path;

/// One processor's time so far: busy and in all (in the system's ticks).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ticks {
    pub busy: u64,
    pub total: u64,
}

/// How busy each processor was between two readings, 0 to 1.
pub fn load(before: &[Ticks], now: &[Ticks]) -> Vec<f32> {
    before
        .iter()
        .zip(now)
        .map(|(a, b)| {
            let total = b.total.saturating_sub(a.total);
            if total == 0 { 0.0 } else { (b.busy.saturating_sub(a.busy) as f32 / total as f32).clamp(0.0, 1.0) }
        })
        .collect()
}

/// The processors: what they are, and each one's ticks now.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cpu {
    pub model: String,
    pub ticks: Vec<Ticks>,
    /// The average clock now, in MHz, where the system says.
    pub mhz: Option<u32>,
}

/// A temperature the machine reports: what it is of, and degrees Celsius.
#[derive(Debug, Clone, PartialEq)]
pub struct Temp {
    pub what: String,
    pub celsius: f32,
}

/// Memory in bytes: all of it, what programs can still have (free, and
/// caches the system gives back), what is cache, and swap.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

/// A mounted file system: where, of what, how large and how much is free.
#[derive(Debug, Clone, PartialEq)]
pub struct FileSystem {
    pub mount: String,
    pub source: String,
    pub total: u64,
    pub free: u64,
}

/// A disk's bytes read and written so far.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskIo {
    pub name: String,
    pub read: u64,
    pub written: u64,
}

/// What kind of core a processor is on a chip with two kinds (Intel's
/// hybrid parts: Performance and Efficiency cores); `Plain` where the chip
/// has one kind, or the system does not say which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoreKind {
    Performance,
    Efficiency,
    Plain,
}

/// Where one logical processor sits: its number, the processors that share
/// its core (itself among them: two with hyper-threading, one without), the
/// processors that share its cluster (the core's level-2 cache: an
/// Efficiency cluster of four, or just the core) and its kind.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub cpu: usize,
    pub siblings: Vec<usize>,
    pub cluster: Vec<usize>,
    pub kind: CoreKind,
    /// The core's number as the system gives it (Linux topology/core_id):
    /// what coretemp names its sensor by ("Core 32"). None on FreeBSD.
    pub core_id: Option<u32>,
}

/// One core: its logical processors (threads), lowest first, and its
/// number as the system gives it, where it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Core {
    pub threads: Vec<usize>,
    pub id: Option<u32>,
}

/// The cores that share a level-2 cache.
#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub cores: Vec<Core>,
}

/// All the cores of one kind, cluster by cluster.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub kind: CoreKind,
    pub clusters: Vec<Cluster>,
}

/// The chip's shape: Performance cores first, then Efficiency, then plain.
/// Empty where the system says nothing; the Hardware app then draws the
/// processors in a plain row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Topology {
    pub sections: Vec<Section>,
}

impl Topology {
    /// Every logical processor in it.
    pub fn threads(&self) -> usize {
        self.sections.iter().flat_map(|s| &s.clusters).flat_map(|c| &c.cores).map(|c| c.threads.len()).sum()
    }

    pub fn cores(&self) -> usize {
        self.sections.iter().flat_map(|s| &s.clusters).map(|c| c.cores.len()).sum()
    }
}

/// The shape from each processor's place: sections by kind, clusters in
/// the order of their lowest processor, cores likewise. A processor named
/// in a sibling or cluster list but missing from `places` is left out.
pub fn arrange(places: &[Place]) -> Topology {
    let mut places: Vec<&Place> = places.iter().collect();
    places.sort_by_key(|p| (p.kind, p.cpu));
    let known = |v: &[usize]| -> Vec<usize> {
        let mut v: Vec<usize> = v.iter().copied().filter(|c| places.iter().any(|p| p.cpu == *c)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut done: Vec<usize> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    for p in &places {
        if done.contains(&p.cpu) {
            continue;
        }
        // The cluster: the processors of this kind listed as sharing it.
        let mut members: Vec<&Place> = known(&p.cluster)
            .into_iter()
            .chain(std::iter::once(p.cpu))
            .filter_map(|c| places.iter().copied().find(|q| q.cpu == c && q.kind == p.kind))
            .filter(|q| !done.contains(&q.cpu))
            .collect();
        members.sort_by_key(|q| q.cpu);
        members.dedup_by_key(|q| q.cpu);
        let mut cores: Vec<Core> = Vec::new();
        for q in &members {
            if done.contains(&q.cpu) {
                continue;
            }
            let mut threads: Vec<usize> =
                known(&q.siblings).into_iter().filter(|c| members.iter().any(|m| m.cpu == *c)).collect();
            if !threads.contains(&q.cpu) {
                threads.push(q.cpu);
                threads.sort_unstable();
            }
            threads.retain(|c| !done.contains(c));
            done.extend(&threads);
            cores.push(Core { threads, id: q.core_id });
        }
        match sections.last_mut() {
            Some(s) if s.kind == p.kind => s.clusters.push(Cluster { cores }),
            _ => sections.push(Section { kind: p.kind, clusters: vec![Cluster { cores }] }),
        }
    }
    Topology { sections }
}

// ---- parsers (plain text in, numbers out) ----

/// Linux /proc/stat: one Ticks per "cpuN" line (not the "cpu" total).
pub fn parse_proc_stat(text: &str) -> Vec<Ticks> {
    text.lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .map(|l| {
            let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|n| n.parse().ok()).collect();
            // user nice system idle iowait irq softirq steal …
            let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
            let total: u64 = v.iter().take(8).sum();
            Ticks { busy: total.saturating_sub(idle), total }
        })
        .collect()
}

/// Linux /proc/cpuinfo: the first "model name".
pub fn parse_cpu_model(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string()))
        .unwrap_or_default()
}

/// Linux /proc/meminfo.
pub fn parse_meminfo(text: &str) -> Memory {
    let kb = |key: &str| {
        text.lines()
            .find_map(|l| {
                l.strip_prefix(key)
                    .and_then(|r| r.strip_prefix(':'))
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .unwrap_or(0)
            * 1024
    };
    Memory {
        total: kb("MemTotal"),
        available: kb("MemAvailable"),
        cached: kb("Cached"),
        swap_total: kb("SwapTotal"),
        swap_free: kb("SwapFree"),
    }
}

/// File systems that are a place on a disk (not the kernel's own).
fn real_fs(source: &str, mount: &str) -> bool {
    const SKIP: [&str; 12] = [
        "tmpfs",
        "devtmpfs",
        "devfs",
        "fdescfs",
        "procfs",
        "linprocfs",
        "linsysfs",
        "overlay",
        "efivarfs",
        "run",
        "none",
        "udev",
    ];
    !SKIP.contains(&source)
        && !mount.starts_with("/proc")
        && !mount.starts_with("/sys")
        && !mount.starts_with("/dev")
        && !mount.starts_with("/run")
}

/// `df -kP`: Filesystem, 1024-blocks, Used, Available, Capacity, Mounted on.
pub fn parse_df(text: &str) -> Vec<FileSystem> {
    // A volume mounted in several places (btrfs subvolumes: / and /home on
    // one device) is listed once, at its first mount.
    let mut seen: Vec<String> = Vec::new();
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 6 {
                return None;
            }
            let (total, free) = (f[1].parse::<u64>().ok()?, f[3].parse::<u64>().ok()?);
            // The mount point may hold spaces: everything after the fifth field.
            let mount = f[5..].join(" ");
            if !real_fs(f[0], &mount) || total == 0 || seen.iter().any(|s| s == f[0]) {
                return None;
            }
            seen.push(f[0].to_string());
            Some(FileSystem { mount, source: f[0].to_string(), total: total * 1024, free: free * 1024 })
        })
        .collect()
}

/// Linux /proc/diskstats: whole disks (not partitions, loops or RAM
/// disks), sectors of 512 bytes read and written.
pub fn parse_diskstats(text: &str) -> Vec<DiskIo> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let name = *f.get(2)?;
            let skip = name.starts_with("loop")
                || name.starts_with("ram")
                || name.starts_with("zram")
                || name.starts_with("dm-")
                || name.starts_with("sr");
            // A partition: sda1, nvme0n1p1, mmcblk0p1.
            let partition = (name.starts_with("sd") || name.starts_with("vd") || name.starts_with("hd"))
                && name.ends_with(|c: char| c.is_ascii_digit())
                || name.contains('p')
                    && (name.starts_with("nvme") || name.starts_with("mmcblk"))
                    && name.rsplit('p').next().is_some_and(|t| t.chars().all(|c| c.is_ascii_digit()));
            if skip || partition {
                return None;
            }
            let read = f.get(5)?.parse::<u64>().ok()? * 512;
            let written = f.get(9)?.parse::<u64>().ok()? * 512;
            Some(DiskIo { name: name.to_string(), read, written })
        })
        .collect()
}

/// A kernel CPU list ("0-3,8,10-11", Linux's sysfs; FreeBSD's "0, 1")
/// as numbers.
pub fn parse_cpu_list(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    out.extend(a..=b.min(a + 4096));
                }
            }
            None => out.extend(part.parse::<usize>().ok()),
        }
    }
    out
}

/// FreeBSD's kern.sched.topology_spec (XML: nested groups, each with its
/// processors and flags): the places, with a THREAD or SMT group as a core
/// and the smallest group sharing a level-2 cache as the cluster. FreeBSD
/// does not say which cores are Performance and which Efficiency, so every
/// place is `Plain` there (said so in the app, not guessed).
pub fn parse_topology_spec(xml: &str) -> Vec<Place> {
    struct Group {
        cache: Option<u32>,
        cpus: Vec<usize>,
        smt: bool,
    }
    let mut groups: Vec<Group> = Vec::new();
    // The groups still open, as indices into `groups`.
    let mut open: Vec<usize> = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        let after = &rest[(end + 1).min(rest.len())..];
        if let Some(attrs) = tag.strip_prefix("group") {
            let cache = attrs
                .split_once("cache-level=\"")
                .and_then(|(_, v)| v.split('"').next())
                .and_then(|v| v.parse::<u32>().ok());
            groups.push(Group { cache, cpus: Vec::new(), smt: false });
            open.push(groups.len() - 1);
        } else if tag == "/group" {
            open.pop();
        } else if tag.starts_with("cpu ") || tag == "cpu" {
            if let Some(&g) = open.last() {
                groups[g].cpus = parse_cpu_list(&after[..after.find('<').unwrap_or(after.len())]);
            }
        } else if tag.starts_with("flag ")
            && let Some(&g) = open.last()
        {
            groups[g].smt |= tag.contains("\"THREAD\"") || tag.contains("\"SMT\"");
        }
        rest = after;
    }
    let mut all: Vec<usize> = groups.iter().flat_map(|g| g.cpus.iter().copied()).collect();
    all.sort_unstable();
    all.dedup();
    let smallest = |cpu: usize, pick: &dyn Fn(&Group) -> bool| {
        groups.iter().filter(|g| pick(g) && g.cpus.contains(&cpu)).min_by_key(|g| g.cpus.len()).map(|g| g.cpus.clone())
    };
    all.iter()
        .map(|&cpu| {
            let siblings = smallest(cpu, &|g| g.smt).unwrap_or_else(|| vec![cpu]);
            let cluster = smallest(cpu, &|g| g.cache == Some(2)).unwrap_or_else(|| siblings.clone());
            Place { cpu, siblings, cluster, kind: CoreKind::Plain, core_id: None }
        })
        .collect()
}

/// The cores and threads in words, each kind marked where it runs two
/// threads on a core (Dave's wording, 2026-11-26): "8 Performance
/// (Hyper-Threading) + 16 Efficiency, 32 Threads"; one kind: "4 Cores
/// (Hyper-Threading), 8 Threads", "6 Cores, 6 Threads". Intel's name for
/// it is Hyper-Threading, AMD's SMT (from the processor's `model`).
pub fn cores_text(t: &Topology, model: &str) -> String {
    let ht = ht_word(model);
    let parts: Vec<String> =
        [(CoreKind::Performance, "Performance"), (CoreKind::Efficiency, "Efficiency"), (CoreKind::Plain, "Cores")]
            .into_iter()
            .filter_map(|(k, word)| {
                let cores: Vec<&Core> = t
                    .sections
                    .iter()
                    .filter(|s| s.kind == k)
                    .flat_map(|s| &s.clusters)
                    .flat_map(|c| &c.cores)
                    .collect();
                if cores.is_empty() {
                    return None;
                }
                let mark = if cores.iter().any(|c| c.threads.len() > 1) { format!(" ({ht})") } else { String::new() };
                Some(format!("{} {word}{mark}", cores.len()))
            })
            .collect();
    format!("{}, {} Threads", parts.join(" + "), t.threads())
}

/// Two threads on a core, by its maker's name.
pub fn ht_word(model: &str) -> &'static str {
    if model.contains("AMD") { "SMT" } else { "Hyper-Threading" }
}

/// What the machine is, from its firmware's tables (SMBIOS/DMI): maker and
/// model, the board, the firmware's version. Any user may read these.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Machine {
    pub maker: String,
    pub product: String,
    /// A model's friendlier name where the maker puts it there (Lenovo:
    /// "ThinkCentre M910q", the product being a type number).
    pub version: String,
    pub board_maker: String,
    pub board: String,
    pub bios: String,
}

impl Machine {
    /// The machine in a few words: its maker and model, or, where the
    /// maker left the firmware's placeholder (a board bought on its own:
    /// "System Product Name", "To be filled by O.E.M."), its board:
    /// "ASUS TUF GAMING Z690-PLUS WIFI".
    pub fn name(&self) -> String {
        let blank = |v: &str| {
            let l = v.trim().to_ascii_lowercase();
            l.is_empty()
                || ["system product name", "to be filled", "default string", "not applicable", "none", "o.e.m", "oem"]
                    .iter()
                    .any(|p| l.contains(p))
        };
        let maker = if blank(&self.maker) { &self.board_maker } else { &self.maker };
        let model = if self.maker.eq_ignore_ascii_case("lenovo") && !blank(&self.version) {
            &self.version
        } else if !blank(&self.product) {
            &self.product
        } else {
            &self.board
        };
        let text = format!("{} {}", maker.trim(), model.trim());
        text.trim().to_string()
    }
}

/// /etc/os-release's PRETTY_NAME (or NAME).
pub fn parse_os_release(text: &str) -> String {
    let get = |k: &str| {
        text.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=').map(|v| v.trim().trim_matches('"').to_string()))
    };
    get("PRETTY_NAME").or_else(|| get("NAME")).unwrap_or_default()
}

/// A time since in words, to the two largest units: "3 days, 4 hours",
/// "2 hours, 5 minutes", "1 minute".
pub fn uptime_text(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    let unit = |n: u64, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
    match (d, h) {
        (0, 0) => unit(m.max(if secs < 60 { 0 } else { 1 }), "minute"),
        (0, _) => format!("{}, {}", unit(h, "hour"), unit(m, "minute")),
        _ => format!("{}, {}", unit(d, "day"), unit(h, "hour")),
    }
}

/// Bytes in words: "7.6 GB", "512 MB".
pub fn bytes_text(b: u64) -> String {
    let b = b as f64;
    if b >= 1e12 {
        format!("{:.1} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} kB", b / 1e3)
    } else {
        format!("{b:.0} B")
    }
}

// ---- reading, per system ----

// Only Linux reads files for these (/proc, /sys); FreeBSD asks sysctl.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The file systems, from `df -kP`.
pub fn file_systems() -> Vec<FileSystem> {
    let out = std::process::Command::new("df").arg("-kP").output();
    out.ok().map(|o| parse_df(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// The machine's name and its system ("Linux 6.18.63", "FreeBSD 15.1-RELEASE-p6").
pub fn system() -> (String, String) {
    let out = |args: &[&str]| {
        std::process::Command::new("uname")
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    (out(&["-n"]), out(&["-sr"]))
}

#[cfg(target_os = "linux")]
mod os {
    use super::*;

    pub fn cpu() -> Cpu {
        let ticks = parse_proc_stat(&read("/proc/stat"));
        let freqs: Vec<u32> = (0..ticks.len())
            .filter_map(|i| {
                read(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")).trim().parse::<u32>().ok()
            })
            .collect();
        let mhz = (!freqs.is_empty()).then(|| freqs.iter().sum::<u32>() / freqs.len() as u32 / 1000);
        Cpu { model: parse_cpu_model(&read("/proc/cpuinfo")), ticks, mhz }
    }

    pub fn memory() -> Memory {
        parse_meminfo(&read("/proc/meminfo"))
    }

    pub fn machine() -> Machine {
        let id = |f: &str| read(format!("/sys/class/dmi/id/{f}")).trim().to_string();
        Machine {
            maker: id("sys_vendor"),
            product: id("product_name"),
            version: id("product_version"),
            board_maker: id("board_vendor"),
            board: id("board_name"),
            bios: id("bios_version"),
        }
    }

    /// "Void Linux"; the kernel is uname's.
    pub fn os_name() -> String {
        let n = parse_os_release(&read("/etc/os-release"));
        if n.is_empty() { "Linux".into() } else { n }
    }

    pub fn uptime() -> Option<u64> {
        read("/proc/uptime").split_whitespace().next()?.parse::<f64>().ok().map(|s| s as u64)
    }

    /// Each online processor's place, from sysfs (any user may read it):
    /// topology/thread_siblings_list for the core, topology/cluster_cpus_list
    /// (or the level-2 cache's shared_cpu_list on older kernels) for the
    /// cluster, and on a hybrid chip /sys/devices/cpu_core/cpus and
    /// cpu_atom/cpus for the kind.
    pub fn topology() -> Topology {
        let base = "/sys/devices/system/cpu";
        let perf = parse_cpu_list(read("/sys/devices/cpu_core/cpus").trim());
        let eff = parse_cpu_list(read("/sys/devices/cpu_atom/cpus").trim());
        let places: Vec<Place> = parse_cpu_list(read(format!("{base}/online")).trim())
            .into_iter()
            .filter_map(|cpu| {
                let d = format!("{base}/cpu{cpu}");
                let siblings = parse_cpu_list(read(format!("{d}/topology/thread_siblings_list")).trim());
                if siblings.is_empty() {
                    return None;
                }
                let mut cluster = parse_cpu_list(read(format!("{d}/topology/cluster_cpus_list")).trim());
                if cluster.is_empty() {
                    cluster = parse_cpu_list(read(format!("{d}/cache/index2/shared_cpu_list")).trim());
                }
                let kind = if perf.contains(&cpu) {
                    CoreKind::Performance
                } else if eff.contains(&cpu) {
                    CoreKind::Efficiency
                } else {
                    CoreKind::Plain
                };
                let core_id = read(format!("{d}/topology/core_id")).trim().parse().ok();
                Some(Place { cpu, siblings, cluster, kind, core_id })
            })
            .collect();
        arrange(&places)
    }

    pub fn disk_io() -> Vec<DiskIo> {
        parse_diskstats(&read("/proc/diskstats"))
    }

    /// Every hwmon temperature: "coretemp Package id 0", "nvme Composite", "nouveau".
    pub fn temps() -> Vec<Temp> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") else { return out };
        let mut mons: Vec<_> = dir.flatten().map(|e| e.path()).collect();
        mons.sort();
        for m in mons {
            let name = read(m.join("name")).trim().to_string();
            // Every tempN_input there, not a fixed range: coretemp numbers
            // its sensors by core id plus two, so a 13900K's Efficiency
            // cores are temp34 to temp49 (a 1..=32 range lost all sixteen).
            let mut nums: Vec<u32> = std::fs::read_dir(&m)
                .map(|d| {
                    d.flatten()
                        .filter_map(|e| {
                            e.file_name().to_str()?.strip_prefix("temp")?.strip_suffix("_input")?.parse().ok()
                        })
                        .collect()
                })
                .unwrap_or_default();
            nums.sort_unstable();
            for i in nums {
                let Ok(milli) = read(m.join(format!("temp{i}_input"))).trim().parse::<i64>() else { continue };
                let label = read(m.join(format!("temp{i}_label"))).trim().to_string();
                let what = if label.is_empty() { name.clone() } else { format!("{name} {label}") };
                out.push(Temp { what, celsius: milli as f32 / 1000.0 });
            }
        }
        out
    }
}

#[cfg(target_os = "freebsd")]
mod os {
    use super::*;
    use std::ffi::CString;

    unsafe extern "C" {
        fn sysctlbyname(
            name: *const std::ffi::c_char,
            oldp: *mut std::ffi::c_void,
            oldlenp: *mut usize,
            newp: *const std::ffi::c_void,
            newlen: usize,
        ) -> std::ffi::c_int;
    }

    /// A sysctl's bytes.
    fn sysctl(name: &str) -> Option<Vec<u8>> {
        let c = CString::new(name).ok()?;
        let mut len = 0usize;
        // SAFETY: a length query (no buffer), then a read into a buffer of that length.
        unsafe {
            if sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null(), 0) != 0 || len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            if sysctlbyname(c.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null(), 0) != 0 {
                return None;
            }
            buf.truncate(len);
            Some(buf)
        }
    }

    fn sysctl_int(name: &str) -> Option<i64> {
        let b = sysctl(name)?;
        Some(match b.len() {
            4 => i32::from_ne_bytes(b[..4].try_into().ok()?) as i64,
            8 => i64::from_ne_bytes(b[..8].try_into().ok()?),
            _ => return None,
        })
    }

    fn sysctl_text(name: &str) -> String {
        sysctl(name).map(|b| String::from_utf8_lossy(&b).trim_end_matches('\0').trim().to_string()).unwrap_or_default()
    }

    pub fn cpu() -> Cpu {
        // kern.cp_times: five longs a processor: user, nice, system, interrupt, idle.
        let ticks = sysctl("kern.cp_times")
            .map(|b| {
                let v: Vec<u64> = b.chunks_exact(8).map(|c| i64::from_ne_bytes(c.try_into().unwrap()) as u64).collect();
                v.chunks_exact(5).map(|c| Ticks { busy: c[0] + c[1] + c[2] + c[3], total: c.iter().sum() }).collect()
            })
            .unwrap_or_default();
        Cpu { model: sysctl_text("hw.model"), ticks, mhz: sysctl_int("dev.cpu.0.freq").map(|m| m as u32) }
    }

    pub fn memory() -> Memory {
        let page = sysctl_int("hw.pagesize").unwrap_or(4096) as u64;
        let pages = |n: &str| sysctl_int(n).unwrap_or(0) as u64 * page;
        let free = pages("vm.stats.vm.v_free_count")
            + pages("vm.stats.vm.v_inactive_count")
            + pages("vm.stats.vm.v_laundry_count");
        let total = sysctl_int("hw.physmem").unwrap_or(0) as u64;
        Memory {
            total,
            available: free.min(total),
            cached: pages("vm.stats.vm.v_inactive_count"),
            swap_total: 0,
            swap_free: 0,
        }
    }

    pub fn disk_io() -> Vec<DiskIo> {
        Vec::new()
    }

    /// kenv's smbios.* (the loader's copy of the firmware's tables; any
    /// user may read them).
    pub fn machine() -> Machine {
        let kenv = |k: &str| {
            std::process::Command::new("kenv")
                .args(["-q", k])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        Machine {
            maker: kenv("smbios.system.maker"),
            product: kenv("smbios.system.product"),
            version: kenv("smbios.system.version"),
            board_maker: kenv("smbios.planar.maker"),
            board: kenv("smbios.planar.product"),
            bios: kenv("smbios.bios.version"),
        }
    }

    pub fn os_name() -> String {
        "FreeBSD".into()
    }

    /// Since kern.boottime (a timeval: seconds first).
    pub fn uptime() -> Option<u64> {
        let b = sysctl("kern.boottime")?;
        let boot = i64::from_ne_bytes(b.get(..8)?.try_into().ok()?);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        Some((now - boot).max(0) as u64)
    }

    /// The scheduler's view of the chip (kern.sched.topology_spec).
    pub fn topology() -> Topology {
        arrange(&parse_topology_spec(&sysctl_text("kern.sched.topology_spec")))
    }

    /// Each processor's temperature (dev.cpu.N.temperature, in tenths of a kelvin).
    pub fn temps() -> Vec<Temp> {
        (0..256)
            .map_while(|i| sysctl(&format!("dev.cpu.{i}.temperature")).map(|b| (i, b)))
            .filter_map(|(i, b)| {
                let dk = i32::from_ne_bytes(b.get(..4)?.try_into().ok()?);
                Some(Temp { what: format!("CPU {i}"), celsius: (dk - 2731) as f32 / 10.0 })
            })
            .collect()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod os {
    use super::*;
    pub fn cpu() -> Cpu {
        Cpu::default()
    }
    pub fn memory() -> Memory {
        Memory::default()
    }
    pub fn disk_io() -> Vec<DiskIo> {
        Vec::new()
    }
    pub fn temps() -> Vec<Temp> {
        Vec::new()
    }
    pub fn topology() -> Topology {
        Topology::default()
    }
    pub fn machine() -> Machine {
        Machine::default()
    }
    pub fn os_name() -> String {
        String::new()
    }
    pub fn uptime() -> Option<u64> {
        None
    }
}

pub use os::{cpu, disk_io, machine, memory, os_name, temps, topology, uptime};

/// The effective user of process `pid`, if it can be read (the terminal's
/// red tab and the server's mark on a superuser's windows): Linux's
/// `/proc/<pid>/status` (its Uid line: real, effective, saved, filesystem);
/// elsewhere (FreeBSD) `ps -o uid=`, which prints the effective user.
pub fn euid_of(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|ids| ids.split_whitespace().nth(1)?.parse().ok())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "uid=", "-p", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The red tab's question, answered on each platform: this test's own
    /// process is the user running it; a process that does not exist, none.
    #[test]
    fn the_effective_user_of_a_process_is_read() {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        assert_eq!(euid_of(std::process::id()), Some(unsafe { geteuid() }));
        assert_eq!(euid_of(999_999_999), None);
    }

    #[test]
    fn processors_and_their_load() {
        let a = parse_proc_stat(
            "cpu  10 0 10 80 0 0 0 0 0 0\ncpu0 5 0 5 40 0 0 0 0 0 0\ncpu1 5 0 5 40 0 0 0 0\nintr 1 2 3\n",
        );
        assert_eq!(a, vec![Ticks { busy: 10, total: 50 }, Ticks { busy: 10, total: 50 }]);
        let b = parse_proc_stat("cpu0 15 0 5 40 0 0 0 0 0 0\ncpu1 5 0 5 50 0 0 0 0\n");
        assert_eq!(load(&a, &b), vec![1.0, 0.0]);
        assert_eq!(
            parse_cpu_model("processor\t: 0\nmodel name\t: Intel(R) Core(TM) i5-10300H CPU @ 2.50GHz\n"),
            "Intel(R) Core(TM) i5-10300H CPU @ 2.50GHz"
        );
    }

    #[test]
    fn memory_from_meminfo() {
        let m = parse_meminfo(
            "MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:   12000000 kB\nCached:  3000000 kB\nSwapTotal: 2000000 kB\nSwapFree: 1500000 kB\n",
        );
        assert_eq!(m.total, 16_000_000 * 1024);
        assert_eq!(m.used(), 4_000_000 * 1024);
        assert_eq!(m.swap_free, 1_500_000 * 1024);
    }

    #[test]
    fn file_systems_from_df_without_the_kernels_own() {
        let df = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/nvme0n1p2   471000000  97000000 350000000      22% /\ntmpfs              8000000      1000   7999000       1% /tmp\ndevtmpfs           10240         0     10240       0% /dev\n/dev/nvme0n1p1     1046508     150000    896508      15% /boot/efi\nzroot/home      400000000  10000000 390000000       3% /home\n/dev/sdb1          1000000     10     999990       1% /media/My Disk\n/dev/nvme0n1p2   471000000  97000000 350000000      22% /var\n";
        let fs = parse_df(df);
        let mounts: Vec<&str> = fs.iter().map(|f| f.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/boot/efi", "/home", "/media/My Disk"]);
        assert_eq!(fs[0].free, 350_000_000 * 1024);
    }

    #[test]
    fn whole_disks_from_diskstats() {
        let t = "   8       0 sda 100 0 2000 0 50 0 4000 0 0 0 0\n   8       1 sda1 10 0 200 0 5 0 400 0 0 0 0\n 259       0 nvme0n1 1 0 10 0 1 0 20 0 0 0 0\n 259       1 nvme0n1p1 1 0 10 0 1 0 20 0 0 0 0\n   7       0 loop0 1 0 10 0 0 0 0 0 0 0 0\n";
        let d = parse_diskstats(t);
        assert_eq!(d.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["sda", "nvme0n1"]);
        assert_eq!(d[0].read, 2000 * 512);
        assert_eq!(d[0].written, 4000 * 512);
    }

    #[test]
    fn bytes_in_words() {
        assert_eq!(bytes_text(7_600_000_000), "7.6 GB");
        assert_eq!(bytes_text(512_000_000), "512 MB");
        assert_eq!(bytes_text(900), "900 B");
    }

    #[test]
    fn cpu_lists_in_both_spellings() {
        assert_eq!(parse_cpu_list("0-3,8,10-11\n".trim()), [0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("0, 1"), [0, 1]);
        assert_eq!(parse_cpu_list(""), Vec::<usize>::new());
    }

    /// techbench's i9-13900K as its sysfs says (read 2026-11-26): eight
    /// Performance cores of two threads (0-15), sixteen Efficiency cores
    /// of one (16-31) in clusters of four.
    #[test]
    fn a_hybrid_chip_keeps_its_shape() {
        let places: Vec<Place> = (0..32)
            .map(|cpu| {
                if cpu < 16 {
                    let s = vec![cpu & !1, cpu | 1];
                    Place {
                        cpu,
                        siblings: s.clone(),
                        cluster: s,
                        kind: CoreKind::Performance,
                        core_id: Some(cpu as u32 / 2 * 4),
                    }
                } else {
                    let c0 = 16 + (cpu - 16) / 4 * 4;
                    Place {
                        cpu,
                        siblings: vec![cpu],
                        cluster: (c0..c0 + 4).collect(),
                        kind: CoreKind::Efficiency,
                        core_id: Some(cpu as u32 + 16),
                    }
                }
            })
            .collect();
        let t = arrange(&places);
        assert_eq!(t.threads(), 32);
        assert_eq!(t.cores(), 24);
        assert_eq!(t.sections.len(), 2);
        let (p, e) = (&t.sections[0], &t.sections[1]);
        assert_eq!(p.kind, CoreKind::Performance);
        assert_eq!(p.clusters.len(), 8);
        assert_eq!(p.clusters[1].cores, [Core { threads: vec![2, 3], id: Some(4) }]);
        assert_eq!(e.kind, CoreKind::Efficiency);
        assert_eq!(e.clusters.len(), 4);
        assert_eq!(e.clusters[1].cores.iter().map(|c| c.threads[0]).collect::<Vec<_>>(), [20, 21, 22, 23]);
        assert_eq!(e.clusters[0].cores[0].id, Some(32));
    }

    /// A plain chip whose threads are numbered core by core apart (Linux on
    /// many Intel parts: 0 and 4 share a core): cores still pair them.
    #[test]
    fn a_plain_chip_pairs_its_threads() {
        let places: Vec<Place> = (0..8)
            .map(|cpu| {
                let s = vec![cpu % 4, cpu % 4 + 4];
                Place { cpu, siblings: s.clone(), cluster: s, kind: CoreKind::Plain, core_id: Some(cpu as u32 % 4) }
            })
            .collect();
        let t = arrange(&places);
        assert_eq!(t.sections.len(), 1);
        assert_eq!(t.sections[0].clusters.len(), 4);
        assert_eq!(t.sections[0].clusters[0].cores, [Core { threads: vec![0, 4], id: Some(0) }]);
        assert_eq!(t.threads(), 8);
    }

    /// FreeBSD's topology_spec in the shape it prints for a chip with SMT
    /// cores and a cluster of four single-thread cores sharing a level-2
    /// cache (written from the format; to check against a hybrid FreeBSD
    /// machine when there is one).
    #[test]
    fn freebsd_topology_spec() {
        let xml = r#"<groups>
 <group level="1" cache-level="3">
  <cpu count="8" mask="ff,0,0,0">0, 1, 2, 3, 4, 5, 6, 7</cpu>
  <children>
   <group level="2" cache-level="2">
    <cpu count="2" mask="3,0,0,0">0, 1</cpu>
    <flags><flag name="THREAD">THREAD group</flag><flag name="SMT">SMT group</flag></flags>
   </group>
   <group level="2" cache-level="2">
    <cpu count="2" mask="c,0,0,0">2, 3</cpu>
    <flags><flag name="THREAD">THREAD group</flag><flag name="SMT">SMT group</flag></flags>
   </group>
   <group level="2" cache-level="2">
    <cpu count="4" mask="f0,0,0,0">4, 5, 6, 7</cpu>
   </group>
  </children>
 </group>
</groups>
"#;
        let places = parse_topology_spec(xml);
        assert_eq!(places.len(), 8);
        assert_eq!(places[1].siblings, [0, 1]);
        assert_eq!(places[5].siblings, [5]);
        assert_eq!(places[5].cluster, [4, 5, 6, 7]);
        let t = arrange(&places);
        assert_eq!(t.cores(), 6);
        assert_eq!(t.sections[0].clusters.len(), 3);
        assert_eq!(t.sections[0].clusters[2].cores.len(), 4);
        // A chip of plain cores with no SMT (kepler's i5-8500T): one group,
        // no flags; six cores of one thread, each its own cluster.
        let plain = parse_topology_spec(
            r#"<groups><group level="1" cache-level="3"><cpu count="6" mask="3f">0, 1, 2, 3, 4, 5</cpu></group></groups>"#,
        );
        assert_eq!(arrange(&plain).cores(), 6);
    }

    #[test]
    fn the_cores_in_daves_words() {
        let core = |t: &[usize]| Core { threads: t.to_vec(), id: None };
        let hybrid = Topology {
            sections: vec![
                Section {
                    kind: CoreKind::Performance,
                    clusters: (0..8).map(|i| Cluster { cores: vec![core(&[2 * i, 2 * i + 1])] }).collect(),
                },
                Section {
                    kind: CoreKind::Efficiency,
                    clusters: (0..4)
                        .map(|c| Cluster { cores: (0..4).map(|i| core(&[16 + 4 * c + i])).collect() })
                        .collect(),
                },
            ],
        };
        assert_eq!(
            cores_text(&hybrid, "13th Gen Intel(R) Core(TM) i9-13900K"),
            "8 Performance (Hyper-Threading) + 16 Efficiency, 32 Threads"
        );
        let plain = |n: usize, smt: bool| Topology {
            sections: vec![Section {
                kind: CoreKind::Plain,
                clusters: (0..n)
                    .map(|i| Cluster { cores: vec![if smt { core(&[i, i + n]) } else { core(&[i]) }] })
                    .collect(),
            }],
        };
        assert_eq!(cores_text(&plain(4, true), "Intel(R) Core(TM) i5-10300H"), "4 Cores (Hyper-Threading), 8 Threads");
        assert_eq!(cores_text(&plain(8, true), "AMD Ryzen 7 5800X"), "8 Cores (SMT), 16 Threads");
        assert_eq!(cores_text(&plain(6, false), "Intel(R) Core(TM) i5-8500T"), "6 Cores, 6 Threads");
    }

    #[test]
    fn machine_names_skip_the_firmwares_placeholders() {
        let m = Machine {
            maker: "ASUS".into(),
            product: "System Product Name".into(),
            board_maker: "ASUSTeK COMPUTER INC.".into(),
            board: "TUF GAMING Z690-PLUS WIFI".into(),
            ..Default::default()
        };
        assert_eq!(m.name(), "ASUS TUF GAMING Z690-PLUS WIFI");
        let lenovo = Machine {
            maker: "LENOVO".into(),
            product: "10MRS0VH00".into(),
            version: "ThinkCentre M910q".into(),
            ..Default::default()
        };
        assert_eq!(lenovo.name(), "LENOVO ThinkCentre M910q");
        let laptop = Machine {
            maker: "ASUSTeK COMPUTER INC.".into(),
            product: "ASUS TUF Gaming F15".into(),
            ..Default::default()
        };
        assert_eq!(laptop.name(), "ASUSTeK COMPUTER INC. ASUS TUF Gaming F15");
    }

    #[test]
    fn os_release_and_uptime_in_words() {
        assert_eq!(parse_os_release("NAME=\"void\"\nPRETTY_NAME=\"Void Linux\"\n"), "Void Linux");
        assert_eq!(parse_os_release("NAME=FreeBSD\n"), "FreeBSD");
        assert_eq!(uptime_text(249_672), "2 days, 21 hours");
        assert_eq!(uptime_text(3600 + 5 * 60), "1 hour, 5 minutes");
        assert_eq!(uptime_text(90), "1 minute");
    }

    #[test]
    fn this_machine_reads_something() {
        let c = cpu();
        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            assert!(!c.ticks.is_empty(), "processors");
            assert!(memory().total > 0, "memory");
            let t = topology();
            assert_eq!(t.threads(), c.ticks.len(), "every processor placed once");
        }
    }
}
