// Windows on the project's own protocol (native milestone 1, step 3).
//
// A client of the session bus asks for a window (`WinRequest::Create`) with
// its identity (Contract 2: the connection's application id and a role), a
// title, a size and whether it draws its own bar. The server places it
// (window memory first, then the monitor under the pointer), answers
// `Created` and a `Configure` with the size in reference units and the exact
// scale of its monitor (Contract 1), and shows it with its first picture.
// Pictures are shared memory or GPU buffers. For shared memory the server
// reads the damaged rows with pread, never mapping the client's file, so a
// client that shrinks its file under us gets a short read and loses its
// connection, where a mapping would take the server down with SIGBUS. The
// pixels go into a MemoryRenderBuffer, drawn by decor.rs's scene like a
// title bar's text. A GPU buffer (dmabuf, step 4) is imported into the
// renderer when the scene is next built, and drawn as a texture; the
// client may draw into it again once it has attached a later picture and
// a frame has followed (the server swaps pictures at attach). A window asks for its next frame by attaching; the
// server says `Frame` after drawing a monitor it is on, at the monitor's
// pace.
//
// Input (step 4): the keyboard through Smithay's seat, as a third kind of
// keyboard focus (focus.rs: the keymap once per client as a descriptor, the
// repeat settings, keys as evdev codes, modifiers serialized as xkb's). The
// pointer beside Smithay's, whose focus type is a Wayland surface: over a
// native window Smithay's focus is none and this file sends enter, leave,
// motion (in the window's reference units), buttons and the wheel, keeping
// the window while a button is held (an implicit grab, as Wayland's). The
// window's cursor shape is drawn while the pointer is over it. The server
// draws every native window's bar and moves and resizes it from there;
// minimize and maximize on request. Fingers as the pointer is
// (touch.rs).
//
// The clipboard: a native client's copy becomes a server-side selection in
// Smithay's data device, owned by that client (ClipOwner::Native), so
// Wayland clients paste from it as from any other, and Xwayland is told; a
// paste is given the read end of a pipe whose write end goes to whoever has
// the clipboard (a Wayland client, X11, or another native client, which
// hears ClipboardSend). Native clients hear ClipboardOffer when it changes.
//
// Placement (Contract 2, step 6): window memory by identity first (where the
// user last left it), then where the window asks to be (a point of a monitor
// by its id, or of its parent window), when that puts it wholly on a
// monitor, then the monitor under the pointer. A later Place request moves
// it on the same terms. Meaning (Contract 3, step 6): each window's tree as
// it last sent it, kept for `vctl tree`; nothing else reads it yet, and a
// secret field's text never arrives (the toolkit keeps it).

use std::{
    os::{fd::AsFd, unix::fs::FileExt},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    output::Output,
    utils::{Buffer as BufferCoords, Logical, Point, Rectangle, Size, Transform, user_data::UserDataMap},
};
use vergence_protocol::{
    bus::{ErrorCode, ServerMsg},
    window::{Buffer, Damage, SHM_ARGB8888, SHM_XRGB8888, WinEvent, WinRequest},
};

use crate::{Server, win::Win};

/// Most windows one client may have open.
const MAX_WINDOWS: usize = 64;
/// The smallest size a window is given, in reference units.
const MIN_SIDE: i32 = 32;

/// A window of the project's own protocol.
/// What a native window said of itself with `Style`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeStyle {
    pub kind: vergence_protocol::window::WinKind,
    pub modal: bool,
    pub min: (f64, f64),
    pub max: (f64, f64),
    /// No ground of its own: the server frosts what is behind its whole body
    /// (bus v36; preview milestone 1, step 6b; decor.rs).
    pub frosted: bool,
}

/// Actions sent to native windows by their meaning (Contract 3, bus v24,
/// meaning milestone 1) and the answers that came back: the last few kept,
/// by serial, for whoever sent them to read (`vctl acted`, later the agent
/// interface).
#[derive(Default)]
pub struct Acts {
    next: u32,
    /// By serial.
    sent: std::collections::BTreeMap<u32, ActSent>,
    /// Agents' paged reads (bus v37) waiting for their window, by serial
    /// (the same numbering as actions).
    reads: std::collections::BTreeMap<u32, ReadSent>,
}

/// A read sent: the client and window it went to, the agent's bus client and
/// serial, the node, and when it went.
type ReadSent = (u32, u32, u32, u32, u64, std::time::Instant);

/// An action sent: the client and window it went to, its answer (done, or
/// why not) once given, the agent to pass the answer to (its bus client and
/// the serial it asked with), if an agent asked, and when it went.
type ActSent = (u32, u32, Option<(bool, String)>, Option<(u32, u32)>, std::time::Instant);

/// How long a window has to answer an action before it fails.
const ACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A tree as it came, made safe to walk (Fable's review): a node whose id
/// came before is dropped, and a parent link that loops or runs deeper than
/// 64 is cut (the node then hangs from the window itself).
pub fn sane_tree(mut nodes: Vec<vergence_protocol::window::Node>) -> Vec<vergence_protocol::window::Node> {
    let mut seen = std::collections::HashSet::new();
    nodes.retain(|n| n.id != 0 && seen.insert(n.id));
    let parent: std::collections::HashMap<u64, u64> = nodes.iter().map(|n| (n.id, n.parent)).collect();
    for n in nodes.iter_mut() {
        let mut p = n.parent;
        let mut depth = 0;
        let mut visited = std::collections::HashSet::new();
        while p != 0 {
            if p == n.id || !visited.insert(p) || depth > 64 || !parent.contains_key(&p) {
                n.parent = 0;
                break;
            }
            p = parent[&p];
            depth += 1;
        }
    }
    nodes
}

/// How many actions' answers are kept.
const ACTS_KEPT: usize = 256;

pub struct NativeWindow {
    /// The bus connection it belongs to, and the id the client chose.
    pub client: u32,
    pub id: u32,
    /// Its identity (Contract 2).
    pub app_id: String,
    pub role: String,
    /// Its persistence token (memory.rs): the one it presented if valid, else new.
    pub token: String,
    /// Whether its program keeps tokens (it presented one, or "new"): memory
    /// then goes by the token; otherwise by numbering, as for programs that
    /// keep none, so they leave no token behind in the store.
    pub keeps_token: bool,
    alive: AtomicBool,
    state: Mutex<State>,
    user_data: UserDataMap,
}

struct State {
    title: String,
    /// The size it was last told to draw at, in reference units, and the scale.
    size: Size<i32, Logical>,
    scale: f64,
    /// Its screen's text factor (displays.rs, Scales::text), told with the scale.
    text: f64,
    maximized: bool,
    activated: bool,
    /// The last configure sent and the last one the client said it drew for;
    /// while they differ, a newer size waits in `pending` (a resize drag sends
    /// one size per acknowledgement, not one per mouse movement).
    sent_serial: u32,
    acked_serial: u32,
    pending: Option<(Size<i32, Logical>, bool)>,
    /// Where it goes when its first picture comes, and whether memory had it maximized.
    place: Point<i32, Logical>,
    place_maximized: bool,
    shown: bool,
    picture: Option<Picture>,
    /// It attached a picture and waits to hear when to draw the next.
    frame_wanted: bool,
    /// The pointer's shape over it, as it asked (step 4).
    cursor: smithay::input::pointer::CursorIcon,
    /// Its meaning as last sent (Contract 3): the focused widget, the caret, the tree.
    meaning: Option<crate::meaning::WindowMeaning>,
    /// What its bar shows, as last sent (bar milestone 1); none: a plain bar
    /// with its title, as before. The server draws it from step 3.
    bar: Option<vergence_protocol::window::Bar>,
    /// It draws under its bar (look milestone 1): its pictures are the bar's
    /// height taller, the strip at their top under the bar; the inset last
    /// sent (the bar's height where it is: the size of controls).
    under_bar: bool,
    inset: i32,
    /// Its parent (Create's, the same connection's window id; 0: none).
    parent: u32,
    /// What it said of itself with `Style` (window styles milestone 1).
    style: Option<NativeStyle>,
    /// Its app menu's commands as last sent, and whether it is a terminal
    /// (app settings milestone 1).
    commands: Vec<vergence_protocol::commands::Command>,
    terminal: bool,
    /// Its bar's style, as it asked (app settings milestone 1).
    bar_style: vergence_protocol::window::BarStyle,
    /// Its last display list, if it sent one (display lists milestone 1),
    /// and the pictures it sent whole, kept by key with their bytes.
    scene: Option<Arc<vergence_scene::Scene>>,
    scene_images: std::collections::HashMap<u64, Arc<vergence_scene::Image>>,
    scene_image_bytes: usize,
    /// Scenes taken, and refused (with the last reason), for vctl.
    scenes: u64,
    scene_refused: Option<(u64, String)>,
    /// The server stopped drawing its scenes (step 6), and why: it sends
    /// pixels from then on (told by `ScenesOff`).
    scenes_off: Option<String>,
}

/// A keyed scene read against the pictures a window keeps (`images`, their
/// bytes in `kept`): keys to forget dropped first, new pictures kept (at
/// most SCENE_KEPT_BYTES in all), the scene decoded and checked. On a
/// refusal the pictures kept stay as they were, but for those forgotten.
pub(crate) fn take_keyed(
    images: &mut std::collections::HashMap<u64, Arc<vergence_scene::Image>>,
    kept: &mut usize,
    bytes: &[u8],
    forget: &[u64],
) -> Result<vergence_scene::Scene, String> {
    for k in forget {
        if let Some(im) = images.remove(k) {
            *kept -= im.rgba.len();
        }
    }
    let d = vergence_scene::Scene::decode_keyed(bytes, &|k| images.get(&k).cloned()).map_err(|e| e.to_string())?;
    let more: usize = d.new.iter().filter(|(k, _)| !images.contains_key(k)).map(|(_, im)| im.rgba.len()).sum();
    if *kept + more > SCENE_KEPT_BYTES {
        return Err(format!("more than {} MB of pictures kept", SCENE_KEPT_BYTES >> 20));
    }
    for (k, im) in d.new {
        if let Some(old) = images.insert(k, im.clone()) {
            *kept -= old.rgba.len();
        }
        *kept += im.rgba.len();
    }
    Ok(d.scene)
}

/// The most picture bytes one window may have kept for its scenes.
const SCENE_KEPT_BYTES: usize = 512 << 20;

/// A memfd's first `len` bytes (a scene), read at once: the client may
/// reuse it as soon as the message is read. Refused if the file is shorter.
fn read_memfd(fd: &vergence_protocol::window::Fd, len: u32) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::from(fd.0.try_clone().map_err(|e| e.to_string())?);
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size < len as u64 {
        return Err(format!("the scene's file has {size} bytes, not {len}"));
    }
    let mut buf = vec![0u8; len as usize];
    file.read_exact_at(&mut buf, 0).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Native windows' share of the pointer and the keyboard (step 4).
#[derive(Default)]
pub struct NativeInput {
    /// Bus clients that have been sent the keymap.
    keymap_sent: std::collections::HashSet<u32>,
    /// The kinds of data on the clipboard as last offered, for clients that
    /// come later.
    clip_kinds: Option<Vec<String>>,
}

/// Who owns a selection the server holds for someone (Smithay's server-side
/// selection user data): Xwayland, or a native client by its bus id.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipOwner {
    X11,
    Native(u32),
    /// The server's own text (what was copied from the app menu's search
    /// field, 2026-11-18), written to whoever pastes.
    Server(Arc<String>),
    /// A picture the server put there (a screenshot with Ctrl, 2026-11-20),
    /// as PNG: written to whoever pastes it as image/png.
    Picture(Arc<Vec<u8>>),
}

/// The server's own picture (PNG) to a paste's pipe, on a thread of its
/// own; asked for as anything but image/png, nothing is written.
pub fn write_clip_picture(png: Arc<Vec<u8>>, kind: &str, fd: std::os::fd::OwnedFd) {
    if kind != "image/png" {
        return;
    }
    let _ = std::thread::Builder::new().name("clipboard".into()).spawn(move || {
        use std::io::Write;
        let mut f = std::fs::File::from(fd);
        let _ = f.write_all(&png);
    });
}

/// The kinds of text the server offers when it holds the clipboard.
pub const TEXT_KINDS: [&str; 5] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING"];

/// The server's own text to a paste's pipe, on a thread of its own (the
/// reader may be slow, or never read).
pub fn write_clip_text(text: Arc<String>, fd: std::os::fd::OwnedFd) {
    let _ = std::thread::Builder::new().name("clipboard".into()).spawn(move || {
        use std::io::Write;
        let mut f = std::fs::File::from(fd);
        let _ = f.write_all(text.as_bytes());
    });
}

/// A native window's pixels, as the scene draws them.
#[derive(Clone)]
pub struct Picture {
    pub content: Content,
    /// Its size in pixels, and in reference units at the scale it was drawn for.
    pub px: Size<i32, BufferCoords>,
    pub logical: Size<i32, Logical>,
    format: u32,
    /// Declared opaque to the renderer (an Xrgb picture of a window that is
    /// not frosted): a picture made otherwise is made again, not reused.
    opaque: bool,
}

#[derive(Clone)]
pub enum Content {
    /// Shared memory, copied in.
    Memory(MemoryRenderBuffer),
    /// A GPU buffer, imported when the scene is built (decor.rs).
    Gpu(Arc<Mutex<GpuPicture>>),
}

pub struct GpuPicture {
    pub dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
    pub texture: Option<
        smithay::backend::renderer::element::texture::TextureBuffer<smithay::backend::renderer::gles::GlesTexture>,
    >,
    /// The same texture as GL knows it, for our shaders (backdrop.rs Shaded).
    pub gles: Option<smithay::backend::renderer::gles::GlesTexture>,
    /// The renderer refused it: not tried again (the window keeps its last picture).
    pub failed: bool,
    /// Which attach this was, counted over all windows: what tells a new
    /// frame from the last (backdrop.rs Shaded). The GL texture's number
    /// can't: a freed one is handed straight back to the next import, so a
    /// new frame looked unchanged and stayed off the screen until the next
    /// (the terminal's echo, for one).
    pub serial: u64,
}

/// The region of a shared-memory picture the renderer may take as opaque:
/// all of an Xrgb one (it has no alpha), none of an Argb one, and none of a
/// frosted window's (bus v36; preview milestone 1, step 6b): the frost the
/// server draws behind it must show through wherever it leaves its pixels
/// clear, and an opaque region would let the renderer skip what is behind.
fn opaque_region(
    fourcc: Fourcc,
    frosted: bool,
    px: Size<i32, BufferCoords>,
) -> Option<Vec<Rectangle<i32, BufferCoords>>> {
    (fourcc == Fourcc::Xrgb8888 && !frosted).then(|| vec![Rectangle::from_size(px)])
}

/// A picture's side in points from its side in pixels: the inverse of the
/// toolkit's pixel_size, which rounds up (580 points at 4/3 are 773.33
/// pixels, drawn as 774). Rounding to the nearest instead gave 581, a body
/// two pixels wider than the bar drawn for the 580 the window was given
/// (at 133⅓%): the largest number of points whose
/// pixels, rounded up, are still `px`.
pub fn points_of(px: i32, scale: f64) -> i32 {
    if !(scale.is_finite() && scale > 0.0) || px <= 0 {
        return px.max(0);
    }
    let up = |p: i32| (p as f64 * scale - vergence_render::PIXEL_SLACK).ceil() as i32;
    let mut p = (px as f64 / scale).floor() as i32;
    while p > 0 && up(p) > px {
        p -= 1;
    }
    while up(p + 1) <= px {
        p += 1;
    }
    p
}

/// The last GPU picture's serial (GpuPicture::serial).
static PICTURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl NativeWindow {
    /// The scale the window was last told (its screen's).
    pub fn scale(&self) -> f64 {
        self.state.lock().unwrap().scale
    }

    /// The text factor the window was last told (its screen's).
    pub fn text(&self) -> f64 {
        self.state.lock().unwrap().text
    }

    pub fn title(&self) -> String {
        self.state.lock().unwrap().title.clone()
    }

    /// Its meaning as last sent (Contract 3): the focused widget, the caret
    /// in window points, the tree.
    /// Its last display list, if any.
    pub fn scene(&self) -> Option<Arc<vergence_scene::Scene>> {
        self.state.lock().unwrap().scene.clone()
    }

    /// For vctl: its scenes so far, the last one's size, the pictures kept,
    /// and the last refusal.
    pub fn scene_report(&self) -> Option<String> {
        let st = self.state.lock().unwrap();
        if st.scenes == 0 && st.scene_refused.is_none() {
            return None;
        }
        let last = st
            .scene
            .as_ref()
            .map_or(String::new(), |s| format!(", {} commands, {:.0}x{:.0}", s.commands.len(), s.width, s.height));
        let refused =
            st.scene_refused.as_ref().map_or(String::new(), |(n, why)| format!(", {n} refused (last: {why})"));
        let off = st.scenes_off.as_ref().map_or(String::new(), |why| format!(", drawn by the app now ({why})"));
        Some(format!(
            "scenes {}{last}, {} pictures kept ({} KB){refused}{off}",
            st.scenes,
            st.scene_images.len(),
            st.scene_image_bytes >> 10
        ))
    }

    /// Its scenes no longer drawn (step 6): what it kept for them let go.
    pub(crate) fn set_scenes_off(&self, why: &str) {
        let mut st = self.state.lock().unwrap();
        st.scenes_off = Some(why.to_string());
        st.scene = None;
        st.scene_images.clear();
        st.scene_image_bytes = 0;
    }

    pub fn scenes_off(&self) -> bool {
        self.state.lock().unwrap().scenes_off.is_some()
    }

    /// A display list sent (Scene): read, its pictures resolved from those
    /// kept, checked (vergence-scene never returns an invalid scene), kept.
    fn take_scene(&self, fd: &vergence_protocol::window::Fd, len: u32, forget: &[u64]) -> Result<(), String> {
        if self.scenes_off() {
            return Err("its scenes are no longer drawn; pixels expected".into());
        }
        let bytes = read_memfd(fd, len)?;
        let mut st = self.state.lock().unwrap();
        let st = &mut *st;
        let scene = take_keyed(&mut st.scene_images, &mut st.scene_image_bytes, &bytes, forget)?;
        st.scene = Some(Arc::new(scene));
        st.scenes += 1;
        Ok(())
    }

    pub fn meaning(&self) -> Option<crate::meaning::WindowMeaning> {
        self.state.lock().unwrap().meaning.clone()
    }

    pub fn size(&self) -> Size<i32, Logical> {
        self.state.lock().unwrap().size
    }

    pub fn picture(&self) -> Option<Picture> {
        self.state.lock().unwrap().picture.clone()
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Returns whether it changed.
    pub fn set_activated(&self, on: bool) -> bool {
        let mut s = self.state.lock().unwrap();
        std::mem::replace(&mut s.activated, on) != on
    }

    pub fn user_data(&self) -> &UserDataMap {
        &self.user_data
    }

    pub fn cursor(&self) -> smithay::input::pointer::CursorIcon {
        self.state.lock().unwrap().cursor
    }

    /// The strip at the top of its pictures that lies under the server's bar,
    /// in points: the bar's height for a window that draws under its bar, else 0.
    pub fn inset(&self) -> i32 {
        self.state.lock().unwrap().inset
    }

    /// What its bar shows, as last sent (none: its title).
    pub fn bar(&self) -> Option<vergence_protocol::window::Bar> {
        self.state.lock().unwrap().bar.clone()
    }

    /// Its app menu's commands, and whether it is a terminal.
    pub fn commands(&self) -> (Vec<vergence_protocol::commands::Command>, bool) {
        let s = self.state.lock().unwrap();
        (s.commands.clone(), s.terminal)
    }

    /// The style it asked its bar to be drawn in.
    pub fn bar_style(&self) -> vergence_protocol::window::BarStyle {
        self.state.lock().unwrap().bar_style
    }

    /// Whether it asked for a frosted ground (bus v36; preview milestone 1,
    /// step 6b): the server frosts what is behind its whole body (decor.rs).
    pub fn frosted(&self) -> bool {
        self.state.lock().unwrap().style.is_some_and(|s| s.frosted)
    }

    /// Its parent's id on its connection (0: none) and what it said of itself.
    pub fn style(&self) -> (u32, Option<NativeStyle>) {
        let s = self.state.lock().unwrap();
        (s.parent, s.style)
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for NativeWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeWindow({}:{} {:?} {:?})", self.client, self.id, self.app_id, self.role)
    }
}

/// Copy the damaged parts of a client's shared-memory picture into `mem`
/// (tightly packed, `width * 4` bytes a row), reading the file with pread.
/// Returns the rectangles copied.
fn copy_damage(
    file: &std::fs::File,
    mem: &mut [u8],
    width: i32,
    height: i32,
    stride: u32,
    damage: &[Damage],
) -> std::io::Result<Vec<Rectangle<i32, BufferCoords>>> {
    let whole = Rectangle::<i32, BufferCoords>::from_size((width, height).into());
    let rects: Vec<Rectangle<i32, BufferCoords>> = if damage.is_empty() {
        vec![whole]
    } else {
        damage
            .iter()
            .filter_map(|d| Rectangle::new((d.x, d.y).into(), (d.w, d.h).into()).intersection(whole))
            .filter(|r| r.size.w > 0 && r.size.h > 0)
            .collect()
    };
    let row = width as usize * 4;
    let mut band = Vec::new();
    for r in &rects {
        // The rows the rectangle covers, read in one call, then its columns copied.
        let (y0, rows) = (r.loc.y as usize, r.size.h as usize);
        let len = (rows - 1) * stride as usize + row;
        band.resize(len, 0);
        file.read_exact_at(&mut band, y0 as u64 * stride as u64)?;
        let (x0, w) = (r.loc.x as usize * 4, r.size.w as usize * 4);
        for i in 0..rows {
            let from = i * stride as usize + x0;
            let to = (y0 + i) * row + x0;
            mem[to..to + w].copy_from_slice(&band[from..from + w]);
        }
    }
    Ok(rects)
}

impl Server {
    /// A window message from bus client `client` (bus.rs).
    pub fn native_request(&mut self, client: u32, req: WinRequest) {
        match req {
            WinRequest::Create { serial, window, role, token, title, width, height, parent, place } => {
                if window == 0 || self.natives.contains_key(&(client, window)) {
                    return self.native_refuse(
                        client,
                        serial,
                        ErrorCode::BadMessage,
                        format!("window id {window} is 0 or in use"),
                    );
                }
                if self.natives.keys().filter(|(c, _)| *c == client).count() >= MAX_WINDOWS {
                    return self.native_refuse(
                        client,
                        serial,
                        ErrorCode::Denied,
                        format!("at most {MAX_WINDOWS} windows"),
                    );
                }
                let side = |v: f64| if v.is_finite() { (v.round() as i32).clamp(MIN_SIDE, 16_384) } else { 640 };
                let asked: Size<i32, Logical> = (side(width), side(height)).into();
                let app_id = self.bus_app_id(client);
                // The window's persistence token (bar milestone 1, step 5): the one it
                // presents, if well formed and not already another open window's of
                // this program, else a new one. "new" asks for a new one from a
                // program that keeps tokens (a torn-off tab's window: no memory, so
                // the place it asks for wins); nothing presented, the program keeps
                // none and its windows go by numbering.
                let keeps_token = !token.is_empty();
                let presented =
                    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
                let taken =
                    self.natives.values().any(|w| w.native().is_some_and(|m| m.app_id == app_id && m.token == token));
                // A fresh token has no memory of its own: a place it asks for (a
                // torn-off tab's drop point) wins over its program's last-closed one.
                let fresh = !(presented && !taken);
                let token = if presented && !taken { token } else { new_token() };
                let n = Arc::new(NativeWindow {
                    client,
                    id: window,
                    app_id: app_id.clone(),
                    role: role.clone(),
                    token: token.clone(),
                    keeps_token,
                    alive: AtomicBool::new(true),
                    state: Mutex::new(State {
                        title,
                        size: asked,
                        scale: 1.0,
                        text: 1.0,
                        maximized: false,
                        activated: false,
                        sent_serial: 0,
                        acked_serial: 0,
                        pending: None,
                        place: Point::default(),
                        place_maximized: false,
                        shown: false,
                        picture: None,
                        frame_wanted: false,
                        cursor: smithay::input::pointer::CursorIcon::Default,
                        meaning: None,
                        bar: None,
                        under_bar: false,
                        inset: 0,
                        parent,
                        style: None,
                        commands: Vec::new(),
                        terminal: false,
                        bar_style: Default::default(),
                        scene: None,
                        scene_images: std::collections::HashMap::new(),
                        scene_image_bytes: 0,
                        scenes: 0,
                        scene_refused: None,
                        scenes_off: None,
                    }),
                    user_data: UserDataMap::new(),
                });
                let w = Win::Native(n.clone());
                // A portal's file dialog: another program's window is its parent.
                let pid = self.bus_pid(client);
                self.portal_adopt(&w, pid);
                // Where it goes: memory first (by identity, memory.rs), then where it
                // asked to be if that is allowed, else fitted and centered on the
                // monitor under the pointer (place.rs).
                let asked_place = self.native_place_rect(client, parent, &place, asked, &w);
                let remembered = if fresh && asked_place.is_some() { None } else { self.remembered_place(&w) };
                // Nor on top of another of its program's windows (a tab dropped
                // where the window it left is).
                let asked_place = asked_place.map(|r| {
                    match self.space.outputs().find_map(|o| self.space.output_geometry(o).filter(|g| g.contains(r.loc)))
                    {
                        Some(og) if fresh => self.clear_of_siblings(&w, r, og),
                        _ => r,
                    }
                });
                let (rect, maximized) = match remembered.or(asked_place.map(|r| (r, false))) {
                    Some(r) => r,
                    None => {
                        let out = self.output_at_pointer().unwrap_or_default();
                        let area = crate::place::content_area(
                            self.work_area(out),
                            if self.has_title(&w) { self.bar_h_in(out) } else { 0 },
                        );
                        let size = crate::place::fit(asked, area.size, Some((MIN_SIDE, MIN_SIDE).into()));
                        // Not exactly on another window of its program.
                        (self.clear_of_siblings(&w, Rectangle::new(crate::place::center(area, size), size), out), false)
                    }
                };
                let (scale, text) = self.screen_at(rect);
                // Its first size on the grid too (grid.rs); its place is when it is shown (put).
                let mut rect = rect;
                if !maximized && self.on_grid(&w) {
                    rect.size = Self::grid_size(rect.size, scale);
                }
                {
                    let mut s = n.state.lock().unwrap();
                    (s.size, s.scale, s.text, s.place, s.place_maximized) =
                        (rect.size, scale, text, rect.loc, maximized);
                }
                self.natives.insert((client, window), w.clone());
                let _ = self.window_identity(&w);
                tracing::info!(
                    "native: {app_id:?} window {window} ({role:?}) asked {}x{}, given {}x{} at scale {scale:.4}",
                    asked.w,
                    asked.h,
                    rect.size.w,
                    rect.size.h
                );
                self.bus_send(client, ServerMsg::Window(WinEvent::Created { serial, window, token }));
                self.native_send_configure(&n);
                // What is on the clipboard now, for a client that came after it was put there.
                if let Some(kinds) = self.native_input.clip_kinds.clone() {
                    self.bus_send(client, ServerMsg::Window(WinEvent::ClipboardOffer { kinds }));
                }
            }
            // The program closed the window itself: the user closed it, for good.
            // (A program that goes away, or a session that ends, sends no Destroy:
            // its windows are remembered for next time.)
            WinRequest::Destroy { window } => {
                if let Some(w) = self.natives.remove(&(client, window)) {
                    self.native_drop(&w);
                    self.forget_window(&w);
                }
                self.acts_fail(client, Some(window));
            }
            WinRequest::Title { window, title } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)) {
                    n.state.lock().unwrap().title = title;
                }
            }
            WinRequest::AckConfigure { window, serial } => {
                let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() else { return };
                let next = {
                    let mut s = n.state.lock().unwrap();
                    if serial > s.acked_serial && serial <= s.sent_serial {
                        s.acked_serial = serial;
                    }
                    if s.acked_serial == s.sent_serial { s.pending.take() } else { None }
                };
                if let Some((size, maximized)) = next {
                    self.native_configure(&n, size, maximized);
                }
            }
            WinRequest::Attach { window, buffer, damage } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                let Win::Native(n) = &w else { return };
                if let Err(e) = self.native_attach(n, buffer, &damage) {
                    tracing::warn!("native: client {client} window {window}: {e}; disconnecting");
                    return self.bus_end(client);
                }
                let n = n.clone();
                self.native_picture_came(&w, &n, window);
            }
            WinRequest::Maximize { window, on } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                if on != self.restore.contains_key(&w) {
                    self.toggle_maximize(&w);
                }
            }
            // The whole screen and back (bus v41), as a Wayland or X11
            // window's request is answered (fullscreen.rs).
            WinRequest::Fullscreen { window, on } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                if on {
                    self.enter_fullscreen(&w, None);
                } else {
                    self.leave_fullscreen(&w);
                }
            }
            // It draws under its bar (look milestone 1): told the inset at once.
            // Its kind, modal, limits (window styles milestone 1). A dialog or
            // palette of another connection's window is a normal window: a
            // program shapes only its own (§7.3). A dialog of no window is a
            // lone one (a Properties window, files milestone 2): a window of
            // its own, listed and stacked as one, with the slim close-only bar
            // (no app button); a palette of none is a normal window.
            WinRequest::Style { window, kind, modal, min, max, frosted } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() {
                    let parent = n.state.lock().unwrap().parent;
                    // A portal's dialog belongs to another program's window (portal.rs).
                    let adopted = self.portal.adopted.contains_key(&Win::Native(n.clone()));
                    let owned =
                        adopted || (parent != 0 && parent != window && self.natives.contains_key(&(client, parent)));
                    let kind = match kind {
                        vergence_protocol::window::WinKind::Dialog if parent == 0 && !adopted => {
                            vergence_protocol::window::WinKind::Dialog
                        }
                        vergence_protocol::window::WinKind::Dialog | vergence_protocol::window::WinKind::Palette
                            if !owned =>
                        {
                            vergence_protocol::window::WinKind::Normal
                        }
                        k => k,
                    };
                    let style = NativeStyle {
                        kind,
                        modal: modal && kind == vergence_protocol::window::WinKind::Dialog,
                        min,
                        max,
                        frosted,
                    };
                    tracing::info!("native: window {window} of {:?} is {style:?}", n.app_id);
                    n.state.lock().unwrap().style = Some(style);
                    // The desktop's icons: in their place, under every window (desktop.rs).
                    if kind == vergence_protocol::window::WinKind::Desktop {
                        let w = Win::Native(n.clone());
                        self.desktop_place(&w);
                    }
                    self.redraw_all();
                }
            }
            // Its app menu (app settings milestone 1): kept; the menu reads it when it opens.
            WinRequest::Commands { window, terminal, commands } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned()
                    && let Win::Native(n) = &w
                {
                    {
                        let mut s = n.state.lock().unwrap();
                        s.commands = commands;
                        s.terminal = terminal;
                    }
                    self.app_menu_refresh(&w);
                }
            }
            // A size asked for (bus v12): within its limits and its screen's work
            // area, kept where it is unless its right or bottom edge would leave the
            // screen; not while maximized, snapped or full screen.
            // Its right click, answered by the server's menu (bus v18; menu.rs).
            WinRequest::ContextMenu { window, x, y } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    let loc = self.space.element_location(&w).unwrap_or_default();
                    let at = smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                        loc.x as f64 + x,
                        loc.y as f64 + y,
                    ));
                    self.open_context_menu(&w, at);
                }
            }
            // The desktop's bare ground clicked (bus v23; aside.rs): only a
            // window of the Desktop kind says so.
            WinRequest::GroundClicked { window } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned()
                    && self.is_desktop(&w)
                {
                    self.ground_clicked();
                }
            }
            // Its own right-click menu by its lines (bus v20; appmenu.rs).
            WinRequest::Menu { window, x, y, lines } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    let loc = self.space.element_location(&w).unwrap_or_default();
                    let at = smithay::utils::Point::<f64, smithay::utils::Logical>::from((
                        loc.x as f64 + x,
                        loc.y as f64 + y,
                    ));
                    self.app_menu_lines(&w, at, lines);
                }
            }
            // Come forward (bus v16): by the rule new windows take the keyboard by.
            WinRequest::Activate { window } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    self.window_activate(&w);
                }
            }
            // Drag and drop (drag.rs, bus v16).
            WinRequest::DragStart { window, label, items } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    self.native_drag_start(w, label, items);
                }
            }
            WinRequest::DragAccept { window, action } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    self.native_drag_accept(&w, action);
                }
            }
            WinRequest::Size { window, width, height } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                let Win::Native(n) = &w else { return };
                if self.restore.contains_key(&w) || self.snap.snapped.contains_key(&w) {
                    return;
                }
                let Some(loc) = self.space.element_location(&w) else { return };
                let screen = self
                    .space
                    .outputs()
                    .filter_map(|o| self.space.output_geometry(o))
                    .find(|g| g.contains(loc))
                    .or_else(|| self.output_at_pointer());
                let mut size = self.within_limits(&w, Size::from((width.round() as i32, height.round() as i32)));
                let mut at = loc;
                // A dialog keeps its center (preview milestone 1: a preview shaped
                // for each file it shows stays where it was over its window).
                let dialog = self.window_style(&w).kind == crate::style::Kind::Dialog;
                // No bigger than 80% of the screen's work area, both sides shrunk
                // alike, so a picture's preview keeps its proportions.
                if dialog && let Some(area) = screen.map(|s| self.work_area(s)) {
                    let f = (0.8 * area.size.w as f64 / size.w.max(1) as f64)
                        .min(0.8 * (area.size.h - self.bar_h(&w)) as f64 / size.h.max(1) as f64)
                        .min(1.0);
                    size = Size::from(((size.w as f64 * f).round() as i32, (size.h as f64 * f).round() as i32));
                }
                if dialog && let Some(g) = self.space.element_geometry(&w) {
                    at.x += (g.size.w - size.w) / 2;
                    at.y += (g.size.h - size.h) / 2;
                }
                if let Some(area) = screen.map(|s| self.work_area(s)) {
                    let bar = self.bar_h(&w);
                    size.w = size.w.min(area.size.w);
                    size.h = size.h.min(area.size.h - bar);
                    at.x = at.x.min(area.loc.x + area.size.w - size.w).max(area.loc.x);
                    at.y = at.y.min(area.loc.y + area.size.h - size.h).max(area.loc.y + bar);
                }
                tracing::info!(
                    "native: {:?} window {window} asks {width}x{height}, given {}x{} at {},{}",
                    n.app_id,
                    size.w,
                    size.h,
                    at.x,
                    at.y
                );
                if at != loc {
                    self.put(w.clone(), at, false);
                }
                let maximized = n.state.lock().unwrap().maximized;
                self.native_configure(n, size, maximized);
            }
            WinRequest::BarStyle { window, style } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() {
                    let was = std::mem::replace(&mut n.state.lock().unwrap().bar_style, style);
                    if was != style {
                        self.redraw_all();
                    }
                }
            }
            WinRequest::UnderBar { window } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() {
                    let was = std::mem::replace(&mut n.state.lock().unwrap().under_bar, true);
                    if !was {
                        self.native_send_configure(&n);
                        self.redraw_all();
                    }
                }
            }
            // What the bar shows (bar milestone 1): kept, and drawn from step 3.
            WinRequest::Bar { window, bar } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() {
                    n.state.lock().unwrap().bar = Some(bar);
                    self.redraw_all();
                }
            }
            WinRequest::Cursor { window, shape } => {
                if let Some(Win::Native(n)) = self.natives.get(&(client, window)).cloned() {
                    n.state.lock().unwrap().cursor = cursor_icon(shape);
                    // Shown at once if the pointer is on it.
                    let id = self.win_id(&Win::Native(n.clone()));
                    if self.input.seat.pointer_focus() == Some(vergence_seat::Target::Native(id)) {
                        self.pointer_image.status =
                            smithay::input::pointer::CursorImageStatus::Named(cursor_icon(shape));
                    }
                }
            }
            WinRequest::Minimize { window } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    self.minimize(&w);
                }
            }
            WinRequest::Place { window, place } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                let Some(geo) = self.space.element_geometry(&w) else { return };
                match self.native_place_rect(client, 0, &place, geo.size, &w) {
                    Some(r) => {
                        tracing::info!(
                            "native: client {client} window {window} placed at {},{} as asked",
                            r.loc.x,
                            r.loc.y
                        );
                        self.put(w.clone(), r.loc, false);
                        self.remember(&w);
                    }
                    None => tracing::info!("native: client {client} window {window} asked for {place:?}: not allowed"),
                }
            }
            // An answer to an action (bus v24): kept for its sender; only the
            // window it was sent to can answer it.
            // A window as a display list (display lists milestone 1): kept;
            // drawn from step 2. A scene that fails is refused and logged,
            // the window keeps what it showed.
            WinRequest::Scene { window, fd, len, forget } => {
                let Some(w) = self.natives.get(&(client, window)).cloned() else { return };
                let Win::Native(n) = &w else { return };
                let n = n.clone();
                let taken = n.take_scene(&fd, len, &forget).and_then(|()| self.native_draw_scene(&n));
                match taken {
                    Ok(()) => self.native_picture_came(&w, &n, window),
                    Err(why) => {
                        // Switched off: said once already (scenes.rs scenes_off).
                        if !n.scenes_off() {
                            tracing::warn!("native: client {client} window {window}: scene refused: {why}");
                        }
                        let mut st = n.state.lock().unwrap();
                        let count = st.scene_refused.as_ref().map_or(0, |r| r.0) + 1;
                        st.scene_refused = Some((count, why));
                    }
                }
            }
            WinRequest::Acted { window, serial, ok, why } => {
                if let Some(entry) = self.acts.sent.get_mut(&serial)
                    && entry.0 == client
                    && entry.1 == window
                    && entry.2.is_none()
                {
                    let why: String = why.chars().take(200).collect();
                    entry.2 = Some((ok, why.clone()));
                    if let Some((agent, agent_serial)) = entry.3 {
                        self.act_logged(agent, serial, client, window, ok, &why);
                        self.bus_send(
                            agent,
                            vergence_protocol::bus::ServerMsg::Agent(vergence_protocol::agent::AgentEvent::Acted {
                                serial: agent_serial,
                                ok,
                                why,
                            }),
                        );
                    }
                }
            }
            WinRequest::TextPage { window, serial, node, found, offset, total, text } => {
                if let Some(&(c, w, agent, agent_serial, n, _)) = self.acts.reads.get(&serial)
                    && c == client
                    && w == window
                    && n == node
                {
                    self.acts.reads.remove(&serial);
                    let msg = if found {
                        vergence_protocol::bus::ServerMsg::Agent(vergence_protocol::agent::AgentEvent::Text {
                            serial: agent_serial,
                            client,
                            window,
                            node,
                            offset,
                            total,
                            text,
                        })
                    } else {
                        vergence_protocol::bus::ServerMsg::Error {
                            serial: agent_serial,
                            code: vergence_protocol::bus::ErrorCode::Failed,
                            message: "no such widget now (the window changed)".into(),
                        }
                    };
                    self.bus_send(agent, msg);
                }
            }
            WinRequest::Meaning { window, focus, caret, nodes } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned()
                    && let Win::Native(n) = &w
                {
                    let new = (focus, caret, sane_tree(nodes));
                    // Shell access, once said, holds for the window's life.
                    if crate::meaning::shell_access(&new.2) || n.app_id == "vergence-term" {
                        self.agents.shell_windows.insert((client, window));
                    }
                    let old = n.state.lock().unwrap().meaning.replace(new.clone());
                    let origin = self.meaning_origin(&w);
                    self.meaning_changed(client, window, old.as_ref(), &new, origin);
                    // A terminal started for an agent: its grant goes to its
                    // tab, in the first tree that has one (not only the first).
                    self.agent_terminal_arrived(client, window, &new.2);
                }
            }
            // The person stopped an agent in a tab (bus v38: Escape twice, the
            // tab mark's click): every agent's grant there ends now.
            WinRequest::RevokeAgent { window, node, agent } => {
                let n = self.agent_revoke_tab(client, window, node);
                tracing::info!(
                    "agents: the person stopped {agent:?} in tab #{node} of {client}/{window} ({n} grant(s))"
                );
            }
            // Every key to this window while it has the keyboard (bus v40,
            // Vergence Remote's viewer): the seat decides (vergence-seat's
            // capture_keys); the window hears the answer.
            WinRequest::CaptureKeys { window, on } => {
                if let Some(w) = self.natives.get(&(client, window)).cloned() {
                    let id = self.win_id(&w);
                    self.input.seat.capture_keys(id, on);
                    tracing::info!("keys: {client}/{window} {} every key", if on { "takes" } else { "gives back" });
                    self.bus_send(
                        client,
                        vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::KeysCaptured {
                            window,
                            on,
                        }),
                    );
                }
            }
            WinRequest::Copy { kinds, .. } => self.native_copy(client, kinds),
            WinRequest::Paste { serial, kind } => self.native_paste(client, serial, kind),
        }
    }

    /// Where a window that asked for `place` would go with `size` (content),
    /// if the request is allowed: wholly on a monitor, below its title bar if
    /// the server draws one. None: Anywhere, an unknown monitor or parent, or
    /// a place that would leave it partly off every monitor.
    fn native_place_rect(
        &self,
        client: u32,
        parent: u32,
        place: &vergence_protocol::window::Place,
        size: Size<i32, Logical>,
        w: &Win,
    ) -> Option<Rectangle<i32, Logical>> {
        use vergence_protocol::window::Place;
        let finite = |v: f64| v.is_finite().then_some(v.round() as i32);
        let at: Point<i32, Logical> = match place {
            Place::Anywhere => return None,
            Place::Monitor { id, x, y } => {
                let out =
                    self.space.outputs().find(|o| crate::memory::monitor_id_of(o).as_deref() == Some(id.as_str()))?;
                let og = self.space.output_geometry(out)?;
                (og.loc.x + finite(*x)?, og.loc.y + finite(*y)?).into()
            }
            Place::Parent { x, y } => {
                let p = self.natives.get(&(client, parent))?;
                let loc = self.space.element_location(p)?;
                (loc.x + finite(*x)?, loc.y + finite(*y)?).into()
            }
        };
        let r = Rectangle::new(at, size);
        self.fits_a_monitor(r, self.has_title(w)).then_some(r)
    }

    /// For `vctl tree`: every native window's meaning as it last sent it,
    /// indented by the tree, the focused widget marked. Values are shown;
    /// a secret field has none (the toolkit never sends it).
    /// Sends `action` to the widget with meaning node `node` in native
    /// window `w`: the serial its answer will carry, or None if `w` is not
    /// a native window.
    pub fn send_act(
        &mut self,
        w: &Win,
        node: u64,
        action: vergence_protocol::window::Action,
        reply: Option<(u32, u32)>,
    ) -> Option<u32> {
        let n = w.native()?;
        let (client, window) = (n.client, n.id);
        self.acts.next = self.acts.next.wrapping_add(1).max(1);
        let serial = self.acts.next;
        self.acts.sent.insert(serial, (client, window, None, reply, std::time::Instant::now()));
        while self.acts.sent.len() > ACTS_KEPT {
            // The oldest goes; an agent still waiting on it hears so.
            if let Some((_, (_, _, None, Some((agent, s)), _))) = self.acts.sent.pop_first() {
                self.bus_send(
                    agent,
                    vergence_protocol::bus::ServerMsg::Agent(vergence_protocol::agent::AgentEvent::Acted {
                        serial: s,
                        ok: false,
                        why: "too many actions at once".into(),
                    }),
                );
            }
        }
        self.bus_send(
            client,
            vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::Act {
                window,
                serial,
                node,
                action,
            }),
        );
        Some(serial)
    }

    /// Asks native window `w` for a page of node `node`'s text for an agent
    /// (bus v37): the answer goes to bus client `agent` with its `serial`.
    /// None if `w` is not a native window.
    pub fn send_read(&mut self, w: &Win, node: u64, offset: u32, len: u32, agent: u32, serial: u32) -> Option<u32> {
        let n = w.native()?;
        let (client, window) = (n.client, n.id);
        self.acts.next = self.acts.next.wrapping_add(1).max(1);
        let s = self.acts.next;
        self.acts.reads.insert(s, (client, window, agent, serial, node, std::time::Instant::now()));
        while self.acts.reads.len() > ACTS_KEPT {
            if let Some((_, (_, _, agent, serial, _, _))) = self.acts.reads.pop_first() {
                self.read_failed(agent, serial, "too many reads at once");
            }
        }
        let len = len.min(vergence_protocol::window::TEXT_PAGE_MAX);
        self.bus_send(
            client,
            vergence_protocol::bus::ServerMsg::Window(vergence_protocol::window::WinEvent::ReadText {
                window,
                serial: s,
                node,
                offset,
                len,
            }),
        );
        Some(s)
    }

    fn read_failed(&mut self, agent: u32, serial: u32, why: &str) {
        self.bus_send(
            agent,
            vergence_protocol::bus::ServerMsg::Error {
                serial,
                code: vergence_protocol::bus::ErrorCode::Failed,
                message: why.into(),
            },
        );
    }

    /// Actions still unanswered by a window that went (or by every window
    /// of a client that left: `window` None) fail, and their agents hear why.
    pub fn acts_fail(&mut self, client: u32, window: Option<u32>) {
        let gone: Vec<u32> = self
            .acts
            .reads
            .iter()
            .filter(|(_, r)| r.0 == client && window.is_none_or(|w| w == r.1))
            .map(|(s, _)| *s)
            .collect();
        for s in gone {
            if let Some((_, _, agent, serial, _, _)) = self.acts.reads.remove(&s) {
                self.read_failed(agent, serial, "the window went away");
            }
        }
        let mut tell = Vec::new();
        for (act, e) in self.acts.sent.iter_mut() {
            if e.0 == client && window.is_none_or(|w| w == e.1) && e.2.is_none() {
                e.2 = Some((false, "the window went away".into()));
                tell.extend(e.3.map(|(a, s)| (a, s, *act, e.0, e.1)));
            }
        }
        for (agent, serial, act, c, w) in tell {
            self.act_logged(agent, act, c, w, false, "the window went away");
            self.bus_send(
                agent,
                vergence_protocol::bus::ServerMsg::Agent(vergence_protocol::agent::AgentEvent::Acted {
                    serial,
                    ok: false,
                    why: "the window went away".into(),
                }),
            );
        }
    }

    /// Actions their windows have not answered in time fail (each second,
    /// with the dock's tick), and their agents hear so.
    pub fn acts_timeout(&mut self) {
        let late: Vec<u32> =
            self.acts.reads.iter().filter(|(_, r)| r.5.elapsed() > ACT_TIMEOUT).map(|(s, _)| *s).collect();
        for s in late {
            if let Some((_, _, agent, serial, _, _)) = self.acts.reads.remove(&s) {
                self.read_failed(agent, serial, "the window did not answer");
            }
        }
        let mut tell = Vec::new();
        for (act, e) in self.acts.sent.iter_mut() {
            if e.2.is_none() && e.4.elapsed() > ACT_TIMEOUT {
                e.2 = Some((false, "the window did not answer".into()));
                tell.extend(e.3.map(|(a, s)| (a, s, *act, e.0, e.1)));
            }
        }
        for (agent, serial, act, c, w) in tell {
            self.act_logged(agent, act, c, w, false, "the window did not answer");
            self.bus_send(
                agent,
                vergence_protocol::bus::ServerMsg::Agent(vergence_protocol::agent::AgentEvent::Acted {
                    serial,
                    ok: false,
                    why: "the window did not answer".into(),
                }),
            );
        }
    }

    /// The window's answer to an agent's act, in agents.log under the same
    /// act number (the server's, unique in a session) as the line that asked.
    fn act_logged(&mut self, agent: u32, act: u32, client: u32, window: u32, ok: bool, why: &str) {
        let app = match self.natives.get(&(client, window)) {
            Some(Win::Native(n)) => n.app_id.clone(),
            _ => String::new(),
        };
        self.agent_log(agent, &app, &crate::meaning::act_answer_line(act, ok, why));
    }

    /// An action's answer: None while it has not come (or the serial is unknown).
    pub fn act_answer(&self, serial: u32) -> Option<Option<(bool, String)>> {
        self.acts.sent.get(&serial).map(|e| e.2.clone())
    }

    pub fn native_tree_report(&self) -> String {
        let mut out = Vec::new();
        let mut keys: Vec<&(u32, u32)> = self.natives.keys().collect();
        keys.sort();
        for k in keys {
            let Some(Win::Native(n)) = self.natives.get(k) else { continue };
            let s = n.state.lock().unwrap();
            // Its index as `vctl windows` and `vctl act` count (the stacking order).
            let index = self
                .space
                .elements()
                .position(|e| e.native().is_some_and(|m| m.client == n.client && m.id == n.id))
                .map_or("-".to_string(), |i| i.to_string());
            out.push(format!("window {index} {:?} ({}, client {} window {})", s.title, n.app_id, n.client, n.id));
            let Some((focus, caret, nodes)) = &s.meaning else {
                out.push("  (no meaning sent)".into());
                continue;
            };
            let depth_of = |mut id: u64| -> usize {
                let mut d = 0;
                while let Some(p) = nodes.iter().find(|m| m.id == id).map(|m| m.parent).filter(|p| *p != 0) {
                    d += 1;
                    id = p;
                    if d > 64 {
                        break;
                    }
                }
                d
            };
            for m in nodes {
                let value = m.value.as_ref().map(|v| format!(" = {v:?}")).unwrap_or_default();
                let states = if m.states.is_empty() { String::new() } else { format!(" [{}]", m.states.join(", ")) };
                let mark = if m.id == *focus { "> " } else { "  " };
                out.push(format!(
                    "{mark}{}#{} {} {:?}{value}{states} at {:.0},{:.0} {:.0}x{:.0}",
                    "  ".repeat(depth_of(m.id)),
                    m.id,
                    m.role,
                    m.name,
                    m.x,
                    m.y,
                    m.w,
                    m.h
                ));
            }
            if let Some((x, y, w, h)) = caret {
                out.push(format!("  caret at {x:.0},{y:.0} {w:.0}x{h:.0}"));
            }
            out.push(format!(
                "  {} widgets, focus {}",
                nodes.len(),
                if *focus == 0 { "none".to_string() } else { format!("{focus:#x}") }
            ));
        }
        if out.is_empty() { "no native windows".into() } else { out.join("\n") }
    }

    fn native_refuse(&mut self, client: u32, serial: u32, code: ErrorCode, message: String) {
        self.bus_send(client, ServerMsg::Error { serial, code, message });
    }

    /// Read a picture into the window's buffer.
    fn native_attach(&mut self, n: &NativeWindow, buffer: Buffer, damage: &[Damage]) -> Result<(), String> {
        let (fd, width, height, stride, format) = match buffer {
            Buffer::Shm { fd, width, height, stride, format } => (fd, width, height, stride, format),
            Buffer::Dmabuf { planes, width, height, format, modifier } => {
                return self.native_attach_gpu(n, planes, width, height, format, modifier);
            }
        };
        let fourcc = match format {
            SHM_ARGB8888 => Fourcc::Argb8888,
            SHM_XRGB8888 => Fourcc::Xrgb8888,
            other => return Err(format!("picture format {other:#x} is not one the server shows")),
        };
        // The decoder checked the numbers (sizes, stride); the file must hold them.
        let need = stride as u64 * height as u64;
        let file = std::fs::File::from(fd.0.as_fd().try_clone_to_owned().map_err(|e| e.to_string())?);
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len < need {
            return Err(format!("picture of {need} bytes in a file of {len}"));
        }
        let (w, h) = (width as i32, height as i32);
        let px: Size<i32, BufferCoords> = (w, h).into();
        let mut s = n.state.lock().unwrap();
        let opaque = opaque_region(fourcc, s.style.is_some_and(|st| st.frosted), px);
        let reuse = s.picture.as_ref().is_some_and(|p| {
            p.px == px && p.format == format && p.opaque == opaque.is_some() && matches!(p.content, Content::Memory(_))
        });
        if !reuse {
            let declared = opaque.is_some();
            let buffer = MemoryRenderBuffer::new(fourcc, px, 1, Transform::Normal, opaque);
            s.picture = Some(Picture {
                content: Content::Memory(buffer),
                px,
                logical: Size::default(),
                format,
                opaque: declared,
            });
        }
        let scale = s.scale;
        let p = s.picture.as_mut().unwrap();
        let Content::Memory(buffer) = &mut p.content else { unreachable!("a memory picture was just made") };
        // A new buffer starts empty: all of the picture is copied.
        let damage = if reuse { damage } else { &[] };
        buffer
            .render()
            .draw(|mem| copy_damage(&file, mem, w, h, stride, damage))
            .map_err(|e| format!("reading the picture: {e}"))?;
        p.logical = (points_of(w, scale), points_of(h, scale)).into();
        Ok(())
    }

    /// A new picture came (attached, or drawn from a scene): the next frame
    /// is wanted, and the first one puts the window on the screen.
    fn native_picture_came(&mut self, w: &Win, n: &Arc<NativeWindow>, window: u32) {
        let show = {
            let mut s = n.state.lock().unwrap();
            s.frame_wanted = true;
            let was_shown = std::mem::replace(&mut s.shown, true);
            !was_shown
        };
        if show {
            let (place, maximized) = {
                let s = n.state.lock().unwrap();
                (s.place, s.place_maximized)
            };
            self.put(w.clone(), place, true);
            if maximized {
                self.toggle_maximize(w);
            }
            // The desktop's icons: their place is the work area, not where it was asked (desktop.rs).
            if self.is_desktop(w) {
                self.desktop_place(w);
            }
            tracing::info!("native: {:?} window {window} shown at {},{}", n.app_id, place.x, place.y);
            self.window_opened(w);
        }
    }

    /// Pixels drawn by the server (a scene on the CPU: RGBA, premultiplied)
    /// as the window's picture.
    pub(crate) fn native_attach_pixels(
        &mut self,
        n: &NativeWindow,
        px: Size<i32, BufferCoords>,
        rgba: &[u8],
    ) -> Result<(), String> {
        let buffer = MemoryRenderBuffer::from_slice(rgba, Fourcc::Abgr8888, px, 1, Transform::Normal, None);
        let mut s = n.state.lock().unwrap();
        let logical = (points_of(px.w, s.scale), points_of(px.h, s.scale)).into();
        s.picture = Some(Picture { content: Content::Memory(buffer), px, logical, format: 0, opaque: false });
        Ok(())
    }

    /// A GPU buffer: checked and kept, to be imported when the scene is built.
    pub(crate) fn native_attach_gpu(
        &mut self,
        n: &NativeWindow,
        planes: Vec<vergence_protocol::window::Plane>,
        width: u32,
        height: u32,
        format: u32,
        modifier: u64,
    ) -> Result<(), String> {
        use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
        let fourcc = Fourcc::try_from(format).map_err(|_| format!("GPU picture format {format:#x} is unknown"))?;
        let mut b = Dmabuf::builder(
            (width as i32, height as i32),
            fourcc,
            smithay::backend::allocator::Modifier::from(modifier),
            DmabufFlags::empty(),
        );
        for p in planes {
            if !b.add_plane(p.fd.0.clone(), p.offset, p.stride) {
                return Err("too many planes".into());
            }
        }
        let dmabuf = b.build().ok_or("a GPU picture with no planes")?;
        let px: Size<i32, BufferCoords> = (width as i32, height as i32).into();
        let mut s = n.state.lock().unwrap();
        let logical = (points_of(width as i32, s.scale), points_of(height as i32, s.scale)).into();
        let serial = PICTURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let gpu = GpuPicture { dmabuf, texture: None, gles: None, failed: false, serial };
        s.picture =
            Some(Picture { content: Content::Gpu(Arc::new(Mutex::new(gpu))), px, logical, format, opaque: false });
        Ok(())
    }

    /// The window is gone: remembered, taken off the screen, forgotten.
    fn native_drop(&mut self, w: &Win) {
        let Win::Native(n) = w else { return };
        if n.state.lock().unwrap().shown {
            self.remember(w);
        }
        // Its parent, while the window is still known (a dialog's).
        let parent = self.window_style(w).parent.filter(|p| smithay::utils::IsAlive::alive(p) && p != w);
        self.space.unmap_elem(w);
        self.restore.remove(w);
        let had_keys = self.input.keyboard_native.as_ref() == Some(w);
        if had_keys {
            self.input.keyboard_native = None;
        }
        n.kill();
        self.scenes_forget(n.client, n.id);
        // The keyboard goes back where the person was (without this, keys went
        // nowhere after the editor's Open dialog closed): to a dialog's
        // parent, else the window in front.
        if had_keys {
            let next = parent
                .filter(|p| self.space.element_geometry(p).is_some())
                .or_else(|| self.space.elements().filter(|e| *e != w).next_back().cloned());
            if let Some(next) = next {
                self.focus_window(&next, smithay::utils::SERIAL_COUNTER.next_serial());
            }
        }
        tracing::info!("native: {:?} window {} closed", n.app_id, n.id);
    }

    /// Bus client `client` has gone: its windows go with it.
    pub fn native_forget(&mut self, client: u32) {
        self.native_input.keymap_sent.remove(&client);
        self.acts_fail(client, None);
        // What it had put on the clipboard goes with it.
        use smithay::wayland::selection::data_device::{
            clear_data_device_selection, current_data_device_selection_userdata,
        };
        let owned = current_data_device_selection_userdata(&self.seat).is_some_and(|o| *o == ClipOwner::Native(client));
        if owned {
            clear_data_device_selection(&self.display_handle, &self.seat);
            self.native_clipboard_offer(Vec::new());
        }
        let mine: Vec<(u32, u32)> = self.natives.keys().filter(|(c, _)| *c == client).copied().collect();
        for k in mine {
            if let Some(w) = self.natives.remove(&k) {
                self.native_drop(&w);
            }
        }
    }

    /// Tell the window its size, scale and state (paced: one configure waits
    /// for the client to acknowledge the last).
    pub fn native_configure(&mut self, n: &NativeWindow, size: Size<i32, Logical>, maximized: bool) {
        // On whole pixels at a fractional scale (grid.rs); a maximized
        // window fills its work area as it is.
        let on_grid = !maximized && self.natives.get(&(n.client, n.id)).is_some_and(|w| self.on_grid(w));
        let size = if on_grid { Self::grid_size(size, n.scale()) } else { size };
        {
            let mut s = n.state.lock().unwrap();
            if s.acked_serial != s.sent_serial {
                s.pending = Some((size, maximized));
                return;
            }
            if s.size == size && s.maximized == maximized {
                return;
            }
            (s.size, s.maximized) = (size, maximized);
        }
        self.native_send_configure(n);
    }

    pub(crate) fn native_send_configure(&mut self, n: &NativeWindow) {
        let bar = self.natives.get(&(n.client, n.id)).cloned().map_or(crate::decor::TITLE_H, |w| self.bar_h(&w));
        let (serial, size, scale, maximized, activated, inset, text) = {
            let mut s = n.state.lock().unwrap();
            s.sent_serial = s.sent_serial.wrapping_add(1).max(1);
            s.inset = if s.under_bar { bar } else { 0 };
            (s.sent_serial, s.size, s.scale, s.maximized, s.activated, s.inset as f64, s.text)
        };
        let ev = WinEvent::Configure {
            window: n.id,
            serial,
            width: size.w as f64,
            height: size.h as f64,
            scale,
            maximized,
            activated,
            inset,
            text,
        };
        self.bus_send(n.client, ServerMsg::Window(ev));
    }

    /// Ask the client to close the window (its close button, the dock's menu).
    pub fn native_close(&mut self, n: &NativeWindow) {
        self.bus_send(n.client, ServerMsg::Window(WinEvent::Close { window: n.id }));
    }

    /// The scale and text factor (displays.rs, Scales::text) of the monitor
    /// a rectangle is mostly on.
    fn screen_at(&self, rect: Rectangle<i32, Logical>) -> (f64, f64) {
        let of = |o: &Output| {
            (o.current_scale().fractional_scale(), self.scale_notes.get(&o.name()).map_or(1.0, |s| s.text))
        };
        self.space
            .outputs()
            .filter_map(|o| {
                let g = self.space.output_geometry(o)?;
                let i = g.intersection(rect)?;
                Some((i.size.w as i64 * i.size.h as i64, of(o)))
            })
            .max_by_key(|(a, _)| *a)
            .map(|(_, s)| s)
            .or_else(|| self.space.outputs().next().map(of))
            .unwrap_or((1.0, 1.0))
    }

    /// `output` has been drawn: windows on it that wait for a frame hear so,
    /// and a window that has moved to a monitor of another scale is told it.
    pub fn native_frames(&mut self, output: &Output) {
        let time_ms = self.start_time.elapsed().as_millis() as u32;
        let shown: Vec<Win> = self.space.elements().filter(|w| matches!(w, Win::Native(_))).cloned().collect();
        for w in shown {
            let Win::Native(n) = &w else { continue };
            if !self.space.outputs_for_element(&w).contains(output) {
                continue;
            }
            if std::mem::take(&mut n.state.lock().unwrap().frame_wanted) {
                self.bus_send(n.client, ServerMsg::Window(WinEvent::Frame { window: n.id, time_ms }));
            }
            let Some(geo) = self.space.element_geometry(&w) else { continue };
            let (scale, text) = self.screen_at(geo);
            let changed = {
                let mut s = n.state.lock().unwrap();
                let moved = (s.scale - scale).abs() > 1e-6 || (s.text - text).abs() > 1e-6;
                if moved {
                    (s.scale, s.text) = (scale, text);
                }
                moved
            };
            if changed {
                tracing::info!("native: {:?} window {} now at scale {scale:.4}, text {text:.3}", n.app_id, n.id);
                // On the new screen's grid (grid.rs): its size again, then its place.
                let (size, maximized) = {
                    let s = n.state.lock().unwrap();
                    (s.size, s.maximized)
                };
                let snapped = if !maximized && self.on_grid(&w) { Self::grid_size(size, scale) } else { size };
                if snapped != size {
                    self.native_configure(n, snapped, maximized);
                } else {
                    self.native_send_configure(n);
                }
                // Moved only when off the grid: map_element also raises it.
                if !maximized && self.grid_loc(&w, geo.loc) != geo.loc {
                    self.put(w.clone(), geo.loc, false);
                }
            }
        }
    }
}

/// The cursor-shape protocol's numbers (wp_cursor_shape_device_v1), which
/// the native protocol uses too, as the server draws them.
pub fn cursor_icon(shape: u16) -> smithay::input::pointer::CursorIcon {
    use smithay::input::pointer::CursorIcon as C;
    match shape {
        2 => C::ContextMenu,
        3 => C::Help,
        4 => C::Pointer,
        5 => C::Progress,
        6 => C::Wait,
        7 => C::Cell,
        8 => C::Crosshair,
        9 => C::Text,
        10 => C::VerticalText,
        11 => C::Alias,
        12 => C::Copy,
        13 => C::Move,
        14 => C::NoDrop,
        15 => C::NotAllowed,
        16 => C::Grab,
        17 => C::Grabbing,
        18 => C::EResize,
        19 => C::NResize,
        20 => C::NeResize,
        21 => C::NwResize,
        22 => C::SResize,
        23 => C::SeResize,
        24 => C::SwResize,
        25 => C::WResize,
        26 => C::EwResize,
        27 => C::NsResize,
        28 => C::NeswResize,
        29 => C::NwseResize,
        30 => C::ColResize,
        31 => C::RowResize,
        32 => C::AllScroll,
        33 => C::ZoomIn,
        34 => C::ZoomOut,
        _ => C::Default,
    }
}

impl Server {
    /// The pointer came onto a native window (the seat's Enter, dispatch.rs).
    pub fn native_enter(&mut self, w: &Win, pos: Point<f64, Logical>) {
        let Win::Native(n) = w else { return };
        let (x, y) = self.native_local(n, pos);
        let serial = smithay::utils::SERIAL_COUNTER.next_serial().into();
        self.bus_send(n.client, ServerMsg::Window(WinEvent::PointerEnter { window: n.id, serial, x, y }));
    }

    /// The pointer left it.
    pub fn native_leave(&mut self, w: &Win) {
        let Win::Native(n) = w else { return };
        self.bus_send(n.client, ServerMsg::Window(WinEvent::PointerLeave { window: n.id }));
    }

    /// The pointer moved on it (or anywhere, while a button pressed on it is held).
    pub fn native_motion(&mut self, w: &Win, pos: Point<f64, Logical>, time_ms: u32) {
        let Win::Native(n) = w else { return };
        let (x, y) = self.native_local(n, pos);
        self.bus_send(n.client, ServerMsg::Window(WinEvent::PointerMotion { window: n.id, time_ms, x, y }));
    }

    /// A point of the desktop in a native window's reference units.
    fn native_local(&self, n: &Arc<NativeWindow>, pos: Point<f64, Logical>) -> (f64, f64) {
        let loc = self.space.element_location(&Win::Native(n.clone())).unwrap_or_default();
        (pos.x - loc.x as f64, pos.y - loc.y as f64)
    }

    /// A button, pressed or let go, for it.
    pub fn native_button(&mut self, w: &Win, button: u32, pressed: bool, serial: u32, time_ms: u32) {
        let Win::Native(n) = w else { return };
        self.bus_send(
            n.client,
            ServerMsg::Window(WinEvent::PointerButton { window: n.id, serial, time_ms, button, pressed }),
        );
    }

    /// The wheel or a touchpad's scroll, for it.
    pub fn native_axis(&mut self, w: &Win, time_ms: u32, source: u8, dx: f64, dy: f64, v120x: i32, v120y: i32) {
        let Win::Native(n) = w else { return };
        self.bus_send(
            n.client,
            ServerMsg::Window(WinEvent::PointerAxis { window: n.id, time_ms, source, dx, dy, v120x, v120y }),
        );
    }

    /// The key repeat a native window is told (settings.rs keeps it).
    fn native_repeat(&self) -> (i32, i32) {
        self.key_repeat()
    }

    /// The keyboard came to a native window (input.rs, focus_window): the
    /// keymap first, if this client hasn't had it, and the repeat, then enter.
    pub fn native_keyboard_enter(&mut self, w: &Win, serial: u32) {
        let Win::Native(n) = w else { return };
        if self.native_input.keymap_sent.insert(n.client) {
            match self.native_keymap() {
                Ok((fd, size)) => self.bus_send(n.client, ServerMsg::Window(WinEvent::Keymap { fd, size })),
                Err(e) => tracing::warn!("native: keymap: {e}"),
            }
            let (rate, delay) = self.native_repeat();
            self.bus_send(n.client, ServerMsg::Window(WinEvent::Repeat { rate, delay }));
        }
        self.bus_send(n.client, ServerMsg::Window(WinEvent::KeyboardEnter { window: n.id, serial }));
    }

    pub fn native_keyboard_leave(&mut self, w: &Win) {
        let Win::Native(n) = w else { return };
        self.bus_send(n.client, ServerMsg::Window(WinEvent::KeyboardLeave { window: n.id }));
    }

    /// A key: the evdev code (xkb's minus 8), as Wayland sends it.
    pub fn native_key(&mut self, w: &Win, xkb_code: u32, pressed: bool, serial: u32, time_ms: u32) {
        let Win::Native(n) = w else { return };
        let key = xkb_code.saturating_sub(8);
        self.bus_send(n.client, ServerMsg::Window(WinEvent::Key { window: n.id, serial, time_ms, key, pressed }));
    }

    /// The modifiers now, from our own xkb state (keys.rs).
    pub fn native_modifiers(&mut self, w: &Win) {
        let Win::Native(n) = w else { return };
        let Some((depressed, latched, locked, group)) = self.input.keys.as_ref().map(|k| k.serialized()) else {
            return;
        };
        self.bus_send(n.client, ServerMsg::Window(WinEvent::Modifiers { depressed, latched, locked, group }));
    }

    /// The keymap (keys.rs: Smithay's own, read before the first key), NUL-terminated as Wayland's,
    /// in an anonymous file the client maps read-only (vergence_protocol::shm).
    fn native_keymap(&mut self) -> std::io::Result<(vergence_protocol::window::Fd, u32)> {
        let text = self
            .input
            .keys
            .as_ref()
            .map(|k| k.text.clone())
            .ok_or_else(|| std::io::Error::other("keymap not read yet"))?;
        let mut bytes = text.into_bytes();
        bytes.push(0);
        let mut shm = vergence_protocol::shm::ShmFile::new(bytes.len())?;
        shm.bytes().copy_from_slice(&bytes);
        // A descriptor of our own for the message: the file stays when the
        // mapping goes with `shm`.
        // SAFETY: shm.fd() is open for as long as `shm`, which outlives this borrow.
        let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(shm.fd()) }.try_clone_to_owned()?;
        Ok((vergence_protocol::window::Fd(Arc::new(fd)), bytes.len() as u32))
    }

    /// A finger on a native window (touch.rs): `phase` 0 down, 1 moved, 2 up,
    /// 3 canceled, at a point of the desktop given in the window's units.
    pub fn native_touch(&mut self, w: &Win, id: i32, phase: u8, at: Point<f64, Logical>, time_ms: u32) {
        let Win::Native(n) = w else { return };
        let (x, y) = self.native_local(n, at);
        self.bus_send(n.client, ServerMsg::Window(WinEvent::Touch { window: n.id, time_ms, id, phase, x, y }));
    }

    /// A native client copies: what it offers becomes the clipboard, for
    /// Wayland clients (Smithay's server-side selection) and X11 alike. Only
    /// a client whose window has the keyboard may (§7.3: no clipboard writes
    /// from the background).
    fn native_copy(&mut self, client: u32, kinds: Vec<String>) {
        use smithay::wayland::selection::{SelectionTarget, data_device::set_data_device_selection};
        if !self.native_has_keyboard(client) {
            return tracing::debug!("native: client {client} copied without the keyboard; ignored");
        }
        tracing::info!("native: client {client} copied {kinds:?}");
        set_data_device_selection(&self.display_handle, &self.seat, kinds.clone(), ClipOwner::Native(client));
        if let Some(xwm) = self.xwm.as_mut() {
            let _ = xwm.new_selection(SelectionTarget::Clipboard, Some(kinds.clone()));
        }
        self.native_clipboard_offer(kinds);
    }

    /// Whether native client `client` owns the window with the keyboard: the
    /// rule for both ends of the clipboard (§7.3, read only on the user's
    /// paste or by the focused app).
    fn native_has_keyboard(&self, client: u32) -> bool {
        clipboard_allowed(self.keyboard_window().and_then(|w| w.native().map(|n| n.client)), client)
    }

    /// The server puts text on the clipboard itself (Copy or Cut in the app
    /// menu's search field), for every kind of client.
    pub fn server_copy(&mut self, text: String) {
        use smithay::wayland::selection::{SelectionTarget, data_device::set_data_device_selection};
        let kinds: Vec<String> = TEXT_KINDS.iter().map(|k| k.to_string()).collect();
        set_data_device_selection(&self.display_handle, &self.seat, kinds.clone(), ClipOwner::Server(Arc::new(text)));
        if let Some(xwm) = self.xwm.as_mut() {
            let _ = xwm.new_selection(SelectionTarget::Clipboard, Some(kinds.clone()));
        }
        self.native_clipboard_offer(kinds);
    }

    /// The server puts a picture (PNG bytes) on the clipboard itself (a
    /// screenshot with Ctrl), for every kind of client.
    pub fn server_copy_picture(&mut self, png: Vec<u8>) {
        use smithay::wayland::selection::{SelectionTarget, data_device::set_data_device_selection};
        let kinds = vec!["image/png".to_string()];
        set_data_device_selection(&self.display_handle, &self.seat, kinds.clone(), ClipOwner::Picture(Arc::new(png)));
        if let Some(xwm) = self.xwm.as_mut() {
            let _ = xwm.new_selection(SelectionTarget::Clipboard, Some(kinds.clone()));
        }
        self.native_clipboard_offer(kinds);
    }

    /// The clipboard's text for the server itself (Paste in the app menu's
    /// search field): asked of whoever holds it through a pipe, read on a
    /// thread, and handed to `then` on the main loop.
    pub fn server_paste(&mut self, then: fn(&mut Server, String)) {
        use smithay::wayland::selection::{
            SelectionTarget,
            data_device::{current_data_device_selection_userdata, request_data_device_client_selection},
        };
        let owner = current_data_device_selection_userdata(&self.seat).map(|o| o.clone());
        if let Some(ClipOwner::Server(text)) = &owner {
            return then(self, text.to_string());
        }
        let kind = self
            .native_input
            .clip_kinds
            .as_ref()
            .and_then(|ks| TEXT_KINDS.iter().find(|k| ks.iter().any(|x| x == *k)))
            .copied()
            .unwrap_or("text/plain;charset=utf-8")
            .to_string();
        let Ok((read, write)) = vergence_protocol::sock::pipe() else { return };
        let sent = match owner {
            Some(ClipOwner::Native(from)) => {
                self.native_clipboard_send(from, kind, write);
                Ok(())
            }
            Some(ClipOwner::X11) => match self.xwm.as_mut() {
                Some(xwm) => xwm.send_selection(SelectionTarget::Clipboard, kind, write).map_err(|e| format!("{e:?}")),
                None => Err("X11 is gone".into()),
            },
            Some(ClipOwner::Server(_)) => Ok(()),
            Some(ClipOwner::Picture(_)) => Err("a picture, not text".into()),
            None => request_data_device_client_selection(&self.seat, kind, write).map_err(|e| format!("{e:?}")),
        };
        if let Err(e) = sent {
            return tracing::info!("paste into the server: nothing on the clipboard ({e})");
        }
        let (tx, rx) = smithay::reexports::calloop::channel::channel::<String>();
        let _ = self.loop_handle.insert_source(rx, move |ev, _, st| {
            if let smithay::reexports::calloop::channel::Event::Msg(text) = ev {
                then(st, text);
            }
        });
        let _ = std::thread::Builder::new().name("paste".into()).spawn(move || {
            use std::io::Read;
            let mut text = String::new();
            let mut f = std::fs::File::from(read).take(64 * 1024);
            if f.read_to_string(&mut text).is_ok() {
                let _ = tx.send(text);
            }
        });
    }

    /// A native client pastes: it gets the read end of a pipe whose write end
    /// goes to whoever has the clipboard. Only the client with the keyboard
    /// may (security pass 2026-11-15, finding 1).
    fn native_paste(&mut self, client: u32, serial: u32, kind: String) {
        use smithay::wayland::selection::{
            SelectionTarget,
            data_device::{current_data_device_selection_userdata, request_data_device_client_selection},
        };
        if !self.native_has_keyboard(client) {
            tracing::debug!("native: client {client} pasted without the keyboard; refused");
            return self.native_refuse(
                client,
                serial,
                ErrorCode::Denied,
                "paste is for the window with the keyboard".into(),
            );
        }
        let (read, write) = match vergence_protocol::sock::pipe() {
            Ok(p) => p,
            Err(e) => return self.native_refuse(client, serial, ErrorCode::BadMessage, format!("no pipe: {e}")),
        };
        let owner = current_data_device_selection_userdata(&self.seat).map(|o| o.clone());
        let sent = match owner {
            Some(ClipOwner::Native(from)) => {
                self.native_clipboard_send(from, kind.clone(), write);
                Ok(())
            }
            Some(ClipOwner::Server(text)) => {
                write_clip_text(text, write);
                Ok(())
            }
            Some(ClipOwner::Picture(png)) => {
                write_clip_picture(png, &kind, write);
                Ok(())
            }
            Some(ClipOwner::X11) => match self.xwm.as_mut() {
                Some(xwm) => {
                    xwm.send_selection(SelectionTarget::Clipboard, kind.clone(), write).map_err(|e| format!("{e:?}"))
                }
                None => Err("X11 is gone".into()),
            },
            // A Wayland client's selection, or none.
            None => request_data_device_client_selection(&self.seat, kind.clone(), write).map_err(|e| format!("{e:?}")),
        };
        match sent {
            Ok(()) => self.bus_send(
                client,
                ServerMsg::Window(WinEvent::ClipboardData {
                    serial,
                    fd: vergence_protocol::window::Fd(Arc::new(read)),
                }),
            ),
            Err(e) => {
                self.native_refuse(client, serial, ErrorCode::BadMessage, format!("nothing to paste as {kind}: {e}"))
            }
        }
    }

    /// Ask native client `client` to write its clipboard as `kind` into `fd`.
    pub fn native_clipboard_send(&mut self, client: u32, kind: String, fd: std::os::fd::OwnedFd) {
        self.bus_send(
            client,
            ServerMsg::Window(WinEvent::ClipboardSend { kind, fd: vergence_protocol::window::Fd(Arc::new(fd)) }),
        );
    }

    /// The clipboard changed: every native client hears what kinds it holds
    /// (empty: nothing).
    pub fn native_clipboard_offer(&mut self, kinds: Vec<String>) {
        self.native_input.clip_kinds = (!kinds.is_empty()).then(|| kinds.clone());
        let mut clients: Vec<u32> = self.natives.keys().map(|(c, _)| *c).collect();
        clients.sort_unstable();
        clients.dedup();
        for c in clients {
            self.bus_send(c, ServerMsg::Window(WinEvent::ClipboardOffer { kinds: kinds.clone() }));
        }
    }

    /// The key repeat changed (settings.rs): every client with native windows hears it.
    pub fn native_repeat_changed(&mut self) {
        let (rate, delay) = self.native_repeat();
        let clients: Vec<u32> = self.native_input.keymap_sent.iter().copied().collect();
        for c in clients {
            self.bus_send(c, ServerMsg::Window(WinEvent::Repeat { rate, delay }));
        }
    }
}

/// The clipboard rule as a plain function: `keyboard` is the native client
/// whose window has the keyboard, if any.
fn clipboard_allowed(keyboard: Option<u32>, client: u32) -> bool {
    keyboard == Some(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// What the toolkit draws for `points` at `scale` (vergence-render
    /// pixel_size rounds up) comes back as the same points, at every scale
    /// the slider stops at and others.
    #[test]
    fn pixels_rounded_up_come_back_as_the_points_given() {
        assert_eq!(points_of(774, 4.0 / 3.0), 580, "techbench's Terminal Settings, 2026-11-18");
        for scale in [0.75, 1.0, 1.25, 4.0 / 3.0, 1.4417, 1.5, 5.0 / 3.0, 2.0, 2.5, 3.0] {
            for points in [1, 160, 320, 579, 580, 581, 760, 1172, 2880] {
                let px = (points as f32 * scale as f32).ceil() as i32;
                let back = points_of(px, scale);
                assert!(
                    back == points || (scale < 1.0 && (back as f64 * scale).ceil() as i32 == px),
                    "{points} pt at {scale}: {px} px gave {back}"
                );
            }
        }
        assert_eq!(points_of(0, 1.5), 0);
        assert_eq!(points_of(100, 0.0), 100);
    }

    /// A frosted window's picture is never declared opaque, whatever its
    /// format (preview milestone 1, step 6b): the frost behind it shows
    /// through. Other windows' Xrgb pictures stay opaque, as before.
    #[test]
    fn a_frosted_window_has_no_opaque_region() {
        let px: Size<i32, BufferCoords> = (64, 32).into();
        assert_eq!(opaque_region(Fourcc::Xrgb8888, false, px), Some(vec![Rectangle::from_size(px)]));
        assert_eq!(opaque_region(Fourcc::Argb8888, false, px), None);
        assert_eq!(opaque_region(Fourcc::Xrgb8888, true, px), None);
        assert_eq!(opaque_region(Fourcc::Argb8888, true, px), None);
    }

    fn file_with(bytes: &[u8]) -> std::fs::File {
        let path = std::env::temp_dir().join(format!("vergence-native-test-{}-{}", std::process::id(), bytes.len()));
        let mut f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
        f.write_all(bytes).unwrap();
        let _ = std::fs::remove_file(&path);
        f
    }

    #[test]
    fn clipboard_is_only_for_the_client_with_the_keyboard() {
        assert!(clipboard_allowed(Some(7), 7));
        assert!(!clipboard_allowed(Some(7), 8), "another client's window has the keyboard");
        assert!(!clipboard_allowed(None, 7), "a Wayland or X window, or none, has the keyboard");
    }

    #[test]
    fn damage_copies_only_its_rectangle_through_the_stride() {
        // A 4 x 3 picture with a stride of 20 bytes (4 bytes of padding a row);
        // each pixel's bytes are its index.
        let mut src = vec![0xEEu8; 20 * 3];
        for y in 0..3 {
            for x in 0..4 {
                src[y * 20 + x * 4..y * 20 + x * 4 + 4].fill((y * 4 + x) as u8);
            }
        }
        let f = file_with(&src);
        let mut mem = vec![0u8; 4 * 4 * 3];
        let got = copy_damage(&f, &mut mem, 4, 3, 20, &[Damage { x: 1, y: 1, w: 2, h: 2 }]).unwrap();
        assert_eq!(got, vec![Rectangle::new((1, 1).into(), (2, 2).into())]);
        let px = |x: usize, y: usize| mem[(y * 4 + x) * 4];
        assert_eq!((px(1, 1), px(2, 1), px(1, 2), px(2, 2)), (5, 6, 9, 10));
        assert_eq!((px(0, 0), px(3, 1), px(0, 2)), (0, 0, 0), "outside the damage, untouched");
    }

    #[test]
    fn no_damage_means_all_of_it_and_damage_outside_is_clipped() {
        let src: Vec<u8> = (0..(8 * 2)).map(|i| i as u8).collect();
        let f = file_with(&src);
        let mut mem = vec![0u8; 8 * 2];
        copy_damage(&f, &mut mem, 2, 2, 8, &[]).unwrap();
        assert_eq!(mem, src);
        let got = copy_damage(
            &f,
            &mut mem,
            2,
            2,
            8,
            &[Damage { x: 1, y: 1, w: 50, h: 50 }, Damage { x: 9, y: 9, w: 1, h: 1 }],
        )
        .unwrap();
        assert_eq!(got, vec![Rectangle::new((1, 1).into(), (1, 1).into())]);
    }

    #[test]
    fn cursor_shapes_follow_the_cursor_shape_protocol() {
        use smithay::input::pointer::CursorIcon as C;
        // wp_cursor_shape_device_v1's numbers: 1 default ... 9 text, 13 move, 34 zoom-out.
        assert_eq!(cursor_icon(1), C::Default);
        assert_eq!(cursor_icon(4), C::Pointer);
        assert_eq!(cursor_icon(9), C::Text);
        assert_eq!(cursor_icon(13), C::Move);
        assert_eq!(cursor_icon(29), C::NwseResize);
        assert_eq!(cursor_icon(34), C::ZoomOut);
        assert_eq!(cursor_icon(0), C::Default, "not a shape");
        assert_eq!(cursor_icon(35), C::Default, "past the protocol's last");
    }

    #[test]
    fn a_file_shorter_than_the_picture_is_an_error_not_a_crash() {
        let f = file_with(&[1, 2, 3, 4]);
        let mut mem = vec![0u8; 4 * 4 * 4];
        assert!(copy_damage(&f, &mut mem, 4, 4, 16, &[]).is_err());
    }
}

/// A new persistence token: 128 random bits, as 32 lowercase hex digits.
fn new_token() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // No randomness to be had: unique enough for one machine's windows.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b = (t ^ (std::process::id() as u128) << 64).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod scene_fuzz {
    use super::*;
    use vergence_scene::{Color, Command, Image, Rect, Scene};

    /// Display lists step 6: damaged scenes through the server's own path
    /// (take_keyed, then the cost a draw would be) never panic, and the
    /// pictures a window keeps stay counted right, within their bound.
    #[test]
    fn damaged_scenes_through_the_server_path() {
        let image = |n: u8| Arc::new(Image { width: 8, height: 4, rgba: vec![n; 8 * 4 * 4] });
        let scene = Scene {
            width: 320.0,
            height: 200.0,
            fonts: Vec::new(),
            images: vec![image(1), image(2)],
            commands: vec![
                Command::Fill {
                    rect: Rect { x: 0.0, y: 0.0, w: 320.0, h: 200.0 },
                    radius: 4.0,
                    color: Color::rgb(200, 200, 200),
                },
                Command::Image { image: 0, rect: Rect { x: 10.0, y: 10.0, w: 80.0, h: 40.0 }, opacity: 1.0 },
                Command::Image { image: 1, rect: Rect { x: 100.0, y: 10.0, w: 80.0, h: 40.0 }, opacity: 0.5 },
                Command::Stroke {
                    rect: Rect { x: 5.0, y: 5.0, w: 300.0, h: 180.0 },
                    radius: 6.0,
                    width: 1.0,
                    color: Color::rgb(0, 0, 0),
                },
            ],
        };
        let first = scene.encode_keyed(&[7, 9], &|_| false).unwrap();
        let later = scene.encode_keyed(&[7, 9], &|_| true).unwrap();
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let (mut images, mut kept) = (std::collections::HashMap::new(), 0usize);
        take_keyed(&mut images, &mut kept, &first, &[]).unwrap();
        assert_eq!(kept, 2 * 128);
        let mut accepted = 0;
        for round in 0..20_000 {
            let mut bytes = if round % 2 == 0 { first.clone() } else { later.clone() };
            for _ in 0..1 + next() % 4 {
                let at = (next() as usize) % bytes.len();
                match next() % 3 {
                    0 => bytes[at] = next() as u8,
                    1 => bytes[at] ^= 1 << (next() % 8),
                    _ => bytes.truncate(at.max(1)),
                }
            }
            let forget: Vec<u64> = if next() % 8 == 0 { vec![7] } else { Vec::new() };
            if let Ok(s) = take_keyed(&mut images, &mut kept, &bytes, &forget) {
                accepted += 1;
                let _ = vergence_render::cost(&s, 1.5);
                let _ = vergence_render::pixel_size(&s, 1.5);
            }
            assert_eq!(kept, images.values().map(|i| i.rgba.len()).sum::<usize>(), "round {round}");
            assert!(kept <= SCENE_KEPT_BYTES);
            // Now and then everything sent again, as the toolkit would after forgetting.
            if images.len() < 2 {
                take_keyed(&mut images, &mut kept, &first, &[]).unwrap();
            }
        }
        assert!(accepted > 0, "some damaged scenes still decode (colors, numbers)");
    }
}
