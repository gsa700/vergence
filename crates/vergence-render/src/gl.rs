//! The operating system's graphics interfaces, as the GPU renderer calls
//! them: EGL (a context without a window), OpenGL ES 3 (drawing) and GBM
//! (the GPU device). These are system interfaces, not borrowed parts
//! (whitepaper §8).
//!
//! They are loaded when a renderer is made (`dlopen`), not linked when the
//! program is built: a machine without them, or without their development
//! files (Techbench has only the run-time libraries), still builds and runs,
//! and draws in software. Only the functions the renderer uses are declared.

use std::ffi::{c_char, c_void};

unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const RTLD_NOW: i32 = 2;

/// The first of these libraries that loads.
fn open(names: &[&std::ffi::CStr]) -> Option<*mut c_void> {
    names.iter().map(|n| unsafe { dlopen(n.as_ptr(), RTLD_NOW) }).find(|h| !h.is_null())
}

/// Declares a struct of functions and how to load it from a library.
macro_rules! library {
    ($(#[$doc:meta])* $name:ident from [$($so:expr),+] { $(fn $f:ident($($a:ident: $t:ty),*) $(-> $r:ty)?;)* }) => {
        $(#[$doc])*
        #[allow(non_snake_case)]
        pub struct $name {
            $(pub $f: unsafe extern "C" fn($($a: $t),*) $(-> $r)?,)*
        }

        impl $name {
            /// None if the library, or any function wanted from it, is missing.
            pub fn load() -> Option<$name> {
                let handle = open(&[$($so),+])?;
                Some($name {
                    $($f: {
                        // SAFETY: a live library handle and a zero-terminated name.
                        let p = unsafe { dlsym(handle, concat!(stringify!($f), "\0").as_ptr() as *const c_char) };
                        if p.is_null() {
                            return None;
                        }
                        // SAFETY: the symbol is the C function of this name, whose signature is the one declared here.
                        unsafe { std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($a: $t),*) $(-> $r)?>(p) }
                    },)*
                })
            }
        }
    };
}

library! {
    /// GBM: the GPU as a device buffers come from.
    Gbm from [c"libgbm.so.1", c"libgbm.so"] {
        fn gbm_create_device(fd: i32) -> *mut c_void;
        fn gbm_device_destroy(device: *mut c_void);
    }
}

library! {
    /// EGL: a drawing context.
    Egl from [c"libEGL.so.1", c"libEGL.so"] {
        fn eglGetPlatformDisplay(platform: u32, native: *mut c_void, attributes: *const isize) -> *mut c_void;
        fn eglInitialize(display: *mut c_void, major: *mut i32, minor: *mut i32) -> u32;
        fn eglBindAPI(api: u32) -> u32;
        fn eglChooseConfig(display: *mut c_void, attributes: *const i32, configs: *mut *mut c_void, size: i32, count: *mut i32) -> u32;
        fn eglCreateContext(display: *mut c_void, config: *mut c_void, share: *mut c_void, attributes: *const i32) -> *mut c_void;
        fn eglDestroyContext(display: *mut c_void, context: *mut c_void) -> u32;
        fn eglMakeCurrent(display: *mut c_void, draw: *mut c_void, read: *mut c_void, context: *mut c_void) -> u32;
        fn eglGetError() -> i32;
    }
}

pub const EGL_PLATFORM_GBM: u32 = 0x31D7;
pub const EGL_PLATFORM_SURFACELESS: u32 = 0x31DD;
pub const EGL_OPENGL_ES_API: u32 = 0x30A0;
pub const EGL_CONTEXT_MAJOR_VERSION: i32 = 0x3098;
pub const EGL_RENDERABLE_TYPE: i32 = 0x3040;
pub const EGL_OPENGL_ES3_BIT: i32 = 0x0040;
pub const EGL_NONE: i32 = 0x3038;

library! {
    /// OpenGL ES 3.
    Gles from [c"libGLESv2.so.2", c"libGLESv2.so"] {
        fn glGetString(name: u32) -> *const c_char;
        fn glGetError() -> u32;
        fn glGetIntegerv(name: u32, value: *mut i32);
        fn glCreateShader(kind: u32) -> u32;
        fn glShaderSource(shader: u32, count: i32, strings: *const *const c_char, lengths: *const i32);
        fn glCompileShader(shader: u32);
        fn glGetShaderiv(shader: u32, name: u32, value: *mut i32);
        fn glGetShaderInfoLog(shader: u32, size: i32, length: *mut i32, log: *mut c_char);
        fn glDeleteShader(shader: u32);
        fn glCreateProgram() -> u32;
        fn glAttachShader(program: u32, shader: u32);
        fn glLinkProgram(program: u32);
        fn glGetProgramiv(program: u32, name: u32, value: *mut i32);
        fn glGetProgramInfoLog(program: u32, size: i32, length: *mut i32, log: *mut c_char);
        fn glUseProgram(program: u32);
        fn glDeleteProgram(program: u32);
        fn glGetUniformLocation(program: u32, name: *const c_char) -> i32;
        fn glUniform1i(location: i32, value: i32);
        fn glUniform2f(location: i32, x: f32, y: f32);
        fn glGenVertexArrays(count: i32, arrays: *mut u32);
        fn glBindVertexArray(array: u32);
        fn glDeleteVertexArrays(count: i32, arrays: *const u32);
        fn glGenBuffers(count: i32, buffers: *mut u32);
        fn glBindBuffer(target: u32, buffer: u32);
        fn glBufferData(target: u32, size: isize, data: *const c_void, usage: u32);
        fn glDeleteBuffers(count: i32, buffers: *const u32);
        fn glEnableVertexAttribArray(index: u32);
        fn glVertexAttribPointer(index: u32, size: i32, kind: u32, normalized: u8, stride: i32, offset: *const c_void);
        fn glVertexAttribDivisor(index: u32, divisor: u32);
        fn glGenTextures(count: i32, textures: *mut u32);
        fn glBindTexture(target: u32, texture: u32);
        fn glDeleteTextures(count: i32, textures: *const u32);
        fn glTexImage2D(target: u32, level: i32, internal: i32, width: i32, height: i32, border: i32, format: u32, kind: u32, pixels: *const c_void);
        fn glTexSubImage2D(target: u32, level: i32, x: i32, y: i32, width: i32, height: i32, format: u32, kind: u32, pixels: *const c_void);
        fn glTexParameteri(target: u32, name: u32, value: i32);
        fn glPixelStorei(name: u32, value: i32);
        fn glActiveTexture(unit: u32);
        fn glGenFramebuffers(count: i32, framebuffers: *mut u32);
        fn glBindFramebuffer(target: u32, framebuffer: u32);
        fn glDeleteFramebuffers(count: i32, framebuffers: *const u32);
        fn glFramebufferTexture2D(target: u32, attachment: u32, texture_target: u32, texture: u32, level: i32);
        fn glCheckFramebufferStatus(target: u32) -> u32;
        fn glViewport(x: i32, y: i32, width: i32, height: i32);
        fn glClearColor(r: f32, g: f32, b: f32, a: f32);
        fn glClear(mask: u32);
        fn glEnable(what: u32);
        fn glDisable(what: u32);
        fn glBlendFunc(source: u32, destination: u32);
        fn glDrawArraysInstanced(mode: u32, first: i32, count: i32, instances: i32);
        fn glReadPixels(x: i32, y: i32, width: i32, height: i32, format: u32, kind: u32, pixels: *mut c_void);
        fn glFinish();
    }
}

pub const GL_VERTEX_SHADER: u32 = 0x8B31;
pub const GL_FRAGMENT_SHADER: u32 = 0x8B30;
pub const GL_COMPILE_STATUS: u32 = 0x8B81;
pub const GL_LINK_STATUS: u32 = 0x8B82;
pub const GL_ARRAY_BUFFER: u32 = 0x8892;
pub const GL_STREAM_DRAW: u32 = 0x88E0;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_TRIANGLE_STRIP: u32 = 0x0005;
pub const GL_TEXTURE_2D: u32 = 0x0DE1;
pub const GL_TEXTURE0: u32 = 0x84C0;
pub const GL_TEXTURE1: u32 = 0x84C1;
pub const GL_R8: u32 = 0x8229;
pub const GL_RED: u32 = 0x1903;
pub const GL_RGBA8: u32 = 0x8058;
pub const GL_RGBA: u32 = 0x1908;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const GL_TEXTURE_WRAP_S: u32 = 0x2802;
pub const GL_TEXTURE_WRAP_T: u32 = 0x2803;
pub const GL_NEAREST: u32 = 0x2600;
pub const GL_CLAMP_TO_EDGE: u32 = 0x812F;
pub const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
pub const GL_PACK_ALIGNMENT: u32 = 0x0D05;
pub const GL_FRAMEBUFFER: u32 = 0x8D40;
pub const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
pub const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
pub const GL_BLEND: u32 = 0x0BE2;
pub const GL_ONE: u32 = 1;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
pub const GL_VERSION: u32 = 0x1F02;
pub const GL_RENDERER: u32 = 0x1F01;
pub const GL_DITHER: u32 = 0x0BD0;
pub const GL_MAX_TEXTURE_SIZE: u32 = 0x0D33;
