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
}
pub use n::*;

/// Message styles (the same on both).
pub const PROMPT_ECHO_OFF: c_int = 1;
pub const PROMPT_ECHO_ON: c_int = 2;
pub const ERROR_MSG: c_int = 3;
pub const TEXT_INFO: c_int = 4;
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

/// One PAM transaction for `user` with `service`: authenticate (an empty
/// password is not enough), then the account check, an expired password
/// allowed. PAM's prompts go to `conv`.
pub fn authenticate(service: &str, user: &str, conv: &mut dyn Conversation) -> Outcome {
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
    // SAFETY: the handle pam_start gave.
    let mut r = unsafe { pam_authenticate(h, DISALLOW_NULL_AUTHTOK) };
    if r == SUCCESS {
        // SAFETY: as above.
        let a = unsafe { pam_acct_mgmt(h, 0) };
        r = if a == NEW_AUTHTOK_REQD { SUCCESS } else { a };
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
