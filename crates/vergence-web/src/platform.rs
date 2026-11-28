//! A WPE platform of our own (web milestone 1, step 2): WPE's display, view
//! and toplevel subclassed in Rust, so WebKit hands its frames to us and we
//! hand them to Vergence's server, and takes input from us, with no Wayland
//! in between.
//!
//! Registered in code, not as a module in WebKit's folders (the research of
//! 2026-11-27): our display is given to each web view through WebKitWebView's
//! construct-only `display` property.
//!
//! The contract is WPE's headless platform's (WPEViewHeadless.cpp): WebKit
//! calls `render_buffer` with a finished frame; when the server says that
//! frame is on the screen we tell WebKit the frame before it is released and
//! this one rendered, and only then does it draw the next. This first cut
//! offers WebKit no EGL display, so it draws on the processor into shared
//! memory (WPEBufferSHM), which the native protocol's shared-memory pictures
//! take as they are; GPU buffers (dmabuf) come next.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::sync::OnceLock;

use crate::ffi::{GError, GPointer, GType, GTypeInfo, GTypeQuery, Gboolean, Lib, wpe_enum};

/// GObjectClass's size on a 64-bit machine; WPE's classes start with one.
const GOBJECT_CLASS_SIZE: usize = 136;
/// Each WPE class's virtual functions (WPEDisplay.h, WPEView.h,
/// WPEToplevel.h, 2.54), then 32 pointers of padding.
const DISPLAY_VFUNCS: usize = 13;
const VIEW_VFUNCS: usize = 9;
const TOPLEVEL_VFUNCS: usize = 7;
const PADDING: usize = 32;
const PTR: usize = std::mem::size_of::<usize>();

/// The size a WPE class must have for our offsets to be right.
pub(crate) const fn class_size(vfuncs: usize) -> usize {
    GOBJECT_CLASS_SIZE + (vfuncs + PADDING) * PTR
}

/// Where virtual function `index` of a WPE class sits in it.
pub(crate) const fn vfunc_offset(index: usize) -> usize {
    GOBJECT_CLASS_SIZE + index * PTR
}

// The virtual functions we fill, by their place in each class.
const DISPLAY_CONNECT: usize = 0;
const DISPLAY_CREATE_VIEW: usize = 1;
const DISPLAY_GET_KEYMAP: usize = 3;
const DISPLAY_CREATE_TOPLEVEL: usize = 11;
const VIEW_RENDER_BUFFER: usize = 1;
const VIEW_SET_CURSOR_FROM_NAME: usize = 4;
const TOPLEVEL_SET_TITLE: usize = 0;
const TOPLEVEL_RESIZE: usize = 2;

/// A finished frame from WebKit, in its pixels (premultiplied ARGB, the
/// native protocol's SHM_ARGB8888), and the rectangles that changed.
pub struct Frame<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub damage: Vec<(i32, i32, i32, i32)>,
}

/// Whoever shows the web view (the test window now; vergence-web's regions
/// in milestone 2).
pub trait Host {
    fn frame(&mut self, frame: Frame);
    /// The pointer's shape, by its CSS name ("pointer", "text", ...).
    fn cursor(&mut self, name: &str);
}

struct Types {
    display: GType,
    view: GType,
    toplevel: GType,
}

static LIB: OnceLock<Lib> = OnceLock::new();
static TYPES: OnceLock<Result<Types, String>> = OnceLock::new();

thread_local! {
    static HOST: RefCell<Option<Box<dyn Host>>> = RefCell::new(None);
    /// The frame WebKit gave last (shown at the next frame callback) and the
    /// one on the screen now: each holds a reference until released.
    static FRAMES: RefCell<(GPointer, GPointer)> = const { RefCell::new((std::ptr::null_mut(), std::ptr::null_mut())) };
    static KEYMAP: RefCell<GPointer> = const { RefCell::new(std::ptr::null_mut()) };
}

pub(crate) fn lib() -> &'static Lib {
    LIB.get().expect("WebKit opened")
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("no NUL")
}

/// Writes `f` into the class at virtual function `index`.
///
/// # Safety
/// `class` is a class structure of the size checked at registration, and
/// `f` is a function pointer of the C signature that slot declares.
unsafe fn set_vfunc(class: GPointer, index: usize, f: *const ()) {
    // SAFETY: per the function's contract; the slot is pointer-aligned.
    unsafe { *((class as *mut u8).add(vfunc_offset(index)) as *mut *const ()) = f };
}

unsafe extern "C" fn display_class_init(class: GPointer, _: GPointer) {
    // SAFETY: GObject calls this with our display class.
    unsafe {
        set_vfunc(class, DISPLAY_CONNECT, display_connect as *const ());
        set_vfunc(class, DISPLAY_CREATE_VIEW, display_create_view as *const ());
        set_vfunc(class, DISPLAY_GET_KEYMAP, display_get_keymap as *const ());
        set_vfunc(class, DISPLAY_CREATE_TOPLEVEL, display_create_toplevel as *const ());
    }
}

unsafe extern "C" fn view_class_init(class: GPointer, _: GPointer) {
    // SAFETY: GObject calls this with our view class.
    unsafe {
        set_vfunc(class, VIEW_RENDER_BUFFER, view_render_buffer as *const ());
        set_vfunc(class, VIEW_SET_CURSOR_FROM_NAME, view_set_cursor_from_name as *const ());
    }
}

unsafe extern "C" fn toplevel_class_init(class: GPointer, _: GPointer) {
    // SAFETY: GObject calls this with our toplevel class.
    unsafe {
        set_vfunc(class, TOPLEVEL_SET_TITLE, toplevel_set_title as *const ());
        set_vfunc(class, TOPLEVEL_RESIZE, toplevel_resize as *const ());
    }
}

/// Registers a subclass of `parent` named `name`, after checking the
/// parent's class is the size our offsets assume.
fn register(lib: &Lib, parent: GType, name: &str, vfuncs: usize, init: crate::ffi::ClassInit) -> Result<GType, String> {
    let mut q = GTypeQuery { type_: 0, type_name: std::ptr::null(), class_size: 0, instance_size: 0 };
    // SAFETY: a valid GType and a query to fill.
    unsafe { (lib.g_type_query)(parent, &mut q) };
    let want = class_size(vfuncs);
    if q.class_size as usize != want {
        return Err(format!(
            "this WebKit's {} class is {} bytes, vergence-web expects {want}: a WebKit version it was not written for",
            name, q.class_size
        ));
    }
    let info = GTypeInfo {
        class_size: q.class_size as u16,
        base_init: None,
        base_finalize: None,
        class_init: Some(init),
        class_finalize: None,
        class_data: std::ptr::null(),
        instance_size: q.instance_size as u16,
        n_preallocs: 0,
        instance_init: None,
        value_table: std::ptr::null(),
    };
    let n = cstr(name);
    // SAFETY: a parent type, a C name and a filled GTypeInfo (copied by GObject).
    let t = unsafe { (lib.g_type_register_static)(parent, n.as_ptr(), &info, 0) };
    if t == 0 { Err(format!("could not register {name}")) } else { Ok(t) }
}

fn types() -> Result<&'static Types, String> {
    TYPES
        .get_or_init(|| {
            let lib = lib();
            // SAFETY: WPE's get_type functions take nothing.
            let (d, v, t) =
                unsafe { ((lib.wpe_display_get_type)(), (lib.wpe_view_get_type)(), (lib.wpe_toplevel_get_type)()) };
            Ok(Types {
                display: register(lib, d, "VergenceWebDisplay", DISPLAY_VFUNCS, display_class_init)?,
                view: register(lib, v, "VergenceWebView", VIEW_VFUNCS, view_class_init)?,
                toplevel: register(lib, t, "VergenceWebToplevel", TOPLEVEL_VFUNCS, toplevel_class_init)?,
            })
        })
        .as_ref()
        .map_err(|e| e.clone())
}

unsafe extern "C" fn display_connect(_: GPointer, _: *mut *mut GError) -> Gboolean {
    1
}

unsafe extern "C" fn display_create_view(display: GPointer) -> GPointer {
    let lib = lib();
    let Ok(t) = types() else { return std::ptr::null_mut() };
    let key = cstr("display");
    // SAFETY: g_object_new with one property, NULL-terminated.
    let view = unsafe { (lib.g_object_new)(t.view, key.as_ptr(), display, std::ptr::null::<c_char>()) };
    // As WPE's headless view: once in a toplevel, the view takes its size and
    // is mapped. Focus waits for the window's (keyboard enter).
    let sig = cstr("notify::toplevel");
    // SAFETY: a signal of the view's, our handler of GObject's notify signature.
    unsafe {
        (lib.g_signal_connect_data)(
            view,
            sig.as_ptr(),
            std::mem::transmute::<unsafe extern "C" fn(GPointer, GPointer, GPointer), crate::ffi::Callback>(
                view_toplevel_changed,
            ),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    view
}

unsafe extern "C" fn view_toplevel_changed(view: GPointer, _: GPointer, _: GPointer) {
    let lib = lib();
    // SAFETY: the view the signal came from.
    unsafe {
        let top = (lib.wpe_view_get_toplevel)(view);
        if top.is_null() {
            return;
        }
        let (mut w, mut h) = (0, 0);
        (lib.wpe_toplevel_get_size)(top, &mut w, &mut h);
        if w > 0 && h > 0 {
            (lib.wpe_view_resized)(view, w, h);
        }
        (lib.wpe_view_map)(view);
    }
}

unsafe extern "C" fn display_get_keymap(_: GPointer) -> GPointer {
    KEYMAP.with(|k| {
        let mut k = k.borrow_mut();
        if k.is_null() {
            // SAFETY: takes nothing; the display keeps the keymap for good.
            *k = unsafe { (lib().wpe_keymap_xkb_new)() };
        }
        *k
    })
}

/// The display's keymap (WPE's xkb one), made on first use.
///
/// # Safety
/// Called on GLib's thread after the platform is made.
pub(crate) unsafe fn display_keymap() -> GPointer {
    // SAFETY: display_get_keymap ignores its display argument.
    unsafe { display_get_keymap(std::ptr::null_mut()) }
}

unsafe extern "C" fn display_create_toplevel(display: GPointer, _max_views: c_uint) -> GPointer {
    let lib = lib();
    let Ok(t) = types() else { return std::ptr::null_mut() };
    let key = cstr("display");
    // SAFETY: as in display_create_view.
    let top = unsafe { (lib.g_object_new)(t.toplevel, key.as_ptr(), display, std::ptr::null::<c_char>()) };
    // SAFETY: a toplevel just made.
    unsafe { (lib.wpe_toplevel_state_changed)(top, wpe_enum::TOPLEVEL_STATE_ACTIVE) };
    top
}

unsafe extern "C" fn view_render_buffer(
    _view: GPointer,
    buffer: GPointer,
    rects: *const [c_int; 4],
    n_rects: c_uint,
    _error: *mut *mut GError,
) -> Gboolean {
    let lib = lib();
    // SAFETY: the buffer WebKit hands us, alive for this call; the type check
    // comes before any SHM call.
    unsafe {
        if (lib.g_type_check_instance_is_a)(buffer, (lib.wpe_buffer_shm_get_type)()) == 0 {
            // No EGL display is offered, so WebKit sends shared memory only.
            return 0;
        }
        let bytes = (lib.wpe_buffer_shm_get_data)(buffer);
        let mut len = 0usize;
        let data = (lib.g_bytes_get_data)(bytes, &mut len);
        let (w, h) = ((lib.wpe_buffer_get_width)(buffer), (lib.wpe_buffer_get_height)(buffer));
        let stride = (lib.wpe_buffer_shm_get_stride)(buffer);
        if data.is_null() || w <= 0 || h <= 0 || (stride as usize) * (h as usize) > len {
            return 0;
        }
        let damage = (0..n_rects as usize)
            .map(|i| {
                let r = *rects.add(i);
                (r[0], r[1], r[2], r[3])
            })
            .collect();
        let frame =
            Frame { pixels: std::slice::from_raw_parts(data, len), width: w as u32, height: h as u32, stride, damage };
        HOST.with(|host| {
            if let Some(host) = host.borrow_mut().as_mut() {
                host.frame(frame);
            }
        });
        // Kept until the server has shown it (frame_done).
        (lib.g_object_ref)(buffer);
        FRAMES.with(|f| {
            let mut f = f.borrow_mut();
            if !f.0.is_null() {
                // A newer frame before the last was shown: dropped unshown,
                // as WPE's headless view drops its pending one.
                (lib.g_object_unref)(f.0);
            }
            f.0 = buffer;
        });
    }
    1
}

unsafe extern "C" fn view_set_cursor_from_name(_: GPointer, name: *const c_char) {
    if name.is_null() {
        return;
    }
    // SAFETY: a C string from WebKit.
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    HOST.with(|host| {
        if let Some(host) = host.borrow_mut().as_mut() {
            host.cursor(&name);
        }
    });
}

unsafe extern "C" fn toplevel_set_title(_: GPointer, _: *const c_char) {}

unsafe extern "C" fn toplevel_resize(_: GPointer, _: c_int, _: c_int) -> Gboolean {
    // A page asking to resize its window: Vergence's server decides sizes.
    0
}

/// The platform: WebKit opened, our types registered, our display connected.
pub struct Platform {
    display: GPointer,
}

impl Platform {
    pub fn new(host: Box<dyn Host>) -> Result<Platform, String> {
        if LIB.get().is_none() {
            let lib = Lib::open()?;
            let _ = LIB.set(lib);
        }
        let t = types()?;
        let lib = lib();
        // SAFETY: our display type, no properties.
        let display = unsafe { (lib.g_object_new)(t.display, std::ptr::null::<c_char>()) };
        let mut err: *mut GError = std::ptr::null_mut();
        // SAFETY: a display just made.
        if unsafe { (lib.wpe_display_connect)(display, &mut err) } == 0 {
            return Err(lib.take_error(err));
        }
        HOST.with(|h| *h.borrow_mut() = Some(host));
        Ok(Platform { display })
    }

    /// A web view on our display, smooth scrolling off (Dave, 2026-11-26).
    pub fn web_view(&self) -> crate::WebView {
        let lib = lib();
        let key = cstr("display");
        // SAFETY: WebKitWebView with its construct-only display.
        let web = unsafe {
            (lib.g_object_new)((lib.webkit_web_view_get_type)(), key.as_ptr(), self.display, std::ptr::null::<c_char>())
        };
        // SAFETY: the view's own settings object.
        unsafe { (lib.webkit_settings_set_enable_smooth_scrolling)((lib.webkit_web_view_get_settings)(web), 0) };
        crate::WebView::new(web)
    }
}

/// The server showed the last frame: the one before it is released, this one
/// rendered, and WebKit may draw the next.
pub(crate) fn frame_done(view: GPointer) {
    let lib = lib();
    FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        if f.0.is_null() {
            return;
        }
        // SAFETY: buffers we hold references to, for this view.
        unsafe {
            if !f.1.is_null() {
                (lib.wpe_view_buffer_released)(view, f.1);
                (lib.g_object_unref)(f.1);
            }
            f.1 = std::mem::replace(&mut f.0, std::ptr::null_mut());
            (lib.wpe_view_buffer_rendered)(view, f.1);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_sizes_match_wpe_2_54() {
        // WPE 2.54's headers; registration checks them against g_type_query.
        assert_eq!(class_size(DISPLAY_VFUNCS), 496);
        assert_eq!(class_size(VIEW_VFUNCS), 464);
        assert_eq!(class_size(TOPLEVEL_VFUNCS), 448);
    }

    #[test]
    fn vfunc_offsets() {
        assert_eq!(vfunc_offset(0), 136);
        assert_eq!(vfunc_offset(DISPLAY_CREATE_TOPLEVEL), 136 + 11 * 8);
    }
}
