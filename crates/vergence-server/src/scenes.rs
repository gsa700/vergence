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
}

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
        format!("scenes drawn {how}: {} on the GPU, {} on the CPU, {} windows with buffers", self.scenes.drawn_gpu, self.scenes.drawn_cpu, self.scenes.buffers.len())
    }
}
