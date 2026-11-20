//! The GPU renderer (toolkit milestone 1, step 3).
//!
//! The same scene, prepared the same way as for the CPU renderer (cpu.rs:
//! positions in pixels, clips resolved, glyphs from the text cache), drawn
//! by the graphics card: every prepared shape becomes one **instance**, a
//! rectangle of pixels with the numbers that describe the shape, and a
//! single fragment program works out how much of each pixel the shape
//! covers with the same arithmetic as the CPU renderer. Glyphs come from an
//! **atlas**, one texture holding every glyph bitmap in use; a glyph is
//! uploaded once and tinted when drawn. Shapes are drawn in the scene's
//! order in as few draw calls as there are pictures to switch between.
//!
//! It draws into a texture and can read the pixels back, with no window,
//! so that its output can be compared with the CPU renderer's, which is how
//! it is tested (tests/gpu.rs). For a window it draws into a [`GpuBuffer`]:
//! memory on the GPU that the display server can show as it is, handed over
//! as a file descriptor (a dmabuf), with no copy through main memory.
//!
//! The interface is OpenGL ES 3 through EGL (decided 2026-11-09), on the
//! GPU's render node through GBM. Where there is no GPU device, EGL's
//! "surfaceless" platform is tried, which on Mesa is a software GL: slow,
//! but it lets the same tests run on a machine without a GPU. A machine
//! with none of this gets an error from [`Gpu::new`] and draws with the CPU
//! renderer.
//!
//! Not yet handled here: a shape inside more than one rounded or off-pixel
//! clip at once (the CPU renderer does it; this one says so and refuses).

use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, c_char, c_void},
    fmt,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::Arc,
};

use vergence_scene::{CELL_GLYPH, CELL_UNDERLINE, Cell, Color, Command, Rect, Scene};
use vergence_text::{Bitmap, Fonts};

use crate::{
    Pixmap, RenderError,
    cpu::{Order, Prepared, RRect, Shape, intersect, prepare},
    gl::{self, Egl, Gbm, Gles},
    pixel_size,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    /// The system has no EGL or OpenGL ES library.
    NoLibrary,
    /// No GPU device, and no software GL either.
    NoDisplay,
    /// EGL refused (what it was asked, and its error code).
    Egl(&'static str, i32),
    /// The drawing program did not compile or link.
    Program(String),
    /// OpenGL reported an error while drawing.
    Gl(u32),
    /// Something this renderer does not do yet.
    Unsupported(&'static str),
    Render(RenderError),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoLibrary => write!(f, "no EGL or OpenGL ES library"),
            GpuError::NoDisplay => write!(f, "no GPU device and no software GL"),
            GpuError::Egl(what, code) => write!(f, "EGL: {what} failed ({code:#x})"),
            GpuError::Program(log) => write!(f, "drawing program: {log}"),
            GpuError::Gl(code) => write!(f, "OpenGL error {code:#x}"),
            GpuError::Unsupported(what) => write!(f, "not drawn on the GPU yet: {what}"),
            GpuError::Render(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<RenderError> for GpuError {
    fn from(e: RenderError) -> Self {
        GpuError::Render(e)
    }
}

/// Floats in one instance: eight attributes of four.
const FLOATS: usize = 32;

const KIND_FILL: f32 = 0.0;
const KIND_STROKE: f32 = 1.0;
const KIND_LINE: f32 = 2.0;
const KIND_GLYPH: f32 = 3.0;
const KIND_IMAGE: f32 = 4.0;

const VERTEX: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec4 a_rect;
layout(location = 1) in vec4 a_shape;
layout(location = 2) in vec4 a_inner;
layout(location = 3) in vec4 a_param;
layout(location = 4) in vec4 a_color;
layout(location = 5) in vec4 a_uv;
layout(location = 6) in vec4 a_clip;
layout(location = 7) in vec4 a_clip_param;
uniform vec2 u_size;
flat out vec4 v_shape;
flat out vec4 v_inner;
flat out vec4 v_param;
flat out vec4 v_color;
flat out vec4 v_uv;
flat out vec4 v_clip;
flat out vec4 v_clip_param;
void main() {
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    vec2 p = mix(a_rect.xy, a_rect.zw, corner);
    // Pixel row 0 goes to the framebuffer's first row, so what is read back starts at the top.
    gl_Position = vec4(p / u_size * 2.0 - 1.0, 0.0, 1.0);
    v_shape = a_shape;
    v_inner = a_inner;
    v_param = a_param;
    v_color = a_color;
    v_uv = a_uv;
    v_clip = a_clip;
    v_clip_param = a_clip_param;
}
"#;

// The coverage arithmetic is cpu.rs's, line for line: edge() and RRect::cover().
const FRAGMENT: &str = r#"#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D u_atlas;
uniform highp sampler2D u_image;
flat in vec4 v_shape;
flat in vec4 v_inner;
flat in vec4 v_param;
flat in vec4 v_color;
flat in vec4 v_uv;
flat in vec4 v_clip;
flat in vec4 v_clip_param;
out vec4 o_color;
//_TEXT_LOOK_

float edge(float lo, float hi, float p) {
    return clamp(min(p + 1.0, hi) - max(p, lo), 0.0, 1.0);
}

float cover(vec4 r, float radius, vec2 p) {
    bool row = radius > 0.0 && (p.y < r.y + radius || p.y + 1.0 > r.w - radius);
    bool col = radius > 0.0 && (p.x < r.x + radius || p.x + 1.0 > r.z - radius);
    if (!(row && col)) {
        return edge(r.x, r.z, p.x) * edge(r.y, r.w, p.y);
    }
    vec2 center = (r.xy + r.zw) * 0.5;
    vec2 q = abs(p + 0.5 - center) - ((r.zw - r.xy) * 0.5 - radius);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    return clamp(0.5 - d, 0.0, 1.0);
}

void main() {
    vec2 p = floor(gl_FragCoord.xy);
    int kind = int(v_param.x);
    float c = 0.0;
    vec4 color = v_color;
    if (kind == 0) {
        c = cover(v_shape, v_param.y, p);
    } else if (kind == 1) {
        c = cover(v_shape, v_param.y, p);
        if (v_param.w > 0.5) {
            c -= cover(v_inner, v_param.z, p);
        }
    } else if (kind == 2) {
        vec2 a = v_shape.xy;
        vec2 d = v_shape.zw - a;
        vec2 q = p + 0.5 - a;
        float len2 = dot(d, d);
        float t = len2 > 0.0 ? clamp(dot(q, d) / len2, 0.0, 1.0) : 0.0;
        c = 0.5 - (length(q - d * t) - v_param.y);
    } else if (kind == 3) {
        c = text_cover(texelFetch(u_atlas, ivec2(p - v_uv.zw + v_uv.xy), 0).r, color.rgb);
    } else {
        // A picture stretched to v_shape: between four of its pixels, mixed
        // with alpha multiplied in so a clear one's color does not bleed.
        vec2 size = v_param.zw;
        vec2 uv = clamp((p + 0.5 - v_shape.xy) / (v_shape.zw - v_shape.xy) * size - 0.5, vec2(0.0), size - 1.0);
        ivec2 t0 = ivec2(floor(uv));
        ivec2 t1 = min(t0 + 1, ivec2(size) - 1);
        vec2 f = uv - floor(uv);
        vec4 mixed = vec4(0.0);
        vec4 t;
        float w;
        t = texelFetch(u_image, t0, 0); w = t.a * (1.0 - f.x) * (1.0 - f.y); mixed += vec4(t.rgb * w, w);
        t = texelFetch(u_image, ivec2(t1.x, t0.y), 0); w = t.a * f.x * (1.0 - f.y); mixed += vec4(t.rgb * w, w);
        t = texelFetch(u_image, ivec2(t0.x, t1.y), 0); w = t.a * (1.0 - f.x) * f.y; mixed += vec4(t.rgb * w, w);
        t = texelFetch(u_image, t1, 0); w = t.a * f.x * f.y; mixed += vec4(t.rgb * w, w);
        c = cover(v_shape, 0.0, p) * v_param.y;
        if (mixed.a <= 0.0) {
            discard;
        }
        color = vec4(mixed.rgb / mixed.a, mixed.a);
    }
    c = clamp(c, 0.0, 1.0);
    if (v_clip_param.y > 0.5) {
        c *= cover(v_clip, v_clip_param.x, p);
    }
    float a = color.a * c;
    if (a <= 0.0) {
        discard;
    }
    o_color = vec4(color.rgb * a, a);
}
"#;

/// One texture of glyph coverage, filled in rows.
struct Atlas {
    texture: u32,
    size: i32,
    /// Where the next glyph goes, and the height of the row being filled.
    x: i32,
    y: i32,
    row: i32,
    /// A bitmap (by its address, kept alive here so the address stays its own) and its place.
    places: HashMap<usize, (i32, i32), std::hash::BuildHasherDefault<AddressHasher>>,
    held: Vec<Arc<Bitmap>>,
}

/// The hash of the atlas's keys: one multiplication. They are addresses of
/// bitmaps this process holds, which no scene can choose, and the atlas is
/// asked once for every glyph of every frame.
#[derive(Default)]
struct AddressHasher(u64);

impl std::hash::Hasher for AddressHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_usize(self.0 as usize ^ *b as usize);
        }
    }

    fn write_usize(&mut self, n: usize) {
        let h = (n as u64 ^ self.0.rotate_left(29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 = h ^ (h >> 32);
    }
}

impl Atlas {
    fn clear(&mut self) {
        (self.x, self.y, self.row) = (0, 0, 0);
        self.places.clear();
        self.held.clear();
    }

    /// The glyph's place in the atlas, uploading it if it is new. None: no room left.
    fn place(&mut self, gl: &Gles, bitmap: &Arc<Bitmap>) -> Option<(i32, i32)> {
        let key = Arc::as_ptr(bitmap) as usize;
        if let Some(at) = self.places.get(&key) {
            return Some(*at);
        }
        let (w, h) = (bitmap.width as i32, bitmap.height as i32);
        if w > self.size || h > self.size {
            return None;
        }
        if self.x + w > self.size {
            (self.x, self.y, self.row) = (0, self.y + self.row + 1, 0);
        }
        if self.y + h > self.size {
            return None;
        }
        let at = (self.x, self.y);
        // SAFETY: the bitmap holds width x height bytes, and rows are read one byte apart.
        unsafe {
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, self.texture);
            (gl.glPixelStorei)(gl::GL_UNPACK_ALIGNMENT, 1);
            (gl.glTexSubImage2D)(gl::GL_TEXTURE_2D, 0, at.0, at.1, w, h, gl::GL_RED, gl::GL_UNSIGNED_BYTE, bitmap.alpha.as_ptr() as *const c_void);
        }
        self.x += w + 1;
        self.row = self.row.max(h);
        self.places.insert(key, at);
        self.held.push(bitmap.clone());
        Some(at)
    }
}

/// Instances to draw in one call, and the picture they need bound, if any.
struct Batch {
    first: usize,
    count: usize,
    image: Option<usize>,
}

/// Memory on the GPU that a window's frame is drawn into and the display
/// server shows: what to tell the server about it is public, the rest is
/// the renderer's. Made by [`Gpu::buffer`], given back with [`Gpu::release`].
pub struct GpuBuffer {
    pub width: u32,
    pub height: u32,
    /// The buffer as a file descriptor (a dmabuf), to send to the server.
    pub fd: OwnedFd,
    /// Bytes a row, and where the first row starts.
    pub stride: u32,
    pub offset: u32,
    /// The pixel format (a fourcc code) and the driver's layout of the memory.
    pub format: u32,
    pub modifier: u64,
    object: *mut c_void,
    image: *mut c_void,
    texture: u32,
    framebuffer: u32,
}

pub struct Gpu {
    egl: Egl,
    gl: Gles,
    /// The GPU as a device, where there is one (not with a software GL).
    gbm: Option<(Gbm, *mut c_void)>,
    /// glEGLImageTargetTexture2DOES, where the GL has it.
    image_target: Option<unsafe extern "C" fn(u32, *mut c_void)>,
    display: *mut c_void,
    context: *mut c_void,
    /// What is drawing: the GL's own description.
    description: String,
    program: u32,
    vertex_array: u32,
    buffer: u32,
    size_uniform: i32,
    atlas: Atlas,
    /// The texture drawn into and its framebuffer, at the last size used.
    target: Option<(u32, u32, u32, u32)>,
    /// A one-pixel texture, bound where no picture is needed.
    blank: u32,
    /// Glyph coverage by the ink (textlook.rs `curves`), on texture unit 2.
    curves: u32,
    /// The pictures of the last frame, each as a texture, by where its
    /// pixels are. Held (so the address cannot be another picture's) until
    /// a frame goes by without it: a program that shows the same picture
    /// frame after frame has it sent to the GPU once.
    pictures: HashMap<usize, (std::sync::Arc<vergence_scene::Image>, u32)>,
}

impl Gpu {
    /// A renderer on this machine's GPU (or its software GL). An error if
    /// there is neither; draw with [`crate::Cpu`] then.
    pub fn new() -> Result<Gpu, GpuError> {
        let egl = Egl::load().ok_or(GpuError::NoLibrary)?;
        let gl = Gles::load().ok_or(GpuError::NoLibrary)?;
        let (display, gbm) = open_display(&egl).ok_or(GpuError::NoDisplay)?;
        // SAFETY: a zero-terminated name; the function, if there is one, has this signature.
        let image_target = unsafe {
            let p = (egl.eglGetProcAddress)(c"glEGLImageTargetTexture2DOES".as_ptr());
            (!p.is_null()).then(|| std::mem::transmute::<*mut c_void, unsafe extern "C" fn(u32, *mut c_void)>(p))
        };
        // SAFETY: EGL calls on a display that initialized; every result is checked.
        let context = unsafe {
            if (egl.eglBindAPI)(gl::EGL_OPENGL_ES_API) == 0 {
                return Err(GpuError::Egl("eglBindAPI", (egl.eglGetError)()));
            }
            let mut config: *mut c_void = std::ptr::null_mut();
            let mut count = 0;
            let wanted = [gl::EGL_RENDERABLE_TYPE, gl::EGL_OPENGL_ES3_BIT, gl::EGL_NONE];
            // No config at all is fine where the context needs none (drawing goes to a texture).
            if (egl.eglChooseConfig)(display, wanted.as_ptr(), &mut config, 1, &mut count) == 0 || count < 1 {
                config = std::ptr::null_mut();
            }
            let version = [gl::EGL_CONTEXT_MAJOR_VERSION, 3, gl::EGL_NONE];
            let context = (egl.eglCreateContext)(display, config, std::ptr::null_mut(), version.as_ptr());
            if context.is_null() {
                return Err(GpuError::Egl("eglCreateContext", (egl.eglGetError)()));
            }
            if (egl.eglMakeCurrent)(display, std::ptr::null_mut(), std::ptr::null_mut(), context) == 0 {
                let code = (egl.eglGetError)();
                (egl.eglDestroyContext)(display, context);
                return Err(GpuError::Egl("eglMakeCurrent", code));
            }
            context
        };
        // SAFETY: a context is current on this thread; GL calls with valid arguments.
        unsafe {
            let text = |name: u32| {
                let p = (gl.glGetString)(name);
                if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
            };
            let description = format!("{} ({})", text(gl::GL_RENDERER), text(gl::GL_VERSION));
            let program = match link(&gl) {
                Ok(p) => p,
                Err(e) => {
                    (egl.eglMakeCurrent)(display, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
                    (egl.eglDestroyContext)(display, context);
                    return Err(e);
                }
            };
            (gl.glUseProgram)(program);
            (gl.glUniform1i)((gl.glGetUniformLocation)(program, c"u_atlas".as_ptr()), 0);
            (gl.glUniform1i)((gl.glGetUniformLocation)(program, c"u_image".as_ptr()), 1);
            // Glyph coverage by the ink (textlook.rs): the CPU's tables, as a texture on unit 2.
            (gl.glUniform1i)((gl.glGetUniformLocation)(program, c"u_curves".as_ptr()), 2);
            let size_uniform = (gl.glGetUniformLocation)(program, c"u_size".as_ptr());

            let (mut vertex_array, mut buffer) = (0, 0);
            (gl.glGenVertexArrays)(1, &mut vertex_array);
            (gl.glBindVertexArray)(vertex_array);
            (gl.glGenBuffers)(1, &mut buffer);
            (gl.glBindBuffer)(gl::GL_ARRAY_BUFFER, buffer);
            for i in 0..8 {
                (gl.glEnableVertexAttribArray)(i);
                (gl.glVertexAttribDivisor)(i, 1);
            }

            let mut limit = 0;
            (gl.glGetIntegerv)(gl::GL_MAX_TEXTURE_SIZE, &mut limit);
            let size = limit.clamp(256, 4096);
            let texture = |internal: u32, format: u32, w: i32, h: i32, pixels: *const c_void| {
                let mut t = 0;
                (gl.glGenTextures)(1, &mut t);
                (gl.glBindTexture)(gl::GL_TEXTURE_2D, t);
                for (name, value) in [(gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST), (gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST), (gl::GL_TEXTURE_WRAP_S, gl::GL_CLAMP_TO_EDGE), (gl::GL_TEXTURE_WRAP_T, gl::GL_CLAMP_TO_EDGE)] {
                    (gl.glTexParameteri)(gl::GL_TEXTURE_2D, name, value as i32);
                }
                (gl.glTexImage2D)(gl::GL_TEXTURE_2D, 0, internal as i32, w, h, 0, format, gl::GL_UNSIGNED_BYTE, pixels);
                t
            };
            let atlas_texture = texture(gl::GL_R8, gl::GL_RED, size, size, std::ptr::null());
            let blank = texture(gl::GL_RGBA8, gl::GL_RGBA, 1, 1, [0u8; 4].as_ptr() as *const c_void);
            let (gamma, contrast) = crate::textlook::look();
            let curves = texture(gl::GL_R8, gl::GL_RED, 256, 256, crate::textlook::curves(gamma, contrast).as_ptr() as *const c_void);
            let error = (gl.glGetError)();
            if error != 0 {
                return Err(GpuError::Gl(error));
            }
            Ok(Gpu {
                egl,
                gl,
                gbm,
                image_target,
                display,
                context,
                description,
                program,
                vertex_array,
                buffer,
                size_uniform,
                atlas: Atlas { texture: atlas_texture, size, x: 0, y: 0, row: 0, places: HashMap::default(), held: Vec::new() },
                target: None,
                blank,
                curves,
                pictures: HashMap::default(),
            })
        }
    }

    /// What is doing the drawing ("Mesa Intel(R) UHD Graphics 630 ...", or a software GL's name).
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The scene at `scale` pixels to the reference unit, over `background`,
    /// read back from the GPU: the picture [`crate::Cpu::render`] gives,
    /// within a rounding step.
    pub fn render(&mut self, scene: &Scene, fonts: &mut Fonts, scale: f32, background: Color) -> Result<Pixmap, GpuError> {
        self.frame(scene, fonts, scale, background, true, None)
    }

    /// Draws the scene on the GPU and waits until it is done, without
    /// reading the pixels back (which costs more than drawing them): what a
    /// window does, and what timings of the drawing itself should use.
    pub fn draw(&mut self, scene: &Scene, fonts: &mut Fonts, scale: f32, background: Color) -> Result<(), GpuError> {
        self.frame(scene, fonts, scale, background, false, None).map(|_| ())
    }

    /// True if this renderer can make buffers a display server can show
    /// ([`Gpu::buffer`]): a real GPU device, not a software GL.
    pub fn shares_buffers(&self) -> bool {
        self.gbm.is_some() && self.image_target.is_some()
    }

    /// A buffer of GPU memory, `width` x `height` pixels, to draw a window's
    /// frames into and hand to the display server.
    pub fn buffer(&mut self, width: u32, height: u32) -> Result<GpuBuffer, GpuError> {
        let (Some((gbm, device)), Some(image_target)) = (&self.gbm, self.image_target) else { return Err(GpuError::Unsupported("buffers a display server can show")) };
        if width == 0 || height == 0 || width > crate::MAX_SIDE || height > crate::MAX_SIDE {
            return Err(GpuError::Render(RenderError::TooLarge));
        }
        // SAFETY: a live device and display, a current context; every result is checked and what was made is released on failure.
        unsafe {
            if (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) == 0 {
                return Err(GpuError::Egl("eglMakeCurrent", (self.egl.eglGetError)()));
            }
            let object = (gbm.gbm_bo_create)(*device, width, height, gl::FORMAT_ARGB8888, gl::GBM_BO_USE_RENDERING);
            if object.is_null() {
                return Err(GpuError::Unsupported("a GPU buffer of this size"));
            }
            let raw = (gbm.gbm_bo_get_fd)(object);
            if raw < 0 {
                (gbm.gbm_bo_destroy)(object);
                return Err(GpuError::Unsupported("a GPU buffer as a file descriptor"));
            }
            let fd = OwnedFd::from_raw_fd(raw);
            let (stride, offset, modifier) = ((gbm.gbm_bo_get_stride)(object), (gbm.gbm_bo_get_offset)(object, 0), (gbm.gbm_bo_get_modifier)(object));
            let mut attributes: Vec<isize> = vec![
                gl::EGL_WIDTH,
                width as isize,
                gl::EGL_HEIGHT,
                height as isize,
                gl::EGL_LINUX_DRM_FOURCC,
                gl::FORMAT_ARGB8888 as isize,
                gl::EGL_DMA_BUF_PLANE0_FD,
                fd.as_raw_fd() as isize,
                gl::EGL_DMA_BUF_PLANE0_OFFSET,
                offset as isize,
                gl::EGL_DMA_BUF_PLANE0_PITCH,
                stride as isize,
            ];
            if modifier != gl::MODIFIER_INVALID {
                attributes.extend_from_slice(&[gl::EGL_DMA_BUF_PLANE0_MODIFIER_LO, (modifier & 0xffff_ffff) as isize, gl::EGL_DMA_BUF_PLANE0_MODIFIER_HI, (modifier >> 32) as isize]);
            }
            attributes.push(gl::EGL_NONE as isize);
            let image = (self.egl.eglCreateImage)(self.display, std::ptr::null_mut(), gl::EGL_LINUX_DMA_BUF, std::ptr::null_mut(), attributes.as_ptr());
            if image.is_null() {
                let code = (self.egl.eglGetError)();
                (gbm.gbm_bo_destroy)(object);
                return Err(GpuError::Egl("eglCreateImage", code));
            }
            let gl = &self.gl;
            let (mut texture, mut framebuffer) = (0, 0);
            (gl.glGenTextures)(1, &mut texture);
            (gl.glActiveTexture)(gl::GL_TEXTURE1);
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, texture);
            (gl.glTexParameteri)(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST as i32);
            (gl.glTexParameteri)(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
            image_target(gl::GL_TEXTURE_2D, image);
            (gl.glGenFramebuffers)(1, &mut framebuffer);
            (gl.glBindFramebuffer)(gl::GL_FRAMEBUFFER, framebuffer);
            (gl.glFramebufferTexture2D)(gl::GL_FRAMEBUFFER, gl::GL_COLOR_ATTACHMENT0, gl::GL_TEXTURE_2D, texture, 0);
            let complete = (gl.glCheckFramebufferStatus)(gl::GL_FRAMEBUFFER) == gl::GL_FRAMEBUFFER_COMPLETE;
            let error = (gl.glGetError)();
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, self.blank);
            let buffer = GpuBuffer { width, height, fd, stride, offset, format: gl::FORMAT_ARGB8888, modifier, object, image, texture, framebuffer };
            if !complete || error != 0 {
                self.release(buffer);
                return Err(if error != 0 { GpuError::Gl(error) } else { GpuError::Unsupported("drawing into a GPU buffer") });
            }
            Ok(buffer)
        }
    }

    /// Gives a buffer back. The display server keeps its own hold on the
    /// memory for as long as it shows it.
    pub fn release(&mut self, buffer: GpuBuffer) {
        // SAFETY: the objects made in buffer(), released once, in this renderer's context.
        unsafe {
            if (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) != 0 {
                (self.gl.glDeleteFramebuffers)(1, &buffer.framebuffer);
                (self.gl.glDeleteTextures)(1, &buffer.texture);
            }
            (self.egl.eglDestroyImage)(self.display, buffer.image);
            if let Some((gbm, _)) = &self.gbm {
                (gbm.gbm_bo_destroy)(buffer.object);
            }
        }
    }

    /// Draws the scene into a buffer of its size at this scale, and waits
    /// until the GPU has finished: the buffer can then be shown.
    pub fn draw_into(&mut self, buffer: &GpuBuffer, scene: &Scene, fonts: &mut Fonts, scale: f32, background: Color) -> Result<(), GpuError> {
        if pixel_size(scene, scale)? != (buffer.width, buffer.height) {
            return Err(GpuError::Render(RenderError::Size));
        }
        self.frame(scene, fonts, scale, background, false, Some(buffer.framebuffer)).map(|_| ())
    }

    /// The pixels of a buffer, read back (for tests).
    pub fn read(&mut self, buffer: &GpuBuffer) -> Result<Pixmap, GpuError> {
        let mut pixmap = Pixmap::new(buffer.width, buffer.height);
        // SAFETY: a current context; the pixmap holds width x height x 4 bytes.
        unsafe {
            if (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) == 0 {
                return Err(GpuError::Egl("eglMakeCurrent", (self.egl.eglGetError)()));
            }
            (self.gl.glBindFramebuffer)(gl::GL_FRAMEBUFFER, buffer.framebuffer);
            (self.gl.glPixelStorei)(gl::GL_PACK_ALIGNMENT, 1);
            (self.gl.glReadPixels)(0, 0, buffer.width as i32, buffer.height as i32, gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, pixmap.data.as_mut_ptr() as *mut c_void);
            let error = (self.gl.glGetError)();
            if error != 0 {
                return Err(GpuError::Gl(error));
            }
        }
        Ok(pixmap)
    }

    /// The texture this renderer draws into when it is given no buffer, at this size.
    /// Runs a texture shader (the conventions of the GLES renderer the server
    /// composites with: `tex`, `alpha`, `v_coords`, a `//_DEFINES_` line) over
    /// the whole of a picture (8-bit RGBA, alpha multiplied in, rows from the
    /// top) and reads the result back, rows from the top: so the server's
    /// shaders ([`crate::blur`], [`crate::round`]) are tested on this GPU
    /// against their CPU arithmetic. `uniforms` are floats by name, one to
    /// four each.
    pub fn shader_pass(&mut self, fragment: &str, rgba: &[u8], w: u32, h: u32, uniforms: &[(&str, &[f32])]) -> Result<Pixmap, GpuError> {
        if w == 0 || h == 0 || rgba.len() != (w * h * 4) as usize {
            return Err(GpuError::Render(RenderError::Size));
        }
        const VERTEX_PASS: &str = "#version 100\nattribute vec2 pos;\nvarying vec2 v_coords;\nvoid main() {\n    v_coords = pos;\n    gl_Position = vec4(pos * 2.0 - 1.0, 0.0, 1.0);\n}\n";
        let fragment = fragment.replace("//_DEFINES_", "");
        // SAFETY: EGL and GL calls on this renderer's own context, made current
        // here; every pointer outlives its call; what is made is released.
        unsafe {
            if (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) == 0 {
                return Err(GpuError::Egl("eglMakeCurrent", (self.egl.eglGetError)()));
            }
            let target = self.own_target(w, h)?;
            let gl = &self.gl;
            let compile = |kind: u32, source: &str| -> Result<u32, GpuError> {
                let shader = (gl.glCreateShader)(kind);
                let (text, len) = (source.as_ptr() as *const c_char, source.len() as i32);
                (gl.glShaderSource)(shader, 1, &text, &len);
                (gl.glCompileShader)(shader);
                let mut ok = 0;
                (gl.glGetShaderiv)(shader, gl::GL_COMPILE_STATUS, &mut ok);
                if ok == 0 {
                    let mut log = [0 as c_char; 2048];
                    let mut n = 0;
                    (gl.glGetShaderInfoLog)(shader, log.len() as i32, &mut n, log.as_mut_ptr());
                    (gl.glDeleteShader)(shader);
                    return Err(GpuError::Program(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned()));
                }
                Ok(shader)
            };
            let vertex = compile(gl::GL_VERTEX_SHADER, VERTEX_PASS)?;
            let frag = match compile(gl::GL_FRAGMENT_SHADER, &fragment) {
                Ok(f) => f,
                Err(e) => {
                    (gl.glDeleteShader)(vertex);
                    return Err(e);
                }
            };
            let program = (gl.glCreateProgram)();
            (gl.glAttachShader)(program, vertex);
            (gl.glAttachShader)(program, frag);
            (gl.glBindAttribLocation)(program, 0, c"pos".as_ptr());
            (gl.glLinkProgram)(program);
            (gl.glDeleteShader)(vertex);
            (gl.glDeleteShader)(frag);
            let mut ok = 0;
            (gl.glGetProgramiv)(program, gl::GL_LINK_STATUS, &mut ok);
            if ok == 0 {
                (gl.glDeleteProgram)(program);
                return Err(GpuError::Program("the shader pass did not link".into()));
            }
            let mut texture = 0u32;
            (gl.glGenTextures)(1, &mut texture);
            (gl.glActiveTexture)(gl::GL_TEXTURE0);
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, texture);
            for (name, value) in [(gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST), (gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST), (gl::GL_TEXTURE_WRAP_S, gl::GL_CLAMP_TO_EDGE), (gl::GL_TEXTURE_WRAP_T, gl::GL_CLAMP_TO_EDGE)] {
                (gl.glTexParameteri)(gl::GL_TEXTURE_2D, name, value as i32);
            }
            (gl.glPixelStorei)(gl::GL_UNPACK_ALIGNMENT, 1);
            (gl.glTexImage2D)(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA8 as i32, w as i32, h as i32, 0, gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, rgba.as_ptr() as *const c_void);
            let quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let (mut vao, mut vbo) = (0u32, 0u32);
            (gl.glGenVertexArrays)(1, &mut vao);
            (gl.glBindVertexArray)(vao);
            (gl.glGenBuffers)(1, &mut vbo);
            (gl.glBindBuffer)(gl::GL_ARRAY_BUFFER, vbo);
            (gl.glBufferData)(gl::GL_ARRAY_BUFFER, std::mem::size_of_val(&quad) as isize, quad.as_ptr() as *const c_void, gl::GL_STREAM_DRAW);
            (gl.glEnableVertexAttribArray)(0);
            (gl.glVertexAttribPointer)(0, 2, gl::GL_FLOAT, 0, 8, std::ptr::null());
            (gl.glVertexAttribDivisor)(0, 0);
            (gl.glBindFramebuffer)(gl::GL_FRAMEBUFFER, target);
            (gl.glViewport)(0, 0, w as i32, h as i32);
            (gl.glDisable)(gl::GL_BLEND);
            (gl.glDisable)(gl::GL_DITHER);
            (gl.glUseProgram)(program);
            let at = |name: &str| {
                let c = std::ffi::CString::new(name).unwrap_or_default();
                (gl.glGetUniformLocation)(program, c.as_ptr())
            };
            (gl.glUniform1i)(at("tex"), 0);
            (gl.glUniform1f)(at("alpha"), 1.0);
            for (name, v) in uniforms {
                let l = at(name);
                match v.len() {
                    1 => (gl.glUniform1f)(l, v[0]),
                    2 => (gl.glUniform2f)(l, v[0], v[1]),
                    3 => (gl.glUniform3f)(l, v[0], v[1], v[2]),
                    _ => (gl.glUniform4f)(l, v[0], v[1], v[2], v[3]),
                }
            }
            (gl.glDrawArraysInstanced)(gl::GL_TRIANGLE_STRIP, 0, 4, 1);
            let mut pixmap = Pixmap::new(w, h);
            (gl.glPixelStorei)(gl::GL_PACK_ALIGNMENT, 1);
            (gl.glReadPixels)(0, 0, w as i32, h as i32, gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, pixmap.data.as_mut_ptr() as *mut c_void);
            (gl.glDeleteBuffers)(1, &vbo);
            (gl.glDeleteVertexArrays)(1, &vao);
            (gl.glDeleteTextures)(1, &texture);
            (gl.glDeleteProgram)(program);
            Ok(pixmap)
        }
    }

    fn own_target(&mut self, w: u32, h: u32) -> Result<u32, GpuError> {
        let gl = &self.gl;
        // SAFETY: a current context (the caller's); what is made is released on failure.
        unsafe {
            match self.target {
                Some((_, f, tw, th)) if (tw, th) == (w, h) => Ok(f),
                old => {
                    if let Some((t, f, ..)) = old {
                        (gl.glDeleteFramebuffers)(1, &f);
                        (gl.glDeleteTextures)(1, &t);
                    }
                    self.target = None;
                    let (mut t, mut f) = (0, 0);
                    (gl.glGenTextures)(1, &mut t);
                    (gl.glActiveTexture)(gl::GL_TEXTURE1);
                    (gl.glBindTexture)(gl::GL_TEXTURE_2D, t);
                    (gl.glTexParameteri)(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST as i32);
                    (gl.glTexParameteri)(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
                    (gl.glTexImage2D)(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA8 as i32, w as i32, h as i32, 0, gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, std::ptr::null());
                    (gl.glGenFramebuffers)(1, &mut f);
                    (gl.glBindFramebuffer)(gl::GL_FRAMEBUFFER, f);
                    (gl.glFramebufferTexture2D)(gl::GL_FRAMEBUFFER, gl::GL_COLOR_ATTACHMENT0, gl::GL_TEXTURE_2D, t, 0);
                    if (gl.glCheckFramebufferStatus)(gl::GL_FRAMEBUFFER) != gl::GL_FRAMEBUFFER_COMPLETE {
                        (gl.glDeleteFramebuffers)(1, &f);
                        (gl.glDeleteTextures)(1, &t);
                        return Err(GpuError::Unsupported("a framebuffer of this size"));
                    }
                    self.target = Some((t, f, w, h));
                    Ok(f)
                }
            }
        }
    }

    fn frame(&mut self, scene: &Scene, fonts: &mut Fonts, scale: f32, background: Color, read: bool, into: Option<u32>) -> Result<Pixmap, GpuError> {
        let (w, h) = pixel_size(scene, scale)?;
        scene.validate().map_err(RenderError::from)?;
        let prepared = prepare(scene, fonts, scale, w, h, Order::Rgba, false);
        // SAFETY: the context is made current on this thread for the calls that follow.
        unsafe {
            if (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) == 0 {
                return Err(GpuError::Egl("eglMakeCurrent", (self.egl.eglGetError)()));
            }
        }
        // Instances, in drawing order. An atlas that fills up is emptied and the frame begun again, once.
        let mut built = self.instances(&prepared, scene, fonts, scale);
        if matches!(built, Err(GpuError::Unsupported("atlas full"))) {
            self.atlas.clear();
            built = self.instances(&prepared, scene, fonts, scale);
        }
        let (mut data, batches) = built?;
        let sizes: Vec<(u32, u32)> = prepared.images.iter().map(|im| (im.width, im.height)).collect();
        Gpu::picture_sizes(&mut data, &batches, &sizes);

        let framebuffer = match into {
            Some(f) => f,
            None => self.own_target(w, h)?,
        };
        let gl = &self.gl;
        // SAFETY: a context is current; every pointer given to GL outlives the call it is given to.
        unsafe {
            (gl.glBindFramebuffer)(gl::GL_FRAMEBUFFER, framebuffer);
            (gl.glViewport)(0, 0, w as i32, h as i32);
            (gl.glDisable)(gl::GL_DITHER);
            let a = background.a as f32 / 255.0;
            (gl.glClearColor)(background.r as f32 / 255.0 * a, background.g as f32 / 255.0 * a, background.b as f32 / 255.0 * a, a);
            (gl.glClear)(gl::GL_COLOR_BUFFER_BIT);
            (gl.glEnable)(gl::GL_BLEND);
            // Colors leave the fragment program with alpha multiplied in.
            (gl.glBlendFunc)(gl::GL_ONE, gl::GL_ONE_MINUS_SRC_ALPHA);
            (gl.glUseProgram)(self.program);
            (gl.glUniform2f)(self.size_uniform, w as f32, h as f32);
            (gl.glBindVertexArray)(self.vertex_array);
            (gl.glBindBuffer)(gl::GL_ARRAY_BUFFER, self.buffer);
            (gl.glBufferData)(gl::GL_ARRAY_BUFFER, std::mem::size_of_val(data.as_slice()) as isize, data.as_ptr() as *const c_void, gl::GL_STREAM_DRAW);
            (gl.glActiveTexture)(gl::GL_TEXTURE0);
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, self.atlas.texture);
            (gl.glActiveTexture)(gl::GL_TEXTURE0 + 2);
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, self.curves);
            (gl.glActiveTexture)(gl::GL_TEXTURE1);
            (gl.glBindTexture)(gl::GL_TEXTURE_2D, self.blank);
            let stride = (FLOATS * 4) as i32;
            let mut used: HashSet<usize> = HashSet::default();
            for batch in &batches {
                if let Some(index) = batch.image {
                    let im = &prepared.images[index];
                    let key = std::sync::Arc::as_ptr(im) as usize;
                    used.insert(key);
                    match self.pictures.get(&key) {
                        Some((_, texture)) => (gl.glBindTexture)(gl::GL_TEXTURE_2D, *texture),
                        None => {
                            let mut texture = 0u32;
                            (gl.glGenTextures)(1, &mut texture);
                            (gl.glBindTexture)(gl::GL_TEXTURE_2D, texture);
                            (gl.glTexParameteri)(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MIN_FILTER, gl::GL_NEAREST as i32);
                            (gl.glTexParameteri)(gl::GL_TEXTURE_2D, gl::GL_TEXTURE_MAG_FILTER, gl::GL_NEAREST as i32);
                            (gl.glPixelStorei)(gl::GL_UNPACK_ALIGNMENT, 1);
                            (gl.glTexImage2D)(gl::GL_TEXTURE_2D, 0, gl::GL_RGBA8 as i32, im.width as i32, im.height as i32, 0, gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, im.rgba.as_ptr() as *const c_void);
                            self.pictures.insert(key, (im.clone(), texture));
                        }
                    }
                }
                // ES 3.0 has no "first instance": the attributes are pointed at the batch's first.
                let base = batch.first * FLOATS * 4;
                for i in 0..8usize {
                    (gl.glVertexAttribPointer)(i as u32, 4, gl::GL_FLOAT, 0, stride, (base + i * 16) as *const c_void);
                }
                (gl.glDrawArraysInstanced)(gl::GL_TRIANGLE_STRIP, 0, 4, batch.count as i32);
            }
            let mut pixmap = Pixmap::new(if read { w } else { 0 }, if read { h } else { 0 });
            if read {
                (gl.glPixelStorei)(gl::GL_PACK_ALIGNMENT, 1);
                (gl.glReadPixels)(0, 0, w as i32, h as i32, gl::GL_RGBA, gl::GL_UNSIGNED_BYTE, pixmap.data.as_mut_ptr() as *mut c_void);
            } else {
                (gl.glFinish)();
            }
            // Pictures this frame did not show are let go.
            self.pictures.retain(|key, (_, texture)| {
                let keep = used.contains(key);
                if !keep {
                    (gl.glDeleteTextures)(1, texture);
                }
                keep
            });
            let error = (gl.glGetError)();
            if error != 0 {
                return Err(GpuError::Gl(error));
            }
            Ok(pixmap)
        }
    }

    /// The prepared shapes as instance data, and the draw calls to make of them.
    fn instances(&mut self, prepared: &Prepared, scene: &Scene, fonts: &mut Fonts, scale: f32) -> Result<(Vec<f32>, Vec<Batch>), GpuError> {
        let (prims, chains) = (&prepared.prims, &prepared.chains);
        let cells: usize = scene.commands.iter().map(|c| if let Command::Cells { cells, .. } = c { cells.len() } else { 0 }).sum();
        let mut data: Vec<f32> = Vec::with_capacity((prims.len() + 2 * cells) * FLOATS);
        let mut batches: Vec<Batch> = Vec::new();
        let rect = |r: &RRect| [r.x0, r.y0, r.x1, r.y1];
        // One instance, put together and then added whole: the list is grown and
        // checked once for it, not once for each of its eight parts.
        fn emit(data: &mut Vec<f32>, batches: &mut Vec<Batch>, parts: [[f32; 4]; 8], image: Option<usize>) {
            let first = data.len() / FLOATS;
            let mut instance = [0.0f32; FLOATS];
            for (to, part) in instance.chunks_exact_mut(4).zip(parts) {
                to.copy_from_slice(&part);
            }
            data.extend_from_slice(&instance);
            match (image, batches.last_mut()) {
                (None, Some(b)) if b.image.is_none() => b.count += 1,
                _ => batches.push(Batch { first, count: 1, image }),
            }
        }
        let color = |c: [u8; 4]| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0];
        let whole = |b: [i32; 4]| [b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32];
        for p in prims.iter() {
            let chain = &chains[p.chain as usize];
            let (clip, clip_param) = match chain.as_slice() {
                [] => ([0.0; 4], [0.0; 4]),
                [one] => (rect(one), [one.radius, 1.0, 0.0, 0.0]),
                // Several: as one, where that is exact (one_clip). Before, such
                // a frame was drawn in software and lost the frost under its
                // window's bar (techbench's Settings at 133⅓%, 2026-11-18).
                many => match one_clip(many, p.bounds) {
                    Some(one) => (rect(&one), [one.radius, 1.0, 0.0, 0.0]),
                    None => return Err(GpuError::Unsupported("a shape inside more than one rounded clip")),
                },
            };
            let (mut shape, mut inner, mut uv) = ([0.0f32; 4], [0.0f32; 4], [0.0f32; 4]);
            let mut param: [f32; 4];
            let mut image = None;
            match &p.shape {
                Shape::Fill(r) => {
                    shape = rect(r);
                    param = [KIND_FILL, r.radius, 0.0, 0.0];
                }
                Shape::Stroke(outer, hole) => {
                    shape = rect(outer);
                    param = [KIND_STROKE, outer.radius, 0.0, 0.0];
                    if let Some(h) = hole {
                        inner = rect(h);
                        (param[2], param[3]) = (h.radius, 1.0);
                    }
                }
                Shape::Line { ax, ay, bx, by, half } => {
                    shape = [*ax, *ay, *bx, *by];
                    param = [KIND_LINE, *half, 0.0, 0.0];
                }
                Shape::Glyph { x, y, bitmap, .. } => {
                    let at = self.atlas.place(&self.gl, bitmap).ok_or(GpuError::Unsupported("atlas full"))?;
                    uv = [at.0 as f32, at.1 as f32, *x as f32, *y as f32];
                    param = [KIND_GLYPH, 0.0, 0.0, 0.0];
                }
                Shape::Image { to, image: index, opacity } => {
                    shape = rect(to);
                    image = Some(*index);
                    // The picture's size in pixels is filled in from the scene when it is bound.
                    param = [KIND_IMAGE, *opacity, 0.0, 0.0];
                }
                Shape::Cells(at) => {
                    // A grid goes from its cells to instances with no shapes kept between:
                    // what the reference renderer's `prepare` makes of it, by the same
                    // arithmetic and in the same order, written straight into the list.
                    let Command::Cells { origin, cell_w, cell_h, baseline, size, columns, cells } = &scene.commands[*at] else { continue };
                    let (columns, scissor) = (*columns as usize, p.bounds);
                    let fill = |data: &mut Vec<f32>, batches: &mut Vec<Batch>, r: RRect, c: Color| {
                        let bounds = intersect(r.bounds(), scissor);
                        if bounds[0] < bounds[2] && bounds[1] < bounds[3] && c.a != 0 {
                            emit(data, batches, [whole(bounds), rect(&r), [0.0; 4], [KIND_FILL, r.radius, 0.0, 0.0], color([c.r, c.g, c.b, c.a]), [0.0; 4], clip, clip_param], None);
                        }
                    };
                    // Where each glyph of this grid is in the atlas, and its box: a screen
                    // of text asks for the same few hundred, thousands of times. A place
                    // is the glyph's, its font's and its quarter-pixel's; another glyph
                    // that maps there takes it.
                    const MEMO: usize = 1024;
                    let mut memo: Vec<Option<(u64, Option<[i32; 6]>)>> = vec![None; MEMO];
                    for (r, row) in cells.chunks_exact(columns).enumerate() {
                        let top = origin.y + r as f32 * cell_h;
                        let mut x = 0;
                        while x < columns {
                            let ground = row[x].ground;
                            let mut end = x + 1;
                            while end < columns && row[end].ground == ground {
                                end += 1;
                            }
                            if ground.a != 0 {
                                fill(&mut data, &mut batches, RRect::device(&Rect::new(origin.x + x as f32 * cell_w, top, (end - x) as f32 * cell_w, *cell_h), scale, 0.0), ground);
                            }
                            x = end;
                        }
                        let lined = |c: &Cell| c.flags & CELL_UNDERLINE != 0;
                        let mut x = 0;
                        while x < columns {
                            let mut end = x + 1;
                            if lined(&row[x]) {
                                while end < columns && lined(&row[end]) && row[end].ink == row[x].ink {
                                    end += 1;
                                }
                                fill(&mut data, &mut batches, RRect::device(&Rect::new(origin.x + x as f32 * cell_w, top + cell_h - 2.0, (end - x) as f32 * cell_w, 1.0), scale, 0.0), row[x].ink);
                            }
                            x = end;
                        }
                        let line = ((top + baseline) * scale).round() as i32;
                        for (c, cell) in row.iter().enumerate() {
                            if cell.flags & CELL_GLYPH == 0 || cell.ink.a == 0 {
                                continue;
                            }
                            let Some(face) = prepared.faces[cell.font as usize] else { continue };
                            let face = fonts.sized(face, *size);
                            let x = (origin.x + c as f32 * cell_w) * scale;
                            let (mut whole_x, mut sub) = (x.floor(), ((x - x.floor()) * 4.0).round() as u8);
                            if sub == 4 {
                                whole_x += 1.0;
                                sub = 0;
                            }
                            let key = cell.glyph as u64 | (cell.font as u64) << 32 | (sub as u64) << 48;
                            let slot = (cell.glyph as usize).wrapping_mul(31).wrapping_add(cell.font as usize * 7 + sub as usize) & (MEMO - 1);
                            let known = match memo[slot] {
                                Some((k, known)) if k == key => known,
                                _ => {
                                    let known = match fonts.raster(face, cell.glyph, size * scale, sub) {
                                        Some(bitmap) => {
                                            let at = self.atlas.place(&self.gl, &bitmap).ok_or(GpuError::Unsupported("atlas full"))?;
                                            Some([at.0, at.1, bitmap.left, bitmap.top, bitmap.width as i32, bitmap.height as i32])
                                        }
                                        None => None,
                                    };
                                    memo[slot] = Some((key, known));
                                    known
                                }
                            };
                            let Some([u, v, left, up, w, h]) = known else { continue };
                            let (px, py) = (whole_x as i32 + left, line + up);
                            let bounds = intersect([px, py, px + w, py + h], scissor);
                            if bounds[0] < bounds[2] && bounds[1] < bounds[3] {
                                let ink = cell.ink;
                                emit(&mut data, &mut batches, [whole(bounds), [0.0; 4], [0.0; 4], [KIND_GLYPH, 0.0, 0.0, 0.0], color([ink.r, ink.g, ink.b, ink.a]), [u as f32, v as f32, px as f32, py as f32], clip, clip_param], None);
                            }
                        }
                    }
                    continue;
                }
            }
            emit(&mut data, &mut batches, [whole(p.bounds), shape, inner, param, color(p.color), uv, clip, clip_param], image);
        }
        Ok((data, batches))
    }

    /// Fills in each picture's size, which the fragment program needs.
    fn picture_sizes(data: &mut [f32], batches: &[Batch], sizes: &[(u32, u32)]) {
        for b in batches {
            if let Some(index) = b.image {
                let at = b.first * FLOATS + 12;
                (data[at + 2], data[at + 3]) = (sizes[index].0 as f32, sizes[index].1 as f32);
            }
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: the objects this renderer made, deleted once in its own context, then the context.
        // The display (and a GPU device under it) is left for the process's lifetime: other
        // renderers in the process may share it.
        unsafe {
            if (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) != 0 {
                let gl = &self.gl;
                if let Some((t, f, ..)) = self.target {
                    (gl.glDeleteFramebuffers)(1, &f);
                    (gl.glDeleteTextures)(1, &t);
                }
                (gl.glDeleteTextures)(1, &self.atlas.texture);
                (gl.glDeleteTextures)(1, &self.blank);
                (gl.glDeleteTextures)(1, &self.curves);
                for (_, texture) in self.pictures.values() {
                    (gl.glDeleteTextures)(1, texture);
                }
                (gl.glDeleteBuffers)(1, &self.buffer);
                (gl.glDeleteVertexArrays)(1, &self.vertex_array);
                (gl.glDeleteProgram)(self.program);
                (gl.glFinish)();
                (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            }
            (self.egl.eglDestroyContext)(self.display, self.context);
        }
    }
}

/// An initialized EGL display: on the first GPU render node that works,
/// else Mesa's surfaceless platform.
#[allow(clippy::type_complexity)]
fn open_display(egl: &Egl) -> Option<(*mut c_void, Option<(Gbm, *mut c_void)>)> {
    use std::os::fd::IntoRawFd;
    let ready = |display: *mut c_void| {
        let (mut major, mut minor) = (0, 0);
        // SAFETY: a display EGL returned, or null, which is checked first.
        !display.is_null() && unsafe { (egl.eglInitialize)(display, &mut major, &mut minor) } != 0
    };
    if let Some(gbm) = Gbm::load() {
        for n in 128..136 {
            let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(format!("/dev/dri/renderD{n}")) else { continue };
            // The device keeps the descriptor for as long as the display lives (the process).
            let fd = file.into_raw_fd();
            // SAFETY: a descriptor we own; results are checked, and released if unused.
            unsafe {
                let device = (gbm.gbm_create_device)(fd);
                if device.is_null() {
                    drop(std::fs::File::from(<std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd)));
                    continue;
                }
                let display = (egl.eglGetPlatformDisplay)(gl::EGL_PLATFORM_GBM, device, std::ptr::null());
                if ready(display) {
                    return Some((display, Some((gbm, device))));
                }
                (gbm.gbm_device_destroy)(device);
                drop(std::fs::File::from(<std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd)));
            }
        }
    }
    // SAFETY: the platform's default display.
    let display = unsafe { (egl.eglGetPlatformDisplay)(gl::EGL_PLATFORM_SURFACELESS, std::ptr::null_mut(), std::ptr::null()) };
    ready(display).then_some((display, None))
}

/// Compiles and links the drawing program.
///
/// # Safety
/// A GL context must be current.
unsafe fn link(gl: &Gles) -> Result<u32, GpuError> {
    unsafe {
        let compile = |kind: u32, source: &str| -> Result<u32, GpuError> {
            let shader = (gl.glCreateShader)(kind);
            let (text, len) = (source.as_ptr() as *const c_char, source.len() as i32);
            (gl.glShaderSource)(shader, 1, &text, &len);
            (gl.glCompileShader)(shader);
            let mut ok = 0;
            (gl.glGetShaderiv)(shader, gl::GL_COMPILE_STATUS, &mut ok);
            if ok == 0 {
                let mut log = [0 as c_char; 2048];
                let mut n = 0;
                (gl.glGetShaderInfoLog)(shader, log.len() as i32, &mut n, log.as_mut_ptr());
                (gl.glDeleteShader)(shader);
                return Err(GpuError::Program(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned()));
            }
            Ok(shader)
        };
        let vertex = compile(gl::GL_VERTEX_SHADER, VERTEX)?;
        let fragment = match compile(gl::GL_FRAGMENT_SHADER, &FRAGMENT.replace("//_TEXT_LOOK_", crate::textlook::GLSL)) {
            Ok(f) => f,
            Err(e) => {
                (gl.glDeleteShader)(vertex);
                return Err(e);
            }
        };
        let program = (gl.glCreateProgram)();
        (gl.glAttachShader)(program, vertex);
        (gl.glAttachShader)(program, fragment);
        (gl.glLinkProgram)(program);
        (gl.glDeleteShader)(vertex);
        (gl.glDeleteShader)(fragment);
        let mut ok = 0;
        (gl.glGetProgramiv)(program, gl::GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut log = [0 as c_char; 2048];
            let mut n = 0;
            (gl.glGetProgramInfoLog)(program, log.len() as i32, &mut n, log.as_mut_ptr());
            (gl.glDeleteProgram)(program);
            return Err(GpuError::Program(CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned()));
        }
        Ok(program)
    }
}

/// Several clips a shape lies in, as the one the shader takes, where that is
/// exact for the pixels it can touch (`bounds`): a clip with no rounding, or
/// whose rounded corners those pixels don't reach, acts as a plain
/// rectangle, and rectangles meet in a rectangle. One truly rounded clip may
/// remain, if the rectangles leave it whole or the pixels inside them. None
/// when two rounded clips both matter there, or a rectangle cuts the one
/// that does.
pub(crate) fn one_clip(chain: &[RRect], bounds: [i32; 4]) -> Option<RRect> {
    let b = [bounds[0] as f32, bounds[1] as f32, bounds[2] as f32, bounds[3] as f32];
    let meets = |a: [f32; 4], c: [f32; 4]| a[0] < c[2] && c[0] < a[2] && a[1] < c[3] && c[1] < a[3];
    let reaches_corner = |c: &RRect| {
        let r = c.radius;
        r > 0.0
            && [
                [c.x0, c.y0, c.x0 + r, c.y0 + r],
                [c.x1 - r, c.y0, c.x1, c.y0 + r],
                [c.x0, c.y1 - r, c.x0 + r, c.y1],
                [c.x1 - r, c.y1 - r, c.x1, c.y1],
            ]
            .iter()
            .any(|q| meets(*q, b))
    };
    let mut square = [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::INFINITY];
    let mut rounded: Option<RRect> = None;
    for c in chain {
        if reaches_corner(c) {
            if rounded.is_some() {
                return None;
            }
            rounded = Some(*c);
        } else {
            square = [square[0].max(c.x0), square[1].max(c.y0), square[2].min(c.x1), square[3].min(c.y1)];
        }
    }
    let inside = |a: [f32; 4], c: [f32; 4]| a[0] >= c[0] && a[1] >= c[1] && a[2] <= c[2] && a[3] <= c[3];
    match rounded {
        None => Some(RRect { x0: square[0], y0: square[1], x1: square[2], y1: square[3], radius: 0.0 }),
        // The rectangles leave the rounded one whole, or the pixels lie inside them.
        Some(c) if inside([c.x0, c.y0, c.x1, c.y1], square) || inside(b, square) => Some(c),
        Some(_) => None,
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    fn rr(x0: f32, y0: f32, x1: f32, y1: f32, radius: f32) -> RRect {
        RRect { x0, y0, x1, y1, radius }
    }

    #[test]
    fn nested_clips_become_one_where_that_is_exact() {
        // A rounded card inside a rounded scroll area, its text well inside both:
        // the card is the clip (its corners are near), the area acts as a square.
        let area = rr(0.0, 0.0, 1000.0, 1600.0, 10.0);
        let card = rr(40.0, 100.0, 960.0, 400.0, 12.0);
        let c = one_clip(&[area, card], [50, 110, 300, 130]).unwrap();
        assert_eq!((c.x0, c.y0, c.x1, c.y1, c.radius), (40.0, 100.0, 960.0, 400.0, 12.0));
        // Pixels reaching no corner of either: a plain rectangle, both met.
        let c = one_clip(&[area, card], [400, 200, 500, 220]).unwrap();
        assert_eq!((c.x0, c.y0, c.x1, c.y1, c.radius), (40.0, 100.0, 960.0, 400.0, 0.0));
        // Square clips only: their meeting.
        let c = one_clip(&[rr(0.0, 0.0, 500.0, 500.0, 0.0), rr(100.0, 50.0, 800.0, 300.0, 0.0)], [0, 0, 900, 900]).unwrap();
        assert_eq!((c.x0, c.y0, c.x1, c.y1), (100.0, 50.0, 500.0, 300.0));
        // A shape in the corner of two rounded clips that share it: not one clip.
        let a = rr(0.0, 0.0, 100.0, 100.0, 10.0);
        let b = rr(0.0, 0.0, 200.0, 200.0, 20.0);
        assert!(one_clip(&[a, b], [0, 0, 15, 15]).is_none());
        // A rectangle cutting through the rounded clip that matters: not one clip.
        assert!(one_clip(&[a, rr(50.0, 0.0, 300.0, 300.0, 0.0)], [0, 0, 100, 100]).is_none());
    }
}
