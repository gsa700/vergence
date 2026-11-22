//! Vergence's power (the settings app's Battery page; later the dock's
//! battery tile): the batteries and mains power, and whether each GPU is
//! awake.
//!
//! Where the systems differ, and how:
//! - Linux: `/sys/class/power_supply/*` (type Battery, Mains), energy in µWh
//!   or charge in µAh with the voltage in µV, power in µW or current in µA;
//!   the GPUs are the display-class PCI devices in `/sys/bus/pci/devices`,
//!   awake or asleep by their runtime power state.
//! - FreeBSD: ACPI's sysctls `hw.acpi.battery.{life,time,state,rate}` and
//!   `hw.acpi.acline`; no per-GPU sleep (the GPU list is empty there).

pub mod history;

/// What a battery is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    Charging,
    Discharging,
    Full,
    /// Plugged in, neither charging nor running down (held at a limit).
    NotCharging,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Battery {
    pub name: String,
    pub state: State,
    /// 0 to 100.
    pub percent: Option<u8>,
    /// Watts going in (charging) or out (discharging).
    pub watts: Option<f64>,
    /// Watt-hours held now, when full, and when new.
    pub energy_wh: Option<f64>,
    pub full_wh: Option<f64>,
    pub design_wh: Option<f64>,
    /// Charge in milliamp-hours, where the battery speaks in charge.
    pub full_mah: Option<f64>,
    pub design_mah: Option<f64>,
    pub volts: Option<f64>,
    pub cycles: Option<u32>,
    pub model: String,
    pub technology: String,
    /// Minutes until empty (discharging) or full (charging), as the system
    /// reports or as worked out from the energy and the power.
    pub minutes: Option<u32>,
}

impl Battery {
    /// How much it holds now against when new, 0 to 100.
    pub fn health(&self) -> Option<u8> {
        // The battery's own units first (charge-based ones' watt-hours depend on the voltage now).
        let (now, new) = match (self.full_mah, self.design_mah, self.full_wh, self.design_wh) {
            (Some(f), Some(d), _, _) => (f, d),
            (_, _, Some(f), Some(d)) => (f, d),
            _ => return None,
        };
        (new > 0.0).then(|| (now / new * 100.0).round().clamp(0.0, 200.0) as u8)
    }
}

/// The machine's power: on mains or not, and its batteries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Power {
    pub on_mains: Option<bool>,
    pub batteries: Vec<Battery>,
}

impl Power {
    /// Running on the battery now: the mains say so, or (where nothing
    /// reports the mains) a battery is running down.
    pub fn on_battery(&self) -> bool {
        match self.on_mains {
            Some(m) => !m,
            None => self.batteries.iter().any(|b| b.state == State::Discharging),
        }
    }
}

/// A GPU and whether it is awake.
#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    /// Its PCI slot ("0000:01:00.0").
    pub slot: String,
    /// "Intel", "NVIDIA", "AMD", or the vendor's number.
    pub vendor: String,
    /// The kernel driver bound to it ("i915", "nouveau").
    pub driver: String,
    /// Awake (true), asleep (false), or the system does not say.
    pub awake: Option<bool>,
}

pub fn vendor_name(id: u32) -> String {
    match id {
        0x8086 => "Intel".into(),
        0x10de => "NVIDIA".into(),
        0x1002 => "AMD".into(),
        _ => format!("{id:04x}"),
    }
}

/// Minutes to empty or to full from watt-hours and watts (Linux; FreeBSD reports them).
#[cfg(target_os = "linux")]
fn minutes_from(state: State, energy: Option<f64>, full: Option<f64>, watts: Option<f64>) -> Option<u32> {
    let w = watts.filter(|w| *w > 0.1)?;
    let wh = match state {
        State::Discharging => energy?,
        State::Charging => (full? - energy?).max(0.0),
        _ => return None,
    };
    Some((wh / w * 60.0).round() as u32)
}

#[cfg(target_os = "linux")]
mod sys {
    use super::*;
    use std::path::Path;

    fn read(dir: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|s| s.trim().chars().filter(|c| !c.is_control()).take(64).collect())
    }

    fn num(dir: &Path, name: &str) -> Option<f64> {
        read(dir, name)?.parse::<f64>().ok()
    }

    /// Read the power supplies under `root` (`/sys/class/power_supply`).
    pub fn power_in(root: &Path) -> Power {
        let mut p = Power::default();
        let Ok(entries) = std::fs::read_dir(root) else { return p };
        let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            match read(&d, "type").as_deref() {
                Some("Mains") => {
                    let online = read(&d, "online").as_deref() == Some("1");
                    p.on_mains = Some(p.on_mains.unwrap_or(false) || online);
                }
                Some("Battery") if read(&d, "present").as_deref() != Some("0") => {
                    let state = match read(&d, "status").as_deref() {
                        Some("Charging") => State::Charging,
                        Some("Discharging") => State::Discharging,
                        Some("Full") => State::Full,
                        Some("Not charging") => State::NotCharging,
                        _ => State::Unknown,
                    };
                    let volts = num(&d, "voltage_now").map(|v| v / 1e6);
                    // Energy in µWh, or charge in µAh times the voltage.
                    let wh =
                        |e: &str, c: &str| num(&d, e).map(|v| v / 1e6).or_else(|| Some(num(&d, c)? / 1e6 * volts?));
                    let watts = num(&d, "power_now")
                        .map(|v| v / 1e6)
                        .or_else(|| Some(num(&d, "current_now")? / 1e6 * volts?))
                        .map(f64::abs);
                    let energy = wh("energy_now", "charge_now");
                    let full = wh("energy_full", "charge_full");
                    let mut b = Battery {
                        name: d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        state,
                        percent: num(&d, "capacity").map(|v| v.clamp(0.0, 100.0) as u8),
                        watts,
                        energy_wh: energy,
                        full_wh: full,
                        design_wh: wh("energy_full_design", "charge_full_design"),
                        full_mah: num(&d, "charge_full").map(|v| v / 1e3),
                        design_mah: num(&d, "charge_full_design").map(|v| v / 1e3),
                        volts,
                        cycles: num(&d, "cycle_count").map(|v| v as u32).filter(|c| *c > 0),
                        model: [read(&d, "manufacturer"), read(&d, "model_name")]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .trim()
                            .to_string(),
                        technology: read(&d, "technology").unwrap_or_default(),
                        minutes: None,
                    };
                    b.minutes = minutes_from(b.state, b.energy_wh, b.full_wh, b.watts);
                    p.batteries.push(b);
                }
                _ => {}
            }
        }
        p
    }

    pub fn power() -> Power {
        power_in(Path::new("/sys/class/power_supply"))
    }

    /// The display-class PCI devices under `root` (`/sys/bus/pci/devices`).
    pub fn gpus_in(root: &Path) -> Vec<Gpu> {
        let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
        let mut out = Vec::new();
        let mut dirs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            let class =
                read(&d, "class").and_then(|c| u32::from_str_radix(c.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
            if class >> 16 != 0x03 {
                continue;
            }
            let vendor =
                read(&d, "vendor").and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
            let driver = std::fs::read_link(d.join("driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let awake = match read(&d, "power/runtime_status").as_deref() {
                Some("active") | Some("resuming") => Some(true),
                Some("suspended") | Some("suspending") => Some(false),
                _ => None,
            };
            out.push(Gpu {
                slot: d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                vendor: vendor_name(vendor),
                driver,
                awake,
            });
        }
        out
    }

    pub fn gpus() -> Vec<Gpu> {
        gpus_in(Path::new("/sys/bus/pci/devices"))
    }
}

#[cfg(target_os = "freebsd")]
mod sys {
    use super::*;

    unsafe extern "C" {
        fn sysctlbyname(
            name: *const std::ffi::c_char,
            old: *mut u8,
            oldlen: *mut usize,
            new: *const u8,
            newlen: usize,
        ) -> i32;
    }

    fn int(name: &std::ffi::CStr) -> Option<i32> {
        let mut v: i32 = 0;
        let mut len = std::mem::size_of::<i32>();
        // SAFETY: a NUL-terminated name and an int-sized buffer.
        let r = unsafe { sysctlbyname(name.as_ptr(), &mut v as *mut i32 as *mut u8, &mut len, std::ptr::null(), 0) };
        (r == 0).then_some(v)
    }

    pub fn power() -> Power {
        let on_mains = int(c"hw.acpi.acline").map(|v| v == 1);
        let mut p = Power { on_mains, batteries: Vec::new() };
        // No battery: the sysctl is missing, or reports -1.
        let Some(life) = int(c"hw.acpi.battery.life").filter(|l| *l >= 0) else { return p };
        let st = int(c"hw.acpi.battery.state").unwrap_or(0);
        // ACPI_BATT_STAT_DISCHARG 1, ACPI_BATT_STAT_CHARGING 2.
        let state = if st & 1 != 0 {
            State::Discharging
        } else if st & 2 != 0 {
            State::Charging
        } else if life >= 100 {
            State::Full
        } else if on_mains == Some(true) {
            State::NotCharging
        } else {
            State::Unknown
        };
        p.batteries.push(Battery {
            name: "battery".into(),
            state,
            percent: Some(life.clamp(0, 100) as u8),
            // hw.acpi.battery.rate is in mW where present.
            watts: int(c"hw.acpi.battery.rate").filter(|r| *r > 0).map(|r| r as f64 / 1000.0),
            minutes: int(c"hw.acpi.battery.time").filter(|t| *t >= 0).map(|t| t as u32),
            ..Default::default()
        });
        p
    }

    pub fn gpus() -> Vec<Gpu> {
        Vec::new()
    }
}

/// The machine's power now.
pub fn power() -> Power {
    sys::power()
}

/// Its GPUs and whether each is awake (Linux; empty on FreeBSD).
pub fn gpus() -> Vec<Gpu> {
    sys::gpus()
}

/// "3 h 20 min", "45 min".
pub fn duration_text(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fake_sys(files: &[(&str, &str)]) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "vergence-power-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        for (path, text) in files {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, format!("{text}\n")).unwrap();
        }
        root
    }

    #[test]
    fn a_battery_that_speaks_in_charge() {
        // Hubble's ASUS A32-K55, discharging at 1.2 A.
        let root = fake_sys(&[
            ("ACAD/type", "Mains"),
            ("ACAD/online", "0"),
            ("BAT1/type", "Battery"),
            ("BAT1/status", "Discharging"),
            ("BAT1/capacity", "80"),
            ("BAT1/charge_now", "2192800"),
            ("BAT1/charge_full", "2741000"),
            ("BAT1/charge_full_design", "4400000"),
            ("BAT1/current_now", "1200000"),
            ("BAT1/voltage_now", "12000000"),
            ("BAT1/manufacturer", "ASUS"),
            ("BAT1/model_name", "A32-K55"),
            ("BAT1/technology", "Li-ion"),
        ]);
        let p = sys::power_in(&root);
        assert_eq!(p.on_mains, Some(false));
        let b = &p.batteries[0];
        assert_eq!((b.state, b.percent), (State::Discharging, Some(80)));
        assert!((b.watts.unwrap() - 14.4).abs() < 1e-6, "{:?}", b.watts);
        assert!((b.energy_wh.unwrap() - 26.3136).abs() < 1e-3);
        assert_eq!(b.minutes, Some(110), "26.3 Wh at 14.4 W");
        assert_eq!(b.health(), Some(62), "2741 of 4400 mAh");
        assert_eq!(b.model, "ASUS A32-K55");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_battery_that_speaks_in_energy_charging() {
        let root = fake_sys(&[
            ("AC/type", "Mains"),
            ("AC/online", "1"),
            ("BAT0/type", "Battery"),
            ("BAT0/status", "Charging"),
            ("BAT0/capacity", "50"),
            ("BAT0/energy_now", "25000000"),
            ("BAT0/energy_full", "50000000"),
            ("BAT0/energy_full_design", "57000000"),
            ("BAT0/power_now", "20000000"),
            ("BAT0/cycle_count", "312"),
            ("hidpp_battery_0/type", "Battery"),
            ("hidpp_battery_0/present", "0"),
        ]);
        let p = sys::power_in(&root);
        assert_eq!(p.on_mains, Some(true));
        assert_eq!(p.batteries.len(), 1, "a battery that is not there is left out");
        let b = &p.batteries[0];
        assert_eq!(b.minutes, Some(75), "25 Wh to go at 20 W");
        assert_eq!(b.health(), Some(88));
        assert_eq!(b.cycles, Some(312));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn gpus_awake_and_asleep() {
        let root = fake_sys(&[
            ("0000:00:02.0/class", "0x030000"),
            ("0000:00:02.0/vendor", "0x8086"),
            ("0000:00:02.0/power/runtime_status", "active"),
            ("0000:01:00.0/class", "0x030000"),
            ("0000:01:00.0/vendor", "0x10de"),
            ("0000:01:00.0/power/runtime_status", "suspended"),
            ("0000:00:1f.3/class", "0x040300"),
            ("0000:00:1f.3/vendor", "0x8086"),
        ]);
        let g = sys::gpus_in(&root);
        assert_eq!(g.len(), 2, "the sound card is no GPU");
        assert_eq!((g[0].vendor.as_str(), g[0].awake), ("Intel", Some(true)));
        assert_eq!((g[1].vendor.as_str(), g[1].awake), ("NVIDIA", Some(false)));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn durations() {
        assert_eq!(duration_text(45), "45 min");
        assert_eq!(duration_text(120), "2 h");
        assert_eq!(duration_text(200), "3 h 20 min");
    }
}
