//! The machine's network interfaces and their addresses, from getifaddrs(3)
//! on Linux and FreeBSD alike. Where the two differ (the socket address
//! header, the hardware address, how a carrier is told), the code says so.
//!
//! - Linux: a `struct sockaddr` starts with a 16-bit family; the hardware
//!   address comes as an `AF_PACKET` entry (`struct sockaddr_ll`); a carrier
//!   is `IFF_LOWER_UP` in the flags (glibc copies the link's flags there).
//! - FreeBSD: a `struct sockaddr` starts with its length and an 8-bit
//!   family; the hardware address comes as an `AF_LINK` entry (`struct
//!   sockaddr_dl`), whose `ifa_data` is the link's `struct if_data`, and its
//!   `ifi_link_state` says whether there is a carrier.

use std::ffi::{CStr, c_char, c_void};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// An interface: its name ("wlp4s0", "em0"), what it seems to be, its
/// state and addresses.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Iface {
    pub name: String,
    pub kind: Kind,
    /// Turned on (IFF_UP).
    pub up: bool,
    /// A cable in and a link at the other end, or a Wi-Fi network joined.
    pub carrier: bool,
    /// Its hardware address, as "aa:bb:cc:dd:ee:ff", if it has one.
    pub mac: Option<String>,
    /// Its addresses, each with its prefix length.
    pub addrs: Vec<(IpAddr, u8)>,
}

/// What an interface seems to be, by its name: the names both systems give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    Loopback,
    WiFi,
    #[default]
    Wired,
    /// A phone sharing its connection by USB cable (an iPhone's Personal
    /// Hotspot: Linux's `ipheth`, FreeBSD's `ipheth(4)` as a `ue` interface).
    Phone,
}

/// The kind an interface's name and flags make it. Linux names Wi-Fi
/// `wl*` (wlp4s0, wlan0), FreeBSD `wlan*`; loopback is `lo` or `lo0`.
pub fn kind_of(name: &str, loopback: bool) -> Kind {
    if loopback {
        Kind::Loopback
    } else if name.starts_with("wl") {
        Kind::WiFi
    } else {
        Kind::Wired
    }
}

/// Names of links that are no cable: bridges, tunnels, containers' and
/// virtual machines' ends, on Linux and FreeBSD.
const VIRTUAL: [&str; 16] = ["docker", "br-", "virbr", "veth", "tun", "tap", "wg", "tailscale", "zt", "vnet", "vboxnet", "vmnet", "bridge", "epair", "gif", "lxc"];

/// Whether an interface is a network cable with a link at the other end (an
/// Ethernet port, a phone's USB tethering): the rule Wi-Fi's "When no cable
/// is connected" goes by, in the server and the settings app alike.
pub fn is_cable(f: &Iface) -> bool {
    matches!(f.kind, Kind::Wired | Kind::Phone) && f.up && f.carrier && f.mac.is_some() && !is_virtual(&f.name)
}

/// Whether an interface is a virtual one (a bridge, a tunnel, a container's
/// end) by its name: never a cable, and not shown as a connection.
pub fn is_virtual(name: &str) -> bool {
    VIRTUAL.iter().any(|v| name.starts_with(v))
}

/// Whether an interface is an iPhone's USB tethering. Where the systems
/// differ: Linux names the interface's driver in /sys (`ipheth`); FreeBSD's
/// ipheth(4) makes a `ue` interface and a `dev.ipheth` device.
#[cfg(target_os = "linux")]
pub fn is_phone(name: &str) -> bool {
    if name.contains('/') || name.starts_with('.') {
        return false;
    }
    std::fs::read_link(format!("/sys/class/net/{name}/device/driver")).is_ok_and(|p| p.file_name().is_some_and(|d| d == "ipheth"))
}

#[cfg(target_os = "freebsd")]
pub fn is_phone(name: &str) -> bool {
    unsafe extern "C" {
        fn sysctlbyname(name: *const c_char, old: *mut u8, oldlen: *mut usize, new: *const u8, newlen: usize) -> i32;
    }
    if !name.starts_with("ue") {
        return false;
    }
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    // SAFETY: a NUL-terminated name and a buffer of `len` bytes.
    let r = unsafe { sysctlbyname(c"dev.ipheth.0.%driver".as_ptr(), buf.as_mut_ptr(), &mut len, std::ptr::null(), 0) };
    r == 0 && buf[..len.min(buf.len())].starts_with(b"ipheth")
}

#[repr(C)]
struct IfAddrs {
    ifa_next: *mut IfAddrs,
    ifa_name: *const c_char,
    ifa_flags: u32,
    ifa_addr: *const u8,
    ifa_netmask: *const u8,
    ifa_dstaddr: *const u8,
    ifa_data: *const c_void,
}

unsafe extern "C" {
    fn getifaddrs(ifap: *mut *mut IfAddrs) -> i32;
    fn freeifaddrs(ifa: *mut IfAddrs);
}

const IFF_UP: u32 = 0x1;
const IFF_LOOPBACK: u32 = 0x8;
const AF_INET: u16 = 2;

#[cfg(target_os = "linux")]
mod sys {
    pub const AF_INET6: u16 = 10;
    pub const AF_PACKET: u16 = 17;
    pub const IFF_LOWER_UP: u32 = 0x10000;

    /// Linux: `sa_family_t` is 16 bits at the start.
    pub unsafe fn family(sa: *const u8) -> u16 {
        unsafe { std::ptr::read_unaligned(sa as *const u16) }
    }
}

#[cfg(target_os = "freebsd")]
mod sys {
    pub const AF_INET6: u16 = 28;
    pub const AF_LINK: u16 = 18;
    /// net/if.h: `LINK_STATE_UP`.
    pub const LINK_STATE_UP: u8 = 2;

    /// FreeBSD: `sa_len`, then an 8-bit `sa_family`.
    pub unsafe fn family(sa: *const u8) -> u16 {
        unsafe { *sa.add(1) as u16 }
    }
}

fn mac_text(bytes: &[u8]) -> Option<String> {
    (bytes.len() == 6 && bytes.iter().any(|b| *b != 0)).then(|| bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"))
}

/// The prefix length a netmask's bytes make.
pub fn prefix_of(mask: &[u8]) -> u8 {
    mask.iter().map(|b| b.count_ones() as u8).sum()
}

/// Every interface, in the order the system lists them.
pub fn list() -> io::Result<Vec<Iface>> {
    let mut head: *mut IfAddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `head` with a list freed below with freeifaddrs.
    if unsafe { getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut out: Vec<Iface> = Vec::new();
    let mut p = head;
    while !p.is_null() {
        // SAFETY: `p` is an entry of the list getifaddrs gave, alive until freeifaddrs.
        let e = unsafe { &*p };
        p = e.ifa_next;
        if e.ifa_name.is_null() {
            continue;
        }
        let name: String = unsafe { CStr::from_ptr(e.ifa_name) }.to_string_lossy().chars().filter(|c| !c.is_control()).take(32).collect();
        let i = match out.iter().position(|f| f.name == name) {
            Some(i) => i,
            None => {
                out.push(Iface { kind: kind_of(&name, e.ifa_flags & IFF_LOOPBACK != 0), name: name.clone(), ..Default::default() });
                out.len() - 1
            }
        };
        let f = &mut out[i];
        f.up |= e.ifa_flags & IFF_UP != 0;
        #[cfg(target_os = "linux")]
        {
            f.carrier |= e.ifa_flags & sys::IFF_LOWER_UP != 0;
        }
        if e.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: a socket address of at least its header, as the system gives it.
        let family = unsafe { sys::family(e.ifa_addr) };
        let mask = |len: usize, at: usize| -> u8 {
            if e.ifa_netmask.is_null() {
                return (len * 8) as u8;
            }
            // SAFETY: the netmask is a socket address of the same family.
            prefix_of(unsafe { std::slice::from_raw_parts(e.ifa_netmask.add(at), len) })
        };
        match family {
            // sockaddr_in: the address at offset 4 on both systems.
            AF_INET => {
                let b = unsafe { std::slice::from_raw_parts(e.ifa_addr.add(4), 4) };
                f.addrs.push((IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])), mask(4, 4)));
            }
            // sockaddr_in6: the address at offset 8 on both systems.
            sys::AF_INET6 => {
                let b = unsafe { std::slice::from_raw_parts(e.ifa_addr.add(8), 16) };
                let mut a = [0u8; 16];
                a.copy_from_slice(b);
                f.addrs.push((IpAddr::V6(Ipv6Addr::from(a)), mask(16, 8)));
            }
            // Linux: sockaddr_ll, sll_halen at 11, sll_addr at 12.
            #[cfg(target_os = "linux")]
            sys::AF_PACKET => {
                let len = unsafe { *e.ifa_addr.add(11) } as usize;
                if len <= 8 {
                    f.mac = mac_text(unsafe { std::slice::from_raw_parts(e.ifa_addr.add(12), len) });
                }
            }
            // FreeBSD: sockaddr_dl, sdl_nlen at 5, sdl_alen at 6, the name
            // then the address from sdl_data at 8; ifa_data the if_data,
            // its ifi_link_state at 4.
            #[cfg(target_os = "freebsd")]
            sys::AF_LINK => {
                let (nlen, alen) = unsafe { (*e.ifa_addr.add(5) as usize, *e.ifa_addr.add(6) as usize) };
                if nlen + alen <= 46 {
                    f.mac = mac_text(unsafe { std::slice::from_raw_parts(e.ifa_addr.add(8 + nlen), alen) });
                }
                if !e.ifa_data.is_null() {
                    f.carrier |= unsafe { *(e.ifa_data as *const u8).add(4) } == sys::LINK_STATE_UP;
                }
            }
            _ => {}
        }
    }
    // SAFETY: the list getifaddrs gave, freed once.
    unsafe { freeifaddrs(head) };
    for f in &mut out {
        if f.kind == Kind::Wired && is_phone(&f.name) {
            f.kind = Kind::Phone;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name() {
        assert_eq!(kind_of("wlp4s0", false), Kind::WiFi);
        assert_eq!(kind_of("wlan0", false), Kind::WiFi);
        assert_eq!(kind_of("em0", false), Kind::Wired);
        assert_eq!(kind_of("enp3s0", false), Kind::Wired);
        assert_eq!(kind_of("lo0", true), Kind::Loopback);
    }

    #[test]
    fn what_counts_as_a_cable() {
        let iface = |name: &str, kind: Kind, carrier: bool| Iface { name: name.into(), kind, up: true, carrier, mac: Some("aa:bb:cc:dd:ee:ff".into()), addrs: vec![] };
        assert!(is_cable(&iface("enp5s0", Kind::Wired, true)));
        assert!(is_cable(&iface("em0", Kind::Wired, true)));
        assert!(is_cable(&iface("eth1", Kind::Phone, true)), "a phone's USB tethering is a cable");
        assert!(!is_phone("../etc"));
        assert!(!is_cable(&iface("enp5s0", Kind::Wired, false)), "no carrier");
        assert!(!is_cable(&iface("wlp4s0", Kind::WiFi, true)));
        assert!(!is_cable(&iface("docker0", Kind::Wired, true)));
        assert!(!is_cable(&iface("virbr0", Kind::Wired, true)));
        assert!(!is_cable(&iface("wg0", Kind::Wired, true)));
        let mut lo = iface("lo", Kind::Loopback, true);
        lo.mac = None;
        assert!(!is_cable(&lo));
    }

    #[test]
    fn prefixes() {
        assert_eq!(prefix_of(&[255, 255, 255, 0]), 24);
        assert_eq!(prefix_of(&[255, 255, 240, 0]), 20);
        assert_eq!(prefix_of(&[0xff; 8]), 64);
    }

    #[test]
    fn this_machine_has_a_loopback_with_its_address() {
        let all = list().unwrap();
        let lo = all.iter().find(|f| f.kind == Kind::Loopback).expect("a loopback interface");
        assert!(lo.up);
        assert!(lo.addrs.iter().any(|(a, p)| *a == IpAddr::V4(Ipv4Addr::LOCALHOST) && *p == 8), "{lo:?}");
    }
}
