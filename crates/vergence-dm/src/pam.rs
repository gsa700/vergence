//! PAM, the operating system's interface for logging in, through our own
//! declarations (Dave, 2026-11-16: an OS interface, not borrowing): Linux-PAM
//! on Linux, OpenPAM on FreeBSD. They agree on the calls and the structures
//! and **differ in the numbers** of several results, kept apart below (the
//! design review; OpenPAM's from FreeBSD's <security/pam_constants.h>,
//! Linux-PAM's from its <security/_pam_types.h>; a test asks each system's
//! PAM to name the numbers, so a wrong one fails there).
//!
//! [`authenticate`] runs one transaction: pam_start, pam_authenticate, then
//! pam_acct_mgmt tolerating an expired password (review I9: a user is never
//! shut out of a running session), pam_end. Every message PAM sends goes to
//! a [`Conversation`]; vdm's relays them to the client.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use crate::secret::Secret;

pub const SUCCESS: c_int = 0;
pub const BUF_ERR: c_int = 5;
#[cfg(target_os = "linux")]
mod n {
    use std::ffi::c_int;
    pub const PERM_DENIED: c_int = 6;
    pub const AUTH_ERR: c_int = 7;
    pub const USER_UNKNOWN: c_int = 10;
    pub const MAXTRIES: c_int = 11;
    pub const NEW_AUTHTOK_REQD: c_int = 12;
    pub const ACCT_EXPIRED: c_int = 13;
    pub const CONV_ERR: c_int = 19;
    /// pam_setcred's flags (Linux-PAM's _pam_types.h).
    pub const ESTABLISH_CRED: c_int = 0x2;
    pub const DELETE_CRED: c_int = 0x4;
    /// pam_chauthtok's flag (Linux-PAM's _pam_types.h).
    pub const CHANGE_EXPIRED_AUTHTOK: c_int = 0x20;
}
#[cfg(not(target_os = "linux"))]
mod n {
    use std::ffi::c_int;
    pub const CONV_ERR: c_int = 6;
    pub const PERM_DENIED: c_int = 7;
    pub const MAXTRIES: c_int = 8;
    pub const AUTH_ERR: c_int = 9;
    pub const NEW_AUTHTOK_REQD: c_int = 10;
    pub const USER_UNKNOWN: c_int = 13;
    pub const ACCT_EXPIRED: c_int = 17;
    /// pam_setcred's flags (OpenPAM's pam_constants.h).
    pub const ESTABLISH_CRED: c_int = 0x1;
    pub const DELETE_CRED: c_int = 0x2;
    /// pam_chauthtok's flag (OpenPAM's pam_constants.h).
    pub const CHANGE_EXPIRED_AUTHTOK: c_int = 0x4;
}
pub use n::*;

/// Message styles (the same on both).
pub const PROMPT_ECHO_OFF: c_int = 1;
pub const PROMPT_ECHO_ON: c_int = 2;
pub const ERROR_MSG: c_int = 3;
pub const TEXT_INFO: c_int = 4;
/// The terminal item (the same on both).
pub const TTY: c_int = 3;
/// The user item (the same on both).
pub const USER: c_int = 2;
/// pam_authenticate's flag: an empty password is not enough (the same on both).
pub const DISALLOW_NULL_AUTHTOK: c_int = 1;
/// The most messages one conversation call may bring (both: 32).
const MAX_NUM_MSG: c_int = 32;

#[repr(C)]
struct Message {
    style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct Response {
    resp: *mut c_char,
    retcode: c_int,
}

type ConvFn = extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int;

#[repr(C)]
struct Conv {
    conv: ConvFn,
    appdata: *mut c_void,
}

unsafe extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const Conv, handle: *mut *mut c_void) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
    fn pam_strerror(handle: *mut c_void, errnum: c_int) -> *const c_char;
    fn pam_setcred(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_open_session(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_close_session(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_putenv(handle: *mut c_void, name_value: *const c_char) -> c_int;
    fn pam_getenvlist(handle: *mut c_void) -> *mut *mut c_char;
    fn pam_set_item(handle: *mut c_void, item: c_int, value: *const c_void) -> c_int;
    fn pam_get_item(handle: *mut c_void, item: c_int, value: *mut *const c_void) -> c_int;
    fn pam_chauthtok(handle: *mut c_void, flags: c_int) -> c_int;
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// PAM's own words for a result.
pub fn describe(code: c_int) -> String {
    // SAFETY: both implementations take a null handle here and return a
    // static string (or null).
    let p = unsafe { pam_strerror(std::ptr::null_mut(), code) };
    if p.is_null() {
        return format!("PAM error {code}");
    }
    // SAFETY: a NUL-terminated static string.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// What PAM says during a transaction, and the answers to its prompts.
pub trait Conversation {
    /// A prompt; the answer, or None to give up (the transaction fails).
    fn prompt(&mut self, echo: bool, text: &str) -> Option<Secret>;
    fn info(&mut self, text: &str);
    fn error(&mut self, text: &str);
}

/// How a transaction ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The person is who they say, and the account may be used.
    Yes,
    /// Wrong password, unknown user, too many tries, account not allowed.
    No,
    /// The conversation was given up (the client went away, cancelled).
    GivenUp,
    /// PAM could not run (no service file, a module failed to load).
    Broken,
}

struct State<'a> {
    conv: &'a mut dyn Conversation,
    gave_up: bool,
}

extern "C" fn converse(num: c_int, msgs: *mut *const Message, resp: *mut *mut Response, appdata: *mut c_void) -> c_int {
    if num <= 0 || num > MAX_NUM_MSG || msgs.is_null() || resp.is_null() || appdata.is_null() {
        return n::CONV_ERR;
    }
    // SAFETY: appdata is the State authenticate() passed for this transaction,
    // alive for its whole length.
    let state = unsafe { &mut *(appdata as *mut State) };
    // Responses are freed by PAM with free(): allocated with calloc.
    // SAFETY: num checked above.
    let out = unsafe { calloc(num as usize, std::mem::size_of::<Response>()) } as *mut Response;
    if out.is_null() {
        return BUF_ERR;
    }
    for i in 0..num as usize {
        // SAFETY: both implementations pass an array of num pointers to messages.
        let m = unsafe { *msgs.add(i) };
        if m.is_null() {
            continue;
        }
        // SAFETY: a message PAM made, with a NUL-terminated text (or null).
        let (style, text) = unsafe { ((*m).style, if (*m).msg.is_null() { String::new() } else { CStr::from_ptr((*m).msg).to_string_lossy().into_owned() }) };
        match style {
            PROMPT_ECHO_OFF | PROMPT_ECHO_ON => {
                let Some(answer) = state.conv.prompt(style == PROMPT_ECHO_ON, &text) else {
                    state.gave_up = true;
                    free_responses(out, i);
                    return n::CONV_ERR;
                };
                // A copy PAM owns and frees; ours (the Secret) is wiped when dropped.
                let len = answer.len();
                // SAFETY: len + 1 bytes, zeroed, for a NUL-terminated copy.
                let p = unsafe { calloc(len + 1, 1) } as *mut u8;
                if p.is_null() {
                    free_responses(out, i);
                    return BUF_ERR;
                }
                // SAFETY: len bytes into a buffer of len + 1.
                unsafe {
                    std::ptr::copy_nonoverlapping(answer.as_bytes().as_ptr(), p, len);
                    (*out.add(i)).resp = p as *mut c_char;
                }
            }
            ERROR_MSG => state.conv.error(&text),
            TEXT_INFO => state.conv.info(&text),
            _ => {}
        }
    }
    // SAFETY: resp is PAM's out-parameter.
    unsafe { *resp = out };
    SUCCESS
}

/// Free the first `n` responses made so far, and the array, wiping each answer first.
fn free_responses(out: *mut Response, n: usize) {
    for i in 0..n {
        // SAFETY: entries 0..n were filled by converse (or are null).
        unsafe {
            let p = (*out.add(i)).resp;
            if !p.is_null() {
                let len = CStr::from_ptr(p).to_bytes().len();
                for k in 0..len {
                    std::ptr::write_volatile(p.add(k), 0);
                }
                free(p.cast());
            }
        }
    }
    // SAFETY: the array calloc gave.
    unsafe { free(out.cast()) };
}

/// Linux-PAM's delay after a failure (pam_unix's 2 s, pam_faildelay's), taken
/// over by the application through the PAM_FAIL_DELAY item (10): vdm keeps a
/// pause of its own per account (1, 2, 4 ... s, across restarts), so PAM's
/// wait only made the "no" slow (Dave, 2026-11-16: FreeBSD, whose OpenPAM has
/// no such delay, was "smoother and more consistent"). Called by PAM; waits for nothing.
#[cfg(target_os = "linux")]
extern "C" fn no_fail_delay(_retval: c_int, _usec: std::ffi::c_uint, _appdata: *mut c_void) {}

/// A transaction that has said yes, for what a login does next (step 5):
/// the session's items and environment, credentials, opening and closing it.
pub struct Handle(*mut c_void);

impl Handle {
    pub fn set_item(&self, item: c_int, value: &str) -> c_int {
        let Ok(v) = CString::new(value) else { return BUF_ERR };
        // SAFETY: PAM copies the string.
        unsafe { pam_set_item(self.0, item, v.as_ptr().cast()) }
    }

    /// One `NAME=value` in PAM's environment (modules such as pam_systemd read it).
    pub fn putenv(&self, name_value: &str) -> c_int {
        let Ok(v) = CString::new(name_value) else { return BUF_ERR };
        // SAFETY: PAM copies the string.
        unsafe { pam_putenv(self.0, v.as_ptr()) }
    }

    pub fn setcred(&self, flags: c_int) -> c_int {
        // SAFETY: the transaction's handle.
        unsafe { pam_setcred(self.0, flags) }
    }

    pub fn open_session(&self) -> c_int {
        // SAFETY: as above.
        unsafe { pam_open_session(self.0, 0) }
    }

    pub fn close_session(&self) -> c_int {
        // SAFETY: as above.
        unsafe { pam_close_session(self.0, 0) }
    }

    /// PAM_USER as PAM has it now.
    pub fn user(&self) -> Option<String> {
        let mut p: *const c_void = std::ptr::null();
        // SAFETY: an out-parameter; PAM keeps the string.
        if unsafe { pam_get_item(self.0, USER, &mut p) } != SUCCESS || p.is_null() {
            return None;
        }
        // SAFETY: a NUL-terminated string PAM owns.
        Some(unsafe { CStr::from_ptr(p.cast()) }.to_string_lossy().into_owned())
    }

    /// PAM's environment for the session (what its modules set), `NAME=value` each.
    pub fn env(&self) -> Vec<String> {
        // SAFETY: a NULL-terminated array of malloc'd strings, all ours to free.
        let list = unsafe { pam_getenvlist(self.0) };
        let mut out = Vec::new();
        if list.is_null() {
            return out;
        }
        let mut i = 0;
        loop {
            // SAFETY: within the array, up to its NULL.
            let p = unsafe { *list.add(i) };
            if p.is_null() {
                break;
            }
            // SAFETY: a NUL-terminated string, freed once.
            unsafe {
                out.push(CStr::from_ptr(p).to_string_lossy().into_owned());
                free(p.cast());
            }
            i += 1;
        }
        // SAFETY: the array itself.
        unsafe { free(list.cast()) };
        out
    }
}

/// One PAM transaction for `user` with `service`: authenticate (an empty
/// password is not enough), then the account check, an expired password
/// allowed. PAM's prompts go to `conv`.
pub fn authenticate(service: &str, user: &str, conv: &mut dyn Conversation) -> Outcome {
    authenticate_then(service, user, conv, &Options::default(), &mut |_| {})
}

/// How a transaction goes beyond the check itself.
#[derive(Default)]
pub struct Options {
    /// PAM_TTY before the check, so rules by terminal apply to it (a login:
    /// security review 2026-11-16, finding 10).
    pub tty: Option<String>,
    /// An expired password must be changed, through the conversation, before
    /// the yes (a login: finding 6). Without it an expired password is let
    /// through (an unlock: review I9, never shut out of a running session).
    pub change_expired: bool,
}

/// As [`authenticate`]; on a yes, `then` runs with the transaction still
/// open (a login opens its session there and runs it to its end), before
/// pam_end.
pub fn authenticate_then(service: &str, user: &str, conv: &mut dyn Conversation, opts: &Options, then: &mut dyn FnMut(&Handle)) -> Outcome {
    let wanted = user.to_string();
    let (Ok(service), Ok(user)) = (CString::new(service), CString::new(user)) else { return Outcome::Broken };
    let mut state = State { conv, gave_up: false };
    let c = Conv { conv: converse, appdata: (&mut state as *mut State).cast() };
    let mut h: *mut c_void = std::ptr::null_mut();
    // SAFETY: NUL-terminated strings and a Conv that outlive the transaction
    // (it ends with pam_end below, before they go).
    let r = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &c, &mut h) };
    if r != SUCCESS || h.is_null() {
        return Outcome::Broken;
    }
    #[cfg(target_os = "linux")]
    {
        // PAM_FAIL_DELAY 10: our function instead of PAM's wait (see no_fail_delay).
        let f: extern "C" fn(c_int, std::ffi::c_uint, *mut c_void) = no_fail_delay;
        // SAFETY: the handle pam_start gave; a function pointer PAM calls with these arguments.
        unsafe { pam_set_item(h, 10, f as *const c_void) };
    }
    if let Some(t) = &opts.tty {
        Handle(h).set_item(TTY, t);
    }
    // SAFETY: the handle pam_start gave.
    let mut r = unsafe { pam_authenticate(h, DISALLOW_NULL_AUTHTOK) };
    // The account is the one asked for: a module may change PAM_USER, and
    // the session must not then start as someone else (finding 10).
    if r == SUCCESS && Handle(h).user().as_deref() != Some(wanted.as_str()) {
        r = PERM_DENIED;
    }
    if r == SUCCESS {
        // SAFETY: as above.
        let a = unsafe { pam_acct_mgmt(h, 0) };
        r = match a {
            // SAFETY: as above; its prompts go through the conversation.
            NEW_AUTHTOK_REQD if opts.change_expired => unsafe { pam_chauthtok(h, CHANGE_EXPIRED_AUTHTOK) },
            NEW_AUTHTOK_REQD => SUCCESS,
            other => other,
        };
    }
    if r == SUCCESS {
        then(&Handle(h));
    }
    // SAFETY: the handle, ended once.
    unsafe { pam_end(h, r) };
    match r {
        SUCCESS => Outcome::Yes,
        _ if state.gave_up => Outcome::GivenUp,
        AUTH_ERR | USER_UNKNOWN | MAXTRIES | PERM_DENIED | ACCT_EXPIRED => Outcome::No,
        _ => Outcome::Broken,
    }
}

/// A transaction with no check, for a session alone (the login screen's
/// own, VDM step 7: logind gives the seat only to a process in a session,
/// review I4): pam_start, `then` (which opens and closes the session), pam_end.
/// Prompts are refused. With `check_account` (automatic login: security
/// review 2026-11-16, finding 8), pam_acct_mgmt first: an account that is
/// locked, expired, or shut out by nologin is refused. Ok when `then` ran.
pub fn session_only(service: &str, user: &str, check_account: bool, then: &mut dyn FnMut(&Handle)) -> Result<(), String> {
    struct Silent;
    impl Conversation for Silent {
        fn prompt(&mut self, _echo: bool, _text: &str) -> Option<Secret> {
            None
        }
        fn info(&mut self, _text: &str) {}
        fn error(&mut self, _text: &str) {}
    }
    let (Ok(service), Ok(user)) = (CString::new(service), CString::new(user)) else { return Err("a name with a NUL".into()) };
    let mut silent = Silent;
    let mut state = State { conv: &mut silent, gave_up: false };
    let c = Conv { conv: converse, appdata: (&mut state as *mut State).cast() };
    let mut h: *mut c_void = std::ptr::null_mut();
    // SAFETY: strings and a Conv that outlive the transaction (pam_end below).
    let r = unsafe { pam_start(service.as_ptr(), user.as_ptr(), &c, &mut h) };
    if r != SUCCESS || h.is_null() {
        return Err(format!("PAM did not start: {}", describe(r)));
    }
    if check_account {
        // SAFETY: the handle pam_start gave.
        let a = unsafe { pam_acct_mgmt(h, 0) };
        if a != SUCCESS {
            // SAFETY: the handle, ended once.
            unsafe { pam_end(h, a) };
            return Err(format!("the account may not log in now: {}", describe(a)));
        }
    }
    then(&Handle(h));
    // SAFETY: the handle, ended once.
    unsafe { pam_end(h, SUCCESS) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers are this system's: PAM names them as we expect.
    #[test]
    fn the_result_numbers_are_this_systems() {
        let low = |c| describe(c).to_lowercase();
        assert!(low(NEW_AUTHTOK_REQD).contains("new"), "NEW_AUTHTOK_REQD: {}", describe(NEW_AUTHTOK_REQD));
        assert!(low(CONV_ERR).contains("conversation"), "CONV_ERR: {}", describe(CONV_ERR));
        assert!(low(AUTH_ERR).contains("authentication"), "AUTH_ERR: {}", describe(AUTH_ERR));
        assert!(low(USER_UNKNOWN).contains("user"), "USER_UNKNOWN: {}", describe(USER_UNKNOWN));
        assert!(low(ACCT_EXPIRED).contains("expired"), "ACCT_EXPIRED: {}", describe(ACCT_EXPIRED));
        assert!(low(MAXTRIES).contains("tr") || low(MAXTRIES).contains("max"), "MAXTRIES: {}", describe(MAXTRIES));
        assert!(low(PERM_DENIED).contains("permission"), "PERM_DENIED: {}", describe(PERM_DENIED));
    }
}
