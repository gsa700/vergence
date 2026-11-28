//! The C calls vergence-web makes into GLib, GObject, WPE Platform and WebKit,
//! found at run time in libWPEWebKit-2.0.so.1 (which brings GLib with it).
//!
//! Our own small binding (web milestone 1, question 6: Rust, nothing more
//! borrowed): only the calls used, each a function pointer read with dlsym.
//! The structures GObject needs from us (GTypeInfo, GTypeQuery, GError) are
//! written out here; WPE's class structures are not, since their sizes are
//! asked of GObject (`g_type_query`) and only the offsets of the virtual
//! functions we fill are computed (platform.rs checks them against GObject's
//! sizes, so a WebKit with a different layout is refused, not corrupted).

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};

pub type GType = usize;
pub type Gboolean = c_int;
pub type GPointer = *mut c_void;

#[repr(C)]
pub struct GTypeQuery {
    pub type_: GType,
    pub type_name: *const c_char,
    pub class_size: c_uint,
    pub instance_size: c_uint,
}

pub type ClassInit = unsafe extern "C" fn(class: GPointer, data: GPointer);

#[repr(C)]
pub struct GTypeInfo {
    pub class_size: u16,
    pub base_init: Option<unsafe extern "C" fn(GPointer)>,
    pub base_finalize: Option<unsafe extern "C" fn(GPointer)>,
    pub class_init: Option<ClassInit>,
    pub class_finalize: Option<unsafe extern "C" fn(GPointer, GPointer)>,
    pub class_data: *const c_void,
    pub instance_size: u16,
    pub n_preallocs: u16,
    pub instance_init: Option<unsafe extern "C" fn(GPointer, GPointer)>,
    pub value_table: *const c_void,
}

#[repr(C)]
pub struct GError {
    pub domain: u32,
    pub code: c_int,
    pub message: *const c_char,
}

/// WPE's enumerations, as numbers (WPEEvent.h, WPEToplevel.h).
pub mod wpe_enum {
    pub const EVENT_POINTER_DOWN: u32 = 1;
    pub const EVENT_POINTER_UP: u32 = 2;
    pub const EVENT_POINTER_MOVE: u32 = 3;
    pub const EVENT_POINTER_ENTER: u32 = 4;
    pub const EVENT_POINTER_LEAVE: u32 = 5;
    pub const EVENT_KEYBOARD_KEY_DOWN: u32 = 7;
    pub const EVENT_KEYBOARD_KEY_UP: u32 = 8;
    pub const INPUT_SOURCE_MOUSE: u32 = 0;
    pub const INPUT_SOURCE_KEYBOARD: u32 = 2;
    pub const INPUT_SOURCE_TOUCHPAD: u32 = 4;
    pub const MODIFIER_CONTROL: u32 = 1 << 0;
    pub const MODIFIER_SHIFT: u32 = 1 << 1;
    pub const MODIFIER_ALT: u32 = 1 << 2;
    pub const MODIFIER_META: u32 = 1 << 3;
    pub const MODIFIER_CAPS_LOCK: u32 = 1 << 4;
    pub const MODIFIER_BUTTON1: u32 = 1 << 8;
    pub const TOPLEVEL_STATE_ACTIVE: u32 = 1 << 2;
}

/// GIOCondition's G_IO_IN, G_IO_HUP, G_IO_ERR.
pub const G_IO_IN: c_uint = 1;
pub const G_IO_ERR: c_uint = 8;
pub const G_IO_HUP: c_uint = 16;

type FdFunc = unsafe extern "C" fn(fd: c_int, condition: c_uint, data: GPointer) -> Gboolean;
pub type Callback = unsafe extern "C" fn();

unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

macro_rules! calls {
    ($($name:ident : $ty:ty;)*) => {
        /// The calls, read once from the library.
        #[allow(non_snake_case)]
        pub struct Lib {
            $(pub $name: $ty,)*
        }

        impl Lib {
            fn resolve(handle: *mut c_void) -> Result<Lib, String> {
                Ok(Lib {
                    $($name: {
                        let n = CString::new(stringify!($name)).expect("name");
                        // SAFETY: dlsym with a live handle and a C string.
                        let p = unsafe { dlsym(handle, n.as_ptr()) };
                        if p.is_null() {
                            return Err(format!("{} not found in WebKit", stringify!($name)));
                        }
                        // SAFETY: the symbol is the C function of that name, whose
                        // signature is the one written here (from WebKit's headers).
                        unsafe { std::mem::transmute::<*mut c_void, $ty>(p) }
                    },)*
                })
            }
        }
    };
}

calls! {
    // GObject's type system.
    g_type_query: unsafe extern "C" fn(GType, *mut GTypeQuery);
    g_type_register_static: unsafe extern "C" fn(GType, *const c_char, *const GTypeInfo, c_uint) -> GType;
    g_type_check_instance_is_a: unsafe extern "C" fn(GPointer, GType) -> Gboolean;
    g_object_new: unsafe extern "C" fn(GType, *const c_char, ...) -> GPointer;
    g_object_ref: unsafe extern "C" fn(GPointer) -> GPointer;
    g_object_unref: unsafe extern "C" fn(GPointer);
    g_signal_connect_data: unsafe extern "C" fn(GPointer, *const c_char, Callback, GPointer, GPointer, c_uint) -> u64;
    g_error_free: unsafe extern "C" fn(*mut GError);
    // GLib's main loop.
    g_main_loop_new: unsafe extern "C" fn(GPointer, Gboolean) -> GPointer;
    g_main_loop_run: unsafe extern "C" fn(GPointer);
    g_main_loop_quit: unsafe extern "C" fn(GPointer);
    g_unix_fd_add: unsafe extern "C" fn(c_int, c_uint, FdFunc, GPointer) -> c_uint;
    g_bytes_get_data: unsafe extern "C" fn(GPointer, *mut usize) -> *const u8;
    // WPE Platform.
    wpe_display_get_type: unsafe extern "C" fn() -> GType;
    wpe_view_get_type: unsafe extern "C" fn() -> GType;
    wpe_toplevel_get_type: unsafe extern "C" fn() -> GType;
    wpe_buffer_shm_get_type: unsafe extern "C" fn() -> GType;
    wpe_display_connect: unsafe extern "C" fn(GPointer, *mut *mut GError) -> Gboolean;
    wpe_buffer_get_width: unsafe extern "C" fn(GPointer) -> c_int;
    wpe_buffer_get_height: unsafe extern "C" fn(GPointer) -> c_int;
    wpe_buffer_shm_get_data: unsafe extern "C" fn(GPointer) -> GPointer;
    wpe_buffer_shm_get_stride: unsafe extern "C" fn(GPointer) -> c_uint;
    wpe_view_get_toplevel: unsafe extern "C" fn(GPointer) -> GPointer;
    wpe_view_resized: unsafe extern "C" fn(GPointer, c_int, c_int);
    wpe_view_map: unsafe extern "C" fn(GPointer);
    wpe_view_focus_in: unsafe extern "C" fn(GPointer);
    wpe_view_focus_out: unsafe extern "C" fn(GPointer);
    wpe_view_buffer_rendered: unsafe extern "C" fn(GPointer, GPointer);
    wpe_view_buffer_released: unsafe extern "C" fn(GPointer, GPointer);
    wpe_view_event: unsafe extern "C" fn(GPointer, GPointer);
    wpe_toplevel_resized: unsafe extern "C" fn(GPointer, c_int, c_int);
    wpe_toplevel_scale_changed: unsafe extern "C" fn(GPointer, f64);
    wpe_toplevel_state_changed: unsafe extern "C" fn(GPointer, c_uint);
    wpe_toplevel_get_size: unsafe extern "C" fn(GPointer, *mut c_int, *mut c_int);
    wpe_keymap_xkb_new: unsafe extern "C" fn() -> GPointer;
    wpe_keymap_translate_keyboard_state:
        unsafe extern "C" fn(GPointer, c_uint, c_uint, c_int, *mut c_uint, *mut c_int, *mut c_int, *mut c_uint) -> Gboolean;
    wpe_event_pointer_move_new: unsafe extern "C" fn(c_uint, GPointer, c_uint, u32, c_uint, f64, f64, f64, f64) -> GPointer;
    wpe_event_pointer_button_new:
        unsafe extern "C" fn(c_uint, GPointer, c_uint, u32, c_uint, c_uint, f64, f64, c_uint) -> GPointer;
    wpe_event_scroll_new:
        unsafe extern "C" fn(GPointer, c_uint, u32, c_uint, f64, f64, Gboolean, Gboolean, f64, f64) -> GPointer;
    wpe_event_keyboard_new: unsafe extern "C" fn(c_uint, GPointer, c_uint, u32, c_uint, c_uint, c_uint) -> GPointer;
    wpe_event_unref: unsafe extern "C" fn(GPointer);
    // WebKit.
    webkit_web_view_get_type: unsafe extern "C" fn() -> GType;
    webkit_web_view_load_uri: unsafe extern "C" fn(GPointer, *const c_char);
    webkit_web_view_get_wpe_view: unsafe extern "C" fn(GPointer) -> GPointer;
    webkit_web_view_get_settings: unsafe extern "C" fn(GPointer) -> GPointer;
    webkit_web_view_get_title: unsafe extern "C" fn(GPointer) -> *const c_char;
    webkit_settings_set_enable_smooth_scrolling: unsafe extern "C" fn(GPointer, Gboolean);
}

/// The library's name; `VERGENCE_WEBKIT_LIB` names another (a lab build).
pub const LIB_NAME: &str = "libWPEWebKit-2.0.so.1";

impl Lib {
    /// Opens WebKit and reads every call, or says what is missing.
    pub fn open() -> Result<Lib, String> {
        let name = std::env::var("VERGENCE_WEBKIT_LIB").unwrap_or_else(|_| LIB_NAME.to_string());
        let c = CString::new(name.clone()).map_err(|_| "bad library name".to_string())?;
        // RTLD_GLOBAL: WebKit's own helpers resolve GLib's symbols through it.
        // SAFETY: dlopen with a C string.
        let handle = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
        if handle.is_null() {
            // SAFETY: dlerror returns a C string or null after a failed dlopen.
            let why = unsafe { dlerror() };
            let why = if why.is_null() {
                String::new()
            } else {
                // SAFETY: non-null, from dlerror.
                unsafe { CStr::from_ptr(why) }.to_string_lossy().into_owned()
            };
            return Err(format!("WebKit is not installed ({name}: {why}); the wpewebkit package provides it"));
        }
        Lib::resolve(handle)
    }

    /// A GError's message, freed.
    pub fn take_error(&self, e: *mut GError) -> String {
        if e.is_null() {
            return "unknown error".into();
        }
        // SAFETY: a GError GLib gave us; read, then freed once.
        unsafe {
            let m = if (*e).message.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*e).message).to_string_lossy().into_owned()
            };
            (self.g_error_free)(e);
            m
        }
    }
}
