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
// Step 6, safety and cost. A scene is untrusted. Before it is drawn its
// cost is estimated (vergence-render `cost`, pixels touched); over
// COST_BUDGET it is not drawn. A draw that takes longer than SLOW is a
// strike; STRIKES of them, or one over TOO_SLOW, the same. Either way the
// window's scenes are switched off: the app is told (bus v31 `ScenesOff`)
// and sends its own pixels from then on, so a scene can cost the server
// one bounded draw at most. Drawn again for another scale (steps 4-5): the
// same budget, and a slow one stops those for the window (enlarged as
// before). Memory: GPU buffers given back to the renderer when a window
// resizes or goes (they were dropped without that before step 6, leaking
// the GPU's memory), copies for other scales at most ALTS_PER_WINDOW per
// window, let go after ALT_IDLE unused, ALT_BYTES in all.
//
// A scene brings its own ground (the toolkit's first fill); it is drawn on
// clear. The whole scene is drawn each time, as the apps did; drawing only
// what changed comes later (the plan's step 2 hope, noted there).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use smithay::utils::{Buffer as BufferCoords, Size};
use vergence_render::{Cpu, Gpu, GpuBuffer};
use vergence_scene::Color;

use crate::{Server, native::NativeWindow};

/// How many buffers a window takes in turn (one shown, one waiting, one drawn).
const BUFFERS: usize = 3;

/// The most work (pixels touched, vergence-render `cost`) one draw may be:
/// about fifty 4K screens filled; a real window is a few.
pub const COST_BUDGET: u64 = 400_000_000;

/// COST_BUDGET, or VERGENCE_SCENES_BUDGET (pixels of work) for tests: a
/// small one switches every window to pixels.
fn budget() -> u64 {
    static B: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *B.get_or_init(|| std::env::var("VERGENCE_SCENES_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(COST_BUDGET))
}
/// A draw slower than this is a strike; this many strikes, scenes off.
const SLOW: Duration = Duration::from_millis(100);
const STRIKES: u32 = 3;
/// One draw slower than this, scenes off at once.
const TOO_SLOW: Duration = Duration::from_millis(500);
/// Copies for other scales: per window, unused this long, bytes in all.
const ALTS_PER_WINDOW: usize = 2;
const ALT_IDLE: Duration = Duration::from_secs(10);
const ALT_BYTES: usize = 1 << 30;

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
    /// Slow draws by window; windows whose copies for other scales stopped.
    strikes: HashMap<(u32, u32), u32>,
    no_alts: HashSet<(u32, u32)>,
    /// Windows whose scenes were switched off, for vctl.
    pub switched_off: u64,
    /// Time spent drawing windows' scenes (not copies for other scales),
    /// and the slowest draw, for vctl (step 7's timing).
    draw_time: Duration,
    slowest: Duration,
}

impl Scenes {
    /// Gives buffers back to the renderer (the server keeps its own hold on
    /// any it still shows).
    fn release(&mut self, bufs: Vec<GpuBuffer>) {
        if let Some(gpu) = self.gpu.as_mut() {
            for b in bufs {
                gpu.release(b);
            }
        }
    }

    /// Copies for other scales: none unused past ALT_IDLE, at most
    /// ALTS_PER_WINDOW a window, ALT_BYTES in all (the oldest go first).
    fn trim_alts(&mut self, now: Instant) {
        let mut keys: Vec<((u32, u32, u64), Instant, usize)> =
            self.alts.iter().map(|(k, e)| (*k, e.used, e.bytes())).collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.1));
        let (mut total, mut per) = (0usize, HashMap::<(u32, u32), usize>::new());
        let mut gone = Vec::new();
        for (k, used, bytes) in keys {
            let n = per.entry((k.0, k.1)).or_default();
            if now.duration_since(used) > ALT_IDLE || *n >= ALTS_PER_WINDOW || total + bytes > ALT_BYTES {
                gone.push(k);
            } else {
                *n += 1;
                total += bytes;
            }
        }
        for k in gone {
            if let Some(e) = self.alts.remove(&k) {
                self.release(e.bufs);
            }
        }
    }
}

struct AltEntry {
    scene: usize,
    bufs: Vec<GpuBuffer>,
    next: usize,
    alt: Option<Alt>,
    used: Instant,
}

impl AltEntry {
    fn bytes(&self) -> usize {
        self.bufs.iter().map(|b| b.stride as usize * b.height as usize).sum()
    }
}

/// A window's picture drawn again at another screen's scale.
#[derive(Clone)]
pub struct Alt {
    pub texture:
        smithay::backend::renderer::element::texture::TextureBuffer<smithay::backend::renderer::gles::GlesTexture>,
    pub gles: smithay::backend::renderer::gles::GlesTexture,
    pub px: Size<i32, BufferCoords>,
    /// Counted apart from the pictures' serials (backdrop.rs tells frames apart by it).
    pub serial: u64,
    /// The GL context it was imported on (backdrop.rs Ctx).
    pub ctx: crate::backdrop::Ctx,
}

static ALT_SERIALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 48);

impl Server {
    /// Draws window `n`'s last scene at its scale and makes it its picture.
    pub fn native_draw_scene(&mut self, n: &NativeWindow) -> Result<(), String> {
        let Some(scene) = n.scene() else { return Ok(()) };
        let scale = n.scale().max(0.25) as f32;
        let (pw, ph) = vergence_render::pixel_size(&scene, scale).map_err(|e| e.to_string())?;
        let key = (n.client, n.id);
        let cost = vergence_render::cost(&scene, scale);
        if cost > budget() {
            let why = format!("a scene too costly to draw ({cost} pixels of work, at most {})", budget());
            self.scenes_off(n, &why);
            return Err(why);
        }
        let now = Instant::now();
        self.scenes.trim_alts(now);
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
                // Resized: the old buffers given back, not just dropped.
                for b in bufs.drain(..) {
                    gpu.release(b);
                }
                *next = 0;
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
                    let fd = vergence_protocol::window::Fd(std::sync::Arc::new(
                        b.fd.try_clone().map_err(|e| e.to_string())?,
                    ));
                    let plane = vergence_protocol::window::Plane { fd, offset: b.offset, stride: b.stride };
                    let (w, h, format, modifier) = (b.width, b.height, b.format, b.modifier);
                    self.native_attach_gpu(n, vec![plane], w, h, format, modifier)?;
                    self.scenes.drawn_gpu += 1;
                    self.scene_timed(n, now.elapsed());
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        "scenes: the GPU renderer failed on {:?} window {} ({e}): scenes drawn on the CPU from now",
                        n.app_id,
                        n.id
                    );
                    let all: Vec<GpuBuffer> = self
                        .scenes
                        .buffers
                        .drain()
                        .flat_map(|(_, (b, _))| b)
                        .chain(self.scenes.alts.drain().flat_map(|(_, e)| e.bufs))
                        .collect();
                    self.scenes.release(all);
                    self.scenes.gpu = None;
                    self.scenes.no_gpu = true;
                }
            }
        }
        // On the CPU, into memory.
        let pix = self.scenes.cpu.render(&scene, fonts, scale, Color::rgba(0, 0, 0, 0)).map_err(|e| e.to_string())?;
        let px: Size<i32, BufferCoords> = (pix.width as i32, pix.height as i32).into();
        self.native_attach_pixels(n, px, &pix.data)?;
        self.scenes.drawn_cpu += 1;
        self.scene_timed(n, now.elapsed());
        Ok(())
    }

    /// A draw of window `n`'s scene took `took`: slow ones counted, too many
    /// (or one far too slow) and its scenes are switched off.
    fn scene_timed(&mut self, n: &NativeWindow, took: Duration) {
        self.scenes.draw_time += took;
        self.scenes.slowest = self.scenes.slowest.max(took);
        if took < SLOW {
            return;
        }
        let strikes = self.scenes.strikes.entry((n.client, n.id)).or_default();
        *strikes += 1;
        tracing::warn!(
            "scenes: client {} window {} took {} ms to draw (strike {})",
            n.client,
            n.id,
            took.as_millis(),
            strikes
        );
        if took >= TOO_SLOW || *strikes >= STRIKES {
            self.scenes_off(n, &format!("its scenes were too slow to draw ({} ms)", took.as_millis()));
        }
    }

    /// Window `n`'s scenes no longer drawn: it is told to send pixels, and
    /// what the server kept for its scenes is let go.
    pub fn scenes_off(&mut self, n: &NativeWindow, why: &str) {
        if n.scenes_off() {
            return;
        }
        tracing::warn!("scenes: client {} window {}: scenes off, the app draws it: {why}", n.client, n.id);
        n.set_scenes_off(why);
        self.scenes_forget(n.client, n.id);
        self.scenes.switched_off += 1;
        let why: String = why.chars().take(200).collect();
        self.bus_send(
            n.client,
            vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::ScenesOff {
                window: n.id,
                why,
            }),
        );
    }

    /// After a GPU hang (or `renew-gpu`): the scene renderer's own context may be as dead
    /// as the screen's (drawing nothing, every call succeeding), so it starts again on a
    /// new one at the next draw, and the hang's slow frame counts against no window.
    pub fn scenes_renew(&mut self) {
        let all: Vec<GpuBuffer> = self
            .scenes
            .buffers
            .drain()
            .flat_map(|(_, (b, _))| b)
            .chain(self.scenes.alts.drain().flat_map(|(_, e)| e.bufs))
            .collect();
        self.scenes.release(all);
        self.scenes.gpu = None;
        self.scenes.strikes.clear();
        // Every window's scene drawn again on the new one.
        let wins: Vec<std::sync::Arc<NativeWindow>> = self
            .natives
            .values()
            .filter_map(|w| match w {
                crate::win::Win::Native(n) if !n.scenes_off() => Some(n.clone()),
                _ => None,
            })
            .collect();
        for n in wins {
            if let Err(e) = self.native_draw_scene(&n) {
                tracing::warn!("scenes: window {} of client {} not drawn again: {e}", n.id, n.client);
            }
        }
    }

    /// A window gone: its buffers let go.
    pub fn scenes_forget(&mut self, client: u32, window: u32) {
        let mut gone: Vec<GpuBuffer> =
            self.scenes.buffers.remove(&(client, window)).map(|(b, _)| b).unwrap_or_default();
        let keys: Vec<_> = self.scenes.alts.keys().filter(|k| (k.0, k.1) == (client, window)).copied().collect();
        for k in keys {
            gone.extend(self.scenes.alts.remove(&k).map(|e| e.bufs).unwrap_or_default());
        }
        self.scenes.release(gone);
        self.scenes.strikes.remove(&(client, window));
        self.scenes.no_alts.remove(&(client, window));
    }

    /// Window `n` drawn from its scene at `scale` (another screen's), for
    /// that screen; None if it has no scene or scenes are drawn on the CPU.
    pub fn scene_alt(
        &mut self,
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        n: &NativeWindow,
        scale: f64,
    ) -> Option<Alt> {
        use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
        use smithay::backend::renderer::ImportDma;
        let scene = n.scene()?;
        if self.scenes.no_alts.contains(&(n.client, n.id)) {
            return None;
        }
        let now = Instant::now();
        let at = std::sync::Arc::as_ptr(&scene) as usize;
        let key = (n.client, n.id, scale.to_bits());
        if let Some(e) = self.scenes.alts.get_mut(&key) {
            e.used = now;
            let now = crate::backdrop::ctx(renderer);
            if e.scene == at && e.alt.as_ref().is_some_and(|a| a.ctx == now) {
                return e.alt.clone();
            }
        }
        let (pw, ph) = vergence_render::pixel_size(&scene, scale as f32).ok()?;
        // Beyond this a window is enlarged instead (a magnified big window
        // would need a picture larger than a GPU keeps; tiles come later).
        if pw > 8192
            || ph > 8192
            || pw as u64 * ph as u64 > 40_000_000
            || vergence_render::cost(&scene, scale as f32) > budget()
        {
            return None;
        }
        self.scenes.gpu.as_ref()?;
        self.scenes.alts.entry(key).or_insert_with(|| AltEntry {
            scene: 0,
            bufs: Vec::new(),
            next: 0,
            alt: None,
            used: now,
        });
        self.scenes.trim_alts(now);
        let gpu = self.scenes.gpu.as_mut()?;
        let entry = self.scenes.alts.get_mut(&key)?;
        if entry.bufs.first().is_some_and(|b| (b.width, b.height) != (pw, ph)) {
            for b in entry.bufs.drain(..) {
                gpu.release(b);
            }
            entry.next = 0;
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
        let mut d = Dmabuf::builder(
            (b.width as i32, b.height as i32),
            fourcc,
            smithay::backend::allocator::Modifier::from(b.modifier),
            DmabufFlags::empty(),
        );
        d.add_plane(b.fd.try_clone().ok()?, b.offset, b.stride);
        let dmabuf = d.build()?;
        let gles = match renderer.import_dmabuf(&dmabuf, None) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("scenes: a picture at another scale could not be shown: {e:?}");
                return None;
            }
        };
        let texture = smithay::backend::renderer::element::texture::TextureBuffer::from_texture(
            renderer,
            gles.clone(),
            1,
            smithay::utils::Transform::Normal,
            None,
        );
        let serial = ALT_SERIALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let ctx = crate::backdrop::ctx(renderer);
        let alt = Alt { texture, gles, px: (pw as i32, ph as i32).into(), serial, ctx };
        entry.next = (entry.next + 1) % 2;
        entry.scene = at;
        entry.alt = Some(alt.clone());
        self.scenes.drawn_alt += 1;
        if now.elapsed() >= SLOW {
            tracing::warn!(
                "scenes: client {} window {} took {} ms to draw at {scale:.2}: enlarged from now",
                n.client,
                n.id,
                now.elapsed().as_millis()
            );
            self.scenes.no_alts.insert((n.client, n.id));
        }
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
        let drawn = (self.scenes.drawn_gpu + self.scenes.drawn_cpu).max(1);
        let times = format!(
            "; {:.2} ms a draw on average, slowest {:.1} ms",
            self.scenes.draw_time.as_secs_f64() * 1e3 / drawn as f64,
            self.scenes.slowest.as_secs_f64() * 1e3
        );
        format!(
            "scenes drawn {how}: {} on the GPU, {} on the CPU, {} windows with buffers; {} drawn again for another screen's scale ({} kept, {} MB); {} switched to pixels",
            self.scenes.drawn_gpu,
            self.scenes.drawn_cpu,
            self.scenes.buffers.len(),
            self.scenes.drawn_alt,
            self.scenes.alts.len(),
            self.scenes.alts.values().map(AltEntry::bytes).sum::<usize>() >> 20,
            self.scenes.switched_off
        ) + &times
    }
}
