//! Where vdm's sockets and state are, by platform.

/// The unlock socket: any local user may connect; vdm answers only for the
/// caller's own account (threat model T7).
#[cfg(target_os = "linux")]
pub const UNLOCK_SOCKET: &str = "/run/vdm/unlock";
#[cfg(not(target_os = "linux"))]
pub const UNLOCK_SOCKET: &str = "/var/run/vdm/unlock";

/// The session socket (step 2, review C4): each user's server stays
/// connected, so vdm can lock them all before the machine sleeps.
#[cfg(target_os = "linux")]
pub const SESSION_SOCKET: &str = "/run/vdm/session";
#[cfg(not(target_os = "linux"))]
pub const SESSION_SOCKET: &str = "/var/run/vdm/session";

/// The control socket: root's only (`vdm before-sleep`, from the system's
/// sleep hooks).
#[cfg(target_os = "linux")]
pub const CONTROL_SOCKET: &str = "/run/vdm/control";
#[cfg(not(target_os = "linux"))]
pub const CONTROL_SOCKET: &str = "/var/run/vdm/control";

/// vdm's own state: the pause after failures per uid, kept across restarts (review M1).
#[cfg(target_os = "linux")]
pub const STATE_DIR: &str = "/var/lib/vdm";
#[cfg(not(target_os = "linux"))]
pub const STATE_DIR: &str = "/var/db/vdm";

/// The PAM service the unlock check runs (our own file, review I2): vdm
/// refuses to start without it rather than fall back to "other".
pub const UNLOCK_SERVICE: &str = "vdm-unlock";

/// Where PAM's service files are.
pub const PAM_DIR: &str = "/etc/pam.d";

/// Below this uid an account is a system account: no unlock, no login
/// (review I10; root is refused by uid 0 and by name, FreeBSD's `toor`
/// shares uid 0).
pub const MIN_UID: u32 = 1000;
