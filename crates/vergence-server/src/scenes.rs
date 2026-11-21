// Windows drawn from their scenes (display lists milestone 1, step 2;
// docs/display-lists-milestone-1.md). A native window that sends a scene
// (bus v30 `Scene`, native.rs `take_scene`) has it drawn here by the same
// renderer the apps use (vergence-render: its own GPU context, our code),
// at the scale the window is shown at, into one of three GPU buffers the
// window takes in turn; the buffer then becomes the window's picture just
// as a GPU picture an app attached does (native.rs `native_attach_gpu`),
// so the bar's frost, the rounded corners, the overview and the dock's
// pictures all show it unchanged. Where the GPU renderer cannot start (a
// machine with no GPU, canary) or fails, the CPU reference renderer draws
// it into memory instead (VERGENCE_SCENES_CPU=1 asks for that, to test).
//
// Step 4: a window across screens of different scales. Its picture is
// drawn at its own screen's scale; on any other screen it overlaps, a
// window with a scene is drawn again from the scene at that screen's scale
// (`scene_alt`, kept until the scene or the scale changes, two buffers in
// turn), so each half is sharp instead of one stretched. On the CPU the
// other screens show the picture stretched, as before.
//
// A scene brings its own ground (the toolkit's first fill); it is drawn on
// clear. The whole scene is drawn each time, as the apps did; drawing only
// what changed comes later (the plan's step 2 hope, noted there).

use std::collections::HashMap;

use smithay::utils::{Buffer as BufferCoords, Size};
use vergence_render::{Cpu, Gpu, GpuBuffer};
use vergence_scene::Color;

use crate::{Server, native::NativeWindow};

/// How many buffers a window takes in turn (one shown, one waiting, one drawn).
const BUFFERS: usize = 3;

/// The scene renderer and each window's buffers.
#[derive(Default)]
pub struct Scenes {
    gpu: Option<Gpu>,
    /// The GPU renderer could not start or failed: the CPU from now on.
    no_gpu: bool,
    cpu: Cpu,
    /// Each window's buffers (by bus client and window) and the one next.
    buffers: HashMap<(u32, u32), (Vec<GpuBuffer>, usize)>,
    /// Scenes drawn, on the GPU and on the CPU, for vctl.
    pub drawn_gpu: u64,
    pub drawn_cpu: u64,
    /// Windows drawn again for another screen's scale: by window and that
    /// scale's bits, the scene drawn (where it is in memory), two buffers
    /// and the texture made from the last.
    alts: HashMap<(u32, u32, u64), AltEntry>,
    pub drawn_alt: u64,
}

struct AltEntry {
    scene: usize,
    bufs: Vec<GpuBuffer>,
    next: usize,
    alt: Option<Alt>,
}

/// A window's picture drawn again at another screen's scale.
#[derive(Clone)]
pub struct Alt {
    pub texture: smithay::backend::renderer::element::texture::TextureBuffer<smithay::backend::renderer::gles::GlesTexture>,
    pub gles: smithay::backend::renderer::gles::GlesTexture,
    pub px: Size<i32, BufferCoords>,
    /// Counted apart from the pictures' serials (backdrop.rs tells frames apart by it).
    pub serial: u64,
}

static ALT_SERIALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 48);

impl Server {
    /// Draws window `n`'s last scene at its scale and makes it its picture.
    pub fn native_draw_scene(&mut self, n: &NativeWindow) -> Result<(), String> {
        let Some(scene) = n.scene() else { return Ok(()) };
        let scale = n.scale().max(0.25) as f32;
        let (pw, ph) = vergence_render::pixel_size(&scene, scale).map_err(|e| e.to_string())?;
        let key = (n.client, n.id);
        // VERGENCE_SCENES_CPU=1: the CPU reference renderer only (tests, comparing).
        if std::env::var_os("VERGENCE_SCENES_CPU").is_some_and(|v| v == "1") {
            self.scenes.no_gpu = true;
        }
        if !self.scenes.no_gpu && self.scenes.gpu.is_none() {
            match Gpu::new() {
                Ok(g) => self.scenes.gpu = Some(g),
                Err(e) => {
                    tracing::warn!("scenes: no GPU renderer ({e:?}): scenes drawn on the CPU");
                    self.scenes.no_gpu = true;
                }
            }
        }
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        if let Some(gpu) = self.scenes.gpu.as_mut() {
            let (bufs, next) = self.scenes.buffers.entry(key).or_insert_with(|| (Vec::new(), 0));
            if bufs.first().is_some_and(|b| (b.width, b.height) != (pw, ph)) {
                bufs.clear();
            }
            let drawn = (|| -> Result<(), String> {
                while bufs.len() < BUFFERS {
                    bufs.push(gpu.buffer(pw, ph).map_err(|e| format!("{e:?}"))?);
                }
                let b = &bufs[*next];
                gpu.draw_into(b, &scene, fonts, scale, Color::rgba(0, 0, 0, 0)).map_err(|e| format!("{e:?}"))?;
                Ok(())
            })();
            match drawn {
                Ok(()) => {
                    let b = &bufs[*next];
                    *next = (*next + 1) % BUFFERS;
                    let fd = vergence_protocol::window::Fd(std::sync::Arc::new(b.fd.try_clone().map_err(|e| e.to_string())?));
                    let plane = vergence_protocol::window::Plane { fd, offset: b.offset, stride: b.stride };
                    let (w, h, format, modifier) = (b.width, b.height, b.format, b.modifier);
                    self.native_attach_gpu(n, vec![plane], w, h, format, modifier)?;
                    self.scenes.drawn_gpu += 1;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("scenes: the GPU renderer failed ({e}): scenes drawn on the CPU from now");
                    self.scenes.gpu = None;
                    self.scenes.buffers.clear();
                    self.scenes.no_gpu = true;
                }
            }
        }
        // On the CPU, into memory.
        let pix = self.scenes.cpu.render(&scene, fonts, scale, Color::rgba(0, 0, 0, 0)).map_err(|e| e.to_string())?;
        let px: Size<i32, BufferCoords> = (pix.width as i32, pix.height as i32).into();
        self.native_attach_pixels(n, px, &pix.data)?;
        self.scenes.drawn_cpu += 1;
        Ok(())
    }

    /// A window gone: its buffers let go.
    pub fn scenes_forget(&mut self, client: u32, window: u32) {
        self.scenes.buffers.remove(&(client, window));
        self.scenes.alts.retain(|k, _| (k.0, k.1) != (client, window));
    }

    /// Window `n` drawn from its scene at `scale` (another screen's), for
    /// that screen; None if it has no scene or scenes are drawn on the CPU.
    pub fn scene_alt(&mut self, renderer: &mut smithay::backend::renderer::gles::GlesRenderer, n: &NativeWindow, scale: f64) -> Option<Alt> {
        use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
        use smithay::backend::renderer::ImportDma;
        let scene = n.scene()?;
        let gpu = self.scenes.gpu.as_mut()?;
        let at = std::sync::Arc::as_ptr(&scene) as usize;
        let key = (n.client, n.id, scale.to_bits());
        let entry = self.scenes.alts.entry(key).or_insert_with(|| AltEntry { scene: 0, bufs: Vec::new(), next: 0, alt: None });
        if entry.scene == at && entry.alt.is_some() {
            return entry.alt.clone();
        }
        let (pw, ph) = vergence_render::pixel_size(&scene, scale as f32).ok()?;
        // Beyond this a window is enlarged instead (a magnified big window
        // would need a picture larger than a GPU keeps; tiles come later).
        if pw > 8192 || ph > 8192 || pw as u64 * ph as u64 > 40_000_000 {
            return None;
        }
        if entry.bufs.first().is_some_and(|b| (b.width, b.height) != (pw, ph)) {
            entry.bufs.clear();
        }
        while entry.bufs.len() < 2 {
            entry.bufs.push(gpu.buffer(pw, ph).ok()?);
        }
        let b = &entry.bufs[entry.next];
        let fonts = self.shelf.fonts.get_or_insert_with(vergence_text::Fonts::system);
        if let Err(e) = gpu.draw_into(b, &scene, fonts, scale as f32, Color::rgba(0, 0, 0, 0)) {
            tracing::warn!("scenes: drawing at another screen's scale failed: {e:?}");
            return None;
        }
        let fourcc = smithay::backend::allocator::Fourcc::try_from(b.format).ok()?;
        let mut d = Dmabuf::builder((b.width as i32, b.height as i32), fourcc, smithay::backend::allocator::Modifier::from(b.modifier), DmabufFlags::empty());
        d.add_plane(b.fd.try_clone().ok()?, b.offset, b.stride);
        let dmabuf = d.build()?;
        let gles = match renderer.import_dmabuf(&dmabuf, None) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("scenes: a picture at another scale could not be shown: {e:?}");
                return None;
            }
        };
        let texture = smithay::backend::renderer::element::texture::TextureBuffer::from_texture(renderer, gles.clone(), 1, smithay::utils::Transform::Normal, None);
        let serial = ALT_SERIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let alt = Alt { texture, gles, px: (pw as i32, ph as i32).into(), serial };
        entry.next = (entry.next + 1) % 2;
        entry.scene = at;
        entry.alt = Some(alt.clone());
        self.scenes.drawn_alt += 1;
        Some(alt)
    }

    /// For vctl: how scenes are drawn and how many have been.
    pub fn scenes_report(&self) -> String {
        let how = if self.scenes.gpu.is_some() {
            "on the GPU"
        } else if self.scenes.no_gpu {
            "on the CPU"
        } else {
            "not yet"
        };
        format!("scenes drawn {how}: {} on the GPU, {} on the CPU, {} windows with buffers; {} drawn again for another screen's scale", self.scenes.drawn_gpu, self.scenes.drawn_cpu, self.scenes.buffers.len(), self.scenes.drawn_alt)
    }
}
