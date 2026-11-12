// Based on Smithay's smallvil example (MIT; see LICENSE-smithay.txt), adapted for Vergence.
//
// The nested backend (server milestone 1c): one host window shows several virtual
// outputs side by side, each with its own scale, derived from a panel's native
// pixels and physical size the way vergd derived it (vergence-profile). Each output
// is rendered into a texture of its own at its own scale, and the textures are put
// side by side in the window. Nested, every output sits on the same host monitor,
// so this proves the mechanics (several scales at once, the preferred scale per
// window, crossing, input); the physical sizes are proved on the real panels in
// stage 2.

use std::time::Duration;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, Frame, Offscreen, Renderer,
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTexture},
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Physical, Rectangle, Size, Transform},
    wayland::fractional_scale::with_fractional_scale,
};
use vergence_profile::scaling::{Distance, derive};

use crate::{Server, state::OutputRegion};

/// A virtual monitor: a name and the panel's native pixels and physical size.
struct Panel {
    name: String,
    px: (u32, u32),
    mm: (u32, u32),
}

/// `VERGENCE_OUTPUTS="name:WxH:WMMxHMM,..."`, left to right. The default is Hubble's
/// desk: the 14" 4K running 1920x1080 (310 x 174 mm) on the left, the ASUS VS229
/// (475 x 267 mm) on the right.
fn panels() -> Vec<Panel> {
    let spec = std::env::var("VERGENCE_OUTPUTS")
        .unwrap_or_else(|_| "14in 4K:1920x1080:310x174,ASUS VS229:1920x1080:475x267".into());
    let pair = |s: &str| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)));
    spec.split(',')
        .filter_map(|p| {
            let mut it = p.split(':');
            Some(Panel { name: it.next()?.to_string(), px: pair(it.next()?)?, mm: pair(it.next()?)? })
        })
        .collect()
}

struct Virtual {
    output: Output,
    scale: f64,
    damage: OutputDamageTracker,
    texture: Option<GlesTexture>,
    region: Rectangle<i32, Physical>,
}

pub fn init_winit(event_loop: &mut EventLoop<Server>, state: &mut Server) -> Result<(), Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init()?;

    let mut virtuals: Vec<Virtual> = panels()
        .into_iter()
        .map(|p| {
            let scale = crate::scale_in_120ths(derive(p.px, Some(p.mm), Distance::Desktop, 1.0).exact);
            tracing::info!("output {:?}: {}x{} px, {}x{} mm, scale {scale:.2}", p.name, p.px.0, p.px.1, p.mm.0, p.mm.1);
            let output = Output::new(
                p.name.clone(),
                PhysicalProperties {
                    size: (p.mm.0 as i32, p.mm.1 as i32).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Vergence".into(),
                    model: p.name,
                    serial_number: "nested".into(),
                },
            );
            let _global = output.create_global::<Server>(&state.display_handle);
            output.user_data().insert_if_missing(|| crate::memory::MonitorId(format!("nested:{}", output.name())));
            Virtual { damage: OutputDamageTracker::from_output(&output), output, scale, texture: None, region: Rectangle::default() }
        })
        .collect();

    lay_out(&mut virtuals, state, backend.window_size());
    // Nested: the virtual panels stand in for EDIDs in the setup key.
    let names: Vec<Vec<u8>> = virtuals.iter().map(|v| format!("nested:{}", v.output.name()).into_bytes()).collect();
    state.set_setup(&names);

    event_loop.handle().insert_source(winit, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => lay_out(&mut virtuals, state, size),
        WinitEvent::Input(event) => state.process_input_event(event),
        WinitEvent::Redraw => {
            let size = backend.window_size();

            // Each output into its own texture, at its own scale.
            for v in virtuals.iter_mut() {
                let renderer: &mut GlesRenderer = backend.renderer();
                let want = Size::from((v.region.size.w, v.region.size.h));
                if v.texture.as_ref().map(|t| smithay::backend::renderer::Texture::size(t)) != Some(want) {
                    v.texture = Some(renderer.create_buffer(Fourcc::Abgr8888, want).expect("texture"));
                    v.damage = OutputDamageTracker::from_output(&v.output);
                }
                let scene = state.scene(renderer, &v.output, v.output.current_scale().fractional_scale());
                let texture = v.texture.as_mut().unwrap();
                let mut fb = renderer.bind(texture).expect("bind texture");
                v.damage.render_output(renderer, &mut fb, 0, &scene, state.background_color(&v.output.name())).expect("render output");
            }

            // Then side by side in the host window.
            {
                let (renderer, mut fb) = backend.bind().expect("bind window");
                let mut frame = renderer.render(&mut fb, size, Transform::Flipped180).expect("frame");
                frame.clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &[Rectangle::from_size(size)]).expect("clear");
                for v in &virtuals {
                    if let Some(t) = &v.texture {
                        frame
                            .render_texture_at(t, v.region.loc, 1, 1.0, Transform::Normal, &[Rectangle::from_size(v.region.size)], &[], 1.0)
                            .expect("compose");
                    }
                }
                let _ = frame.finish().expect("finish"); // submit() waits for the rendering
            }
            backend.submit(Some(&[Rectangle::from_size(size)])).expect("submit");

            // Each window draws at the scale of the output most of it is on.
            let start = state.start_time.elapsed();
            for window in state.space.elements() {
                let Some(geo) = state.space.element_geometry(window) else { continue };
                let main = virtuals
                    .iter()
                    .filter_map(|v| {
                        let o = state.space.output_geometry(&v.output)?;
                        let i = o.intersection(geo)?;
                        Some((i.size.w as i64 * i.size.h as i64, v))
                    })
                    .max_by_key(|(area, _)| *area)
                    .map(|(_, v)| v);
                if let Some(v) = main {
                    window.with_surfaces(|_, states| {
                        with_fractional_scale(states, |fs| fs.set_preferred_scale(v.scale));
                    });
                    window.send_frame(&v.output, start, Some(Duration::ZERO), |_, _| Some(v.output.clone()));
                }
            }
            // Native windows that wait for a frame, and scale changes (native.rs).
            let drawn: Vec<smithay::output::Output> = virtuals.iter().map(|v| v.output.clone()).collect();
            for o in &drawn {
                state.native_frames(o);
            }

            state.space.refresh();
            state.popups.cleanup();
            let _ = state.display_handle.flush_clients();
            backend.window().request_redraw();
        }
        WinitEvent::CloseRequested => state.loop_signal.stop(),
        _ => (),
    })?;

    Ok(())
}

/// Splits the window into equal columns, one per output, left to right; each output
/// is as many pixels as its column, at its own scale, and the outputs sit side by side
/// in the server's logical space.
fn lay_out(virtuals: &mut [Virtual], state: &mut Server, window: Size<i32, Physical>) {
    let n = virtuals.len().max(1) as i32;
    let col = window.w / n;
    let mut logical_x: f64 = 0.0;
    state.regions.clear();
    for (i, v) in virtuals.iter_mut().enumerate() {
        v.region = Rectangle::new((i as i32 * col, 0).into(), (col, window.h).into());
        let mode = Mode { size: v.region.size, refresh: 60_000 };
        v.output.change_current_state(Some(mode), Some(Transform::Normal), Some(Scale::Fractional(v.scale)), None);
        v.output.set_preferred(mode);
        state.space.map_output(&v.output, (logical_x.round() as i32, 0));
        state.regions.push(OutputRegion {
            window: v.region.to_f64(),
            logical_x,
            scale: v.scale,
        });
        logical_x += col as f64 / v.scale;
    }
    state.window_px = window;
    state.bus_outputs_changed();
}
