// Every GPU on the seat, probed (GPUs milestone 1, step 1; whitepaper §8.3,
// decision 3: capabilities probed, never assumed by vendor). At start each
// GPU is opened through the seat, and what it can do is logged and kept for
// `vctl gpus`: its driver, its connectors and which have a screen, the
// formats its planes scan out, and, through EGL on its own GBM device, the
// formats it can render to and import, and whether a linear buffer is among
// them (the format two GPUs most often share: steps 3 and 4). Then it is
// closed again, so a GPU with nothing to do goes back to sleep (Hubble's
// NVIDIA card wakes for the probe and suspends again). Nothing here changes
// what the server draws on: drm.rs still opens the seat's primary GPU.

use smithay::{
    backend::{
        allocator::{Fourcc, Modifier, gbm::GbmDevice},
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
        session::Session,
        udev::all_gpus,
    },
    reexports::{
        drm::{
            Device as _,
            control::{Device as _, connector},
        },
        rustix::fs::OFlags,
    },
    utils::DeviceFd,
};

use crate::state::Server;

/// What one GPU said it can do.
#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub path: String,
    /// Its PCI slot, where the system says (Linux).
    pub slot: Option<String>,
    pub driver: String,
    /// The one the session draws with (drm.rs's choice).
    pub primary: bool,
    /// Its connectors ("HDMI-A-1") and whether a screen is on each.
    pub connectors: Vec<(String, bool)>,
    /// Formats its planes can show, by name ("XR24").
    pub scanout: Vec<String>,
    /// From EGL on its GBM device: formats it renders to and imports, and
    /// whether each list has a linear XRGB8888 or ARGB8888.
    pub render: Option<Formats>,
    /// What went wrong, if the probe could not finish.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Formats {
    pub render_count: usize,
    pub texture_count: usize,
    pub render_linear: bool,
    pub texture_linear: bool,
}

fn fourcc_name(code: u32) -> String {
    // Names shorter than four letters are padded with spaces ("C8  ").
    let name: String =
        code.to_le_bytes().iter().map(|b| if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '?' }).collect();
    name.trim_end().to_string()
}

/// A GPU's PCI slot (Linux: the device's link in /sys; where the systems
/// differ, FreeBSD gives none here).
fn slot_of(path: &std::path::Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let name = path.file_name()?.to_str()?;
        let link = std::fs::read_link(format!("/sys/class/drm/{name}/device")).ok()?;
        return link.file_name().map(|n| n.to_string_lossy().into_owned());
    }
    #[allow(unreachable_code)]
    {
        let _ = path;
        None
    }
}

/// What an EGL context renders to and imports (the drawing GPU's, from drm.rs).
pub fn formats_of(context: &EGLContext) -> Formats {
    let render =
        context.dmabuf_render_formats().iter().copied().collect::<smithay::backend::allocator::format::FormatSet>();
    let texture =
        context.dmabuf_texture_formats().iter().copied().collect::<smithay::backend::allocator::format::FormatSet>();
    Formats {
        render_count: render.iter().count(),
        texture_count: texture.iter().count(),
        render_linear: linear_xrgb(&render),
        texture_linear: linear_xrgb(&texture),
    }
}

fn linear_xrgb(formats: &smithay::backend::allocator::format::FormatSet) -> bool {
    formats.iter().any(|f| matches!(f.code, Fourcc::Xrgb8888 | Fourcc::Argb8888) && f.modifier == Modifier::Linear)
}

/// Probe one GPU: open it, ask, close it. The one drm.rs draws with comes
/// open (`drawing`: its descriptor and its renderer's formats): a second open
/// through logind fails, "Invalid argument" (on Fedora a machine's only GPU
/// showed no driver and no screens), and a second EGL display
/// on it would share, and on drop could end, the session's own.
fn probe_one(
    session: &mut smithay::backend::session::libseat::LibSeatSession,
    path: &std::path::Path,
    drawing: Option<(&DrmDeviceFd, &Formats)>,
) -> GpuInfo {
    let mut info = GpuInfo {
        path: path.display().to_string(),
        slot: slot_of(path),
        primary: drawing.is_some(),
        ..Default::default()
    };
    let fd = match drawing {
        Some((fd, _)) => fd.clone(),
        None => match session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK) {
            Ok(fd) => DrmDeviceFd::new(DeviceFd::from(fd)),
            Err(e) => {
                info.error = Some(format!("open: {e}"));
                return info;
            }
        },
    };
    if let Ok(d) = fd.get_driver() {
        info.driver = d.name().to_string_lossy().into_owned();
    }
    match fd.resource_handles() {
        Ok(res) => {
            for h in res.connectors() {
                if let Ok(c) = fd.get_connector(*h, false) {
                    info.connectors.push((
                        format!("{}-{}", c.interface().as_str(), c.interface_id()),
                        c.state() == connector::State::Connected,
                    ));
                }
            }
        }
        Err(e) => info.error = Some(format!("resources: {e}")),
    }
    if let Ok(planes) = fd.plane_handles() {
        let mut seen: Vec<String> = Vec::new();
        for p in planes {
            if let Ok(plane) = fd.get_plane(p) {
                for f in plane.formats() {
                    let n = fourcc_name(*f);
                    if !seen.contains(&n) {
                        seen.push(n);
                    }
                }
            }
        }
        info.scanout = seen;
    }
    // Rendering: EGL on the GPU's own GBM device (a display-only GPU has none).
    if let Some((_, formats)) = drawing {
        info.render = Some(formats.clone());
        return info;
    }
    let rendered = (|| -> Result<Formats, String> {
        let gbm = GbmDevice::new(fd.clone()).map_err(|e| format!("gbm: {e}"))?;
        let display = unsafe { EGLDisplay::new(gbm) }.map_err(|e| format!("egl: {e}"))?;
        let context = EGLContext::new(&display).map_err(|e| format!("egl context: {e}"))?;
        let render =
            context.dmabuf_render_formats().iter().copied().collect::<smithay::backend::allocator::format::FormatSet>();
        let texture = context
            .dmabuf_texture_formats()
            .iter()
            .copied()
            .collect::<smithay::backend::allocator::format::FormatSet>();
        Ok(Formats {
            render_count: render.iter().count(),
            texture_count: texture.iter().count(),
            render_linear: linear_xrgb(&render),
            texture_linear: linear_xrgb(&texture),
        })
    })();
    match rendered {
        Ok(f) => info.render = Some(f),
        Err(e) => info.error = Some(e),
    }
    info
}

/// Probe every GPU on the seat; `primary` is the path drm.rs draws with,
/// `fd` its open descriptor and `formats` what its renderer offers.
pub fn probe(
    session: &mut smithay::backend::session::libseat::LibSeatSession,
    seat: &str,
    primary: &std::path::Path,
    fd: &DrmDeviceFd,
    formats: &Formats,
) -> Vec<GpuInfo> {
    let paths = all_gpus(seat).unwrap_or_default();
    let mut out = Vec::new();
    for path in paths {
        let info = probe_one(session, &path, (path == primary).then_some((fd, formats)));
        log(&info);
        out.push(info);
    }
    out
}

fn log(g: &GpuInfo) {
    let screens: Vec<String> =
        g.connectors.iter().map(|(n, on)| if *on { format!("{n} (screen)") } else { n.clone() }).collect();
    tracing::info!(
        "gpu: {}{} {} driver {}, connectors {}, scanout {}",
        g.path,
        g.slot.as_deref().map(|s| format!(" [{s}]")).unwrap_or_default(),
        if g.primary { "(draws the session)" } else { "(not used yet)" },
        g.driver,
        if screens.is_empty() { "none".into() } else { screens.join(", ") },
        g.scanout.join(" ")
    );
    match (&g.render, &g.error) {
        (Some(f), _) => tracing::info!(
            "gpu: {}: renders {} formats (linear XRGB {}), imports {} (linear XRGB {})",
            g.path,
            f.render_count,
            if f.render_linear { "yes" } else { "no" },
            f.texture_count,
            if f.texture_linear { "yes" } else { "no" }
        ),
        (None, Some(e)) => tracing::warn!("gpu: {}: {e}", g.path),
        _ => {}
    }
}

/// A GPU in words, by its driver: "NVIDIA graphics (nouveau)".
pub fn words(driver: &str) -> String {
    let vendor = match driver {
        "i915" | "xe" => "Intel",
        "nouveau" | "nvidia-drm" | "nvidia" => "NVIDIA",
        "amdgpu" | "radeon" => "AMD",
        "" => return "graphics".into(),
        _ => return format!("{driver} graphics"),
    };
    format!("{vendor} graphics ({driver})")
}

impl Server {
    /// Which GPU shows (or, `drawn`, draws) the screen with this connector
    /// name, in words; empty when there is no DRM backend (nested).
    pub fn gpu_words(&self, connector: &str, drawn: bool) -> String {
        let Some(path) = self.backend.as_ref().and_then(|b| b.gpu_path(connector, drawn)) else { return String::new() };
        let driver = self.gpus.iter().find(|g| g.path == path).map(|g| g.driver.as_str()).unwrap_or("");
        words(driver)
    }

    /// For `vctl gpus`.
    pub fn describe_gpus(&self) -> String {
        if self.gpus.is_empty() {
            return "no GPUs probed (nested, or before start)".into();
        }
        let mut out = String::new();
        for g in &self.gpus {
            out.push_str(&format!(
                "{}{}  driver {}{}\n",
                g.path,
                g.slot.as_deref().map(|s| format!(" [{s}]")).unwrap_or_default(),
                g.driver,
                if g.primary { "  (draws the session)" } else { "" }
            ));
            for (n, on) in &g.connectors {
                out.push_str(&format!("  {n}{}\n", if *on { "  screen connected" } else { "" }));
            }
            out.push_str(&format!("  scans out: {}\n", g.scanout.join(" ")));
            if let Some(f) = &g.render {
                out.push_str(&format!(
                    "  renders {} formats (linear XRGB: {}), imports {} (linear XRGB: {})\n",
                    f.render_count,
                    if f.render_linear { "yes" } else { "no" },
                    f.texture_count,
                    if f.texture_linear { "yes" } else { "no" }
                ));
            }
            if let Some(e) = &g.error {
                out.push_str(&format!("  probe: {e}\n"));
            }
        }
        out.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpus_in_words() {
        assert_eq!(words("i915"), "Intel graphics (i915)");
        assert_eq!(words("nouveau"), "NVIDIA graphics (nouveau)");
        assert_eq!(words("amdgpu"), "AMD graphics (amdgpu)");
        assert_eq!(words("vkms"), "vkms graphics");
    }

    #[test]
    fn fourcc_names() {
        assert_eq!(fourcc_name(Fourcc::Xrgb8888 as u32), "XR24");
        assert_eq!(fourcc_name(Fourcc::Argb8888 as u32), "AR24");
        assert_eq!(fourcc_name(Fourcc::C8 as u32), "C8");
        assert_eq!(fourcc_name(0x0101_0101), "????");
    }
}
