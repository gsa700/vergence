//! What the machine is doing (tools milestone 1, the Hardware app): its
//! processors and their load, frequency and temperatures, its memory, its
//! file systems and how busy its disks are, its name and system. Our own
//! code, read the way each system offers it: Linux from /proc and /sys,
//! FreeBSD from sysctls (sysctlbyname, from the C library every program
//! links); how full each file system is from `df -kP`, which both systems
//! have. The parsers are plain functions of the text, tested here against
//! samples of it.

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
            for i in 1..=32 {
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
}

pub use os::{cpu, disk_io, memory, temps};

/// The effective user of process `pid`, if it can be read (the terminal's
/// red tab and the server's mark on a superuser's windows): Linux's
/// /proc/<pid>/status (its Uid line: real, effective, saved, filesystem);
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
    fn this_machine_reads_something() {
        let c = cpu();
        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            assert!(!c.ticks.is_empty(), "processors");
            assert!(memory().total > 0, "memory");
        }
    }
}
