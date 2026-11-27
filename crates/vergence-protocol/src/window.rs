//! Windows on the native protocol (whitepaper §6.3 to §6.5; native milestone
//! 1): a program's windows, their pictures, the input the user gives them,
//! the clipboard, where they ask to be (Contract 2) and what they mean
//! (Contract 3). On the same connection as the session bus (bus.rs), with
//! opcodes from 100 up.
//!
//! **File descriptors** travel beside the frame (sock.rs). A message that
//! carries any says how many as its first field, a `u8`, and they are taken
//! in the order they arrived; a message claiming descriptors that did not
//! come is refused, and the connection with it.
//!
//! Sizes and places are in **reference units** (Contract 1), as `f64`,
//! except a picture's own size and its damage, which are its pixels.

use std::{
    fmt,
    os::fd::{AsRawFd, OwnedFd, RawFd},
    sync::Arc,
};

use crate::wire::{Decoder, Encoder, WireError};

/// A file descriptor in a message. Shared, so a message can be cloned; equal
/// when it is the same descriptor.
#[derive(Clone)]
pub struct Fd(pub Arc<OwnedFd>);

impl Fd {
    pub fn new(fd: OwnedFd) -> Fd {
        Fd(Arc::new(fd))
    }

    pub fn raw(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl fmt::Debug for Fd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fd({})", self.raw())
    }
}

impl PartialEq for Fd {
    fn eq(&self, other: &Fd) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Where descriptors for a message being read come from: the ones that
/// arrived, in order.
pub trait FdSource {
    fn take_fd(&mut self) -> Option<OwnedFd>;
}

impl FdSource for std::collections::VecDeque<OwnedFd> {
    fn take_fd(&mut self) -> Option<OwnedFd> {
        self.pop_front()
    }
}

/// No descriptors at all (a reader that cannot have any).
pub struct NoFds;

impl FdSource for NoFds {
    fn take_fd(&mut self) -> Option<OwnedFd> {
        None
    }
}

// Limits, besides the frame's 64 KB.
pub const MAX_DAMAGE: usize = 256;
pub const MAX_PLANES: usize = 4;
/// The largest scene, in bytes (its pictures included), and the most keys
/// one Scene forgets.
pub const MAX_SCENE_BYTES: u32 = 300 << 20;
pub const MAX_FORGET: usize = 4096;
pub const MAX_KINDS: usize = 32;
pub const MAX_NODES: usize = 4096;
pub const MAX_STATES: usize = 16;
/// The most items a node lists (a choice's), and the most room they take
/// together (bus v25).
pub const MAX_ITEMS: usize = 512;
/// A picture's longest side, in pixels.
pub const MAX_SIDE: u32 = 16_384;
/// A bar's tabs, a tab title's bytes, and the program's own tab-menu entries.
pub const MAX_TABS: usize = 256;
pub const MAX_TAB_TITLE: usize = 1024;
pub const MAX_MENU: usize = 16;

/// What a window's bar shows, for the server to draw (bar milestone 1): the
/// window's title, or tabs. The window's buttons are the desktop's.
#[derive(Debug, Clone, PartialEq)]
pub enum Bar {
    /// The window's title (its `Title`).
    Plain,
    /// Tabs, in order, `front` being the id of the one in front; `plus`
    /// a + that asks for a new tab; `accent` a color (0xRRGGBB) the tab in
    /// front may wear; `menu` the program's own entries for a tab's menu,
    /// after the server's (Close, Close the others, Move to a new window).
    Tabs { tabs: Vec<Tab>, front: u32, plus: bool, accent: Option<u32>, menu: Vec<MenuEntry> },
    /// The program's own buttons (back and forward, say) and then the
    /// window's title, both starting `from` points in from the window's left
    /// (a sidebar's width), or after the window's buttons if they reach
    /// further (window styles milestone 1: the settings app).
    Tools { buttons: Vec<BarButton>, from: f64 },
}

/// A program's own button in its bar: an id it chose (unique, not 0), what
/// it shows, its name (for Contract 3), and whether it can be pressed now.
#[derive(Debug, Clone, PartialEq)]
pub struct BarButton {
    pub id: u32,
    pub icon: BarIcon,
    pub name: String,
    pub enabled: bool,
}

/// What a program's bar button shows; the server draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarIcon {
    Back,
    Forward,
    /// The button shows its name as text, at the bar's far end, before the
    /// window's buttons (preview milestone 1: a preview's "Open with Reader").
    Label,
}

/// The most buttons a bar may carry of its program's own.
pub const MAX_BAR_BUTTONS: usize = 8;

/// One tab: an id the program chose (unique in the window, not 0), its
/// title, and whether it has a close button.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: u32,
    pub title: String,
    pub closable: bool,
    /// What runs in it runs as the superuser (look milestone 1): the server
    /// draws the tab in the desktop's alarm red; the color is never the
    /// program's to choose.
    pub alert: bool,
    /// It has edits not saved (bus v14): the server marks it in the desktop's
    /// color for that, the same in every app (see docs/editor-milestone-1.md).
    pub edited: bool,
}

/// An entry of a tab's menu that the program adds: an id it chose and a label.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    pub id: u32,
    pub label: String,
}

/// Shared-memory picture formats, as DRM fourcc codes (the same numbering as
/// a GPU buffer's): four bytes a pixel, blue, green, red, then alpha (or
/// unused), little-endian; alpha premultiplied. The server shows these two.
pub const SHM_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
pub const SHM_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");

/// A part of a picture that changed, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Damage {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One plane of a GPU buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub fd: Fd,
    pub offset: u32,
    pub stride: u32,
}

/// A window's picture: shared memory, or a GPU buffer (linux-dmabuf's
/// terms: a fourcc format, a modifier, its planes).
#[derive(Debug, Clone, PartialEq)]
pub enum Buffer {
    Shm { fd: Fd, width: u32, height: u32, stride: u32, format: u32 },
    Dmabuf { planes: Vec<Plane>, width: u32, height: u32, format: u32, modifier: u64 },
}

/// Where a window asks to be (Contract 2): the server decides.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// Wherever the server thinks best (the monitor under the pointer, or
    /// where this window was last on this monitor setup).
    Anywhere,
    /// At a point of a monitor, by its id, from its top left corner.
    Monitor { id: String, x: f64, y: f64 },
    /// At a point from its parent window's top left corner.
    Parent { x: f64, y: f64 },
}

/// One widget of a window's meaning (Contract 3), as the toolkit lays it out.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Stable between updates; the parent's, 0 for the window itself.
    pub id: u64,
    pub parent: u64,
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub states: Vec<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// What it offers to choose from, by name (a choice's items; bus v25):
    /// what [`Action::Choose`] takes.
    pub items: Vec<String>,
}

/// What an agent asks a window's widget to do (Contract 3, bus v24,
/// meaning milestone 1): the widget answers as if the person had used it.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Press it: a button, a switch, a menu item, a choice (which opens).
    Press,
    /// Give it the keyboard.
    Focus,
    /// Its value: a field's or a text area's whole text (never a secret
    /// field's), a slider's number, a switch's "on" or "off".
    Set(String),
    /// A choice's item, by its name.
    Choose(String),
    /// Scroll whatever holds it until it can be seen.
    Reveal,
    /// Type this text into it, as the keyboard would (bus v38, terminal agent
    /// milestone 1): a terminal's pane. At most [`TYPE_MAX`] bytes.
    Type(String),
    /// Press this key, by name (bus v38): one of [`KEY_NAMES`], or `ctrl+`
    /// and a letter.
    Key(String),
}

/// The most text one [`Action::Type`] carries.
pub const TYPE_MAX: usize = 16 * 1024;

/// The keys [`Action::Key`] may name (as the control socket's `vctl key`
/// names them), besides `ctrl+<a-z>`.
pub const KEY_NAMES: &[&str] = &[
    "Return",
    "Tab",
    "Escape",
    "BackSpace",
    "Delete",
    "Up",
    "Down",
    "Left",
    "Right",
    "Home",
    "End",
    "Prior",
    "Next",
    "Insert",
    "space",
];

/// Whether a key name is one [`Action::Key`] takes.
pub fn key_name_ok(name: &str) -> bool {
    KEY_NAMES.contains(&name)
        || name.strip_prefix("ctrl+").is_some_and(|k| k.len() == 1 && k.as_bytes()[0].is_ascii_lowercase())
}

/// What a client asks of its windows.
#[derive(Debug, Clone, PartialEq)]
pub enum WinRequest {
    /// A new window, `window` an id the client chose (not 0). Its identity
    /// (Contract 2): the connection's application id, `role`, and `token`:
    /// one the server gave the same window on an earlier run; "new" for a new
    /// window of a program that keeps tokens; empty if the program keeps none
    /// (its windows are then told apart by the order they open in).
    /// Reply: [`WinEvent::Created`], then a [`WinEvent::Configure`].
    Create {
        serial: u32,
        window: u32,
        role: String,
        token: String,
        title: String,
        width: f64,
        height: f64,
        parent: u32,
        place: Place,
    },
    Destroy {
        window: u32,
    },
    Title {
        window: u32,
        title: String,
    },
    /// The configure with this serial is drawn for.
    AckConfigure {
        window: u32,
        serial: u32,
    },
    /// A new picture, and what changed since the last one (empty: all of it).
    /// Shared memory is copied when the server reads this message: the client
    /// may draw into it again at once. A GPU buffer is read while the server
    /// draws: it is free again once a later picture has been attached and a
    /// `Frame` has followed (the server swaps pictures at attach), so a client
    /// takes two or more in turn.
    Attach {
        window: u32,
        buffer: Buffer,
        damage: Vec<Damage>,
    },
    /// The pointer's shape over this window (the cursor-shape protocol's numbers).
    Cursor {
        window: u32,
        shape: u16,
    },
    Maximize {
        window: u32,
        on: bool,
    },
    Minimize {
        window: u32,
    },
    /// Ask again where to be (Contract 2).
    Place {
        window: u32,
        place: Place,
    },
    /// The window's meaning now (Contract 3): its widgets, the focused one
    /// (0: none) and the text caret, if a field has one.
    Meaning {
        window: u32,
        focus: u64,
        caret: Option<(f64, f64, f64, f64)>,
        nodes: Vec<Node>,
    },
    /// This client has what is to be pasted, in these kinds of data, in
    /// answer to the press or key of `input_serial`.
    Copy {
        input_serial: u32,
        kinds: Vec<String>,
    },
    /// The clipboard's data in this kind, please. Reply: [`WinEvent::ClipboardData`].
    Paste {
        serial: u32,
        kind: String,
    },
    /// What the window's bar shows now; the server draws it (it draws every
    /// native window's bar). Sent again whenever it changes.
    Bar {
        window: u32,
        bar: Bar,
    },
    /// This window draws under its bar (look milestone 1): its pictures are
    /// taller by the `inset` each [`WinEvent::Configure`] gives, the strip
    /// at their top lies under the server's bar (frosted there), and nothing
    /// that takes input belongs in it. Sent once, after Create.
    UnderBar {
        window: u32,
    },
    /// What kind of window this is, whether it is modal (only a dialog can
    /// be), and its smallest and largest size in points (0 on an axis: no
    /// limit). After Create, and again whenever it changes (window styles
    /// milestone 1). The server keeps every size within the limits and draws
    /// the bar the kind has. `frosted` (bus v36, preview milestone 1, step
    /// 6b): the window has no ground of its own; the server frosts what is
    /// behind it under its whole body, joined with its bar into one surface,
    /// as macOS's Quick Look window (the content an inset card on it).
    Style {
        window: u32,
        kind: WinKind,
        modal: bool,
        min: (f64, f64),
        max: (f64, f64),
        frosted: bool,
    },
    /// The window's commands now (app settings milestone 1): what its app
    /// menu holds (crate::commands). `terminal`: its Ctrl+letter keys belong
    /// to the programs it runs, so roles' keys take Shift. After Create, and
    /// again whenever they change (a command enabled, a switch flipped).
    Commands {
        window: u32,
        terminal: bool,
        commands: Vec<crate::commands::Command>,
    },
    /// The style its bar is drawn in (app settings milestone 1): the
    /// desktop's, or the app's own light or dark. After Create, and again
    /// whenever the app's setting changes.
    BarStyle {
        window: u32,
        style: BarStyle,
    },
    /// The size the window would like now, in points (a calculator opening
    /// its scientific keys; tools milestone 1). The server decides: within
    /// the window's limits and its screen, not while maximized or snapped.
    Size {
        window: u32,
        width: f64,
        height: f64,
    },
    /// Start dragging (files milestone 1, step 8), while the pointer's left
    /// button is held on this window: what is dragged, each kind with its
    /// data (file drags are a few paths, so the data goes with the drag:
    /// [`MAX_DRAG_BYTES`] in all), and the words the server's ghost shows.
    DragStart {
        window: u32,
        label: String,
        items: Vec<(String, Vec<u8>)>,
    },
    /// The answer to [`WinEvent::DragOver`]: what a drop here would do
    /// ([`DROP_NONE`], [`DROP_COPY`], [`DROP_MOVE`]).
    DragAccept {
        window: u32,
        action: u8,
    },
    /// Bring this window forward with the keyboard: the program was handed
    /// something to show by a start the person just made (a file opened in
    /// File Viewer going to the running Text Editor). The server grants it
    /// by the rule a new window takes the keyboard by (opened.rs).
    Activate {
        window: u32,
    },
    /// Open the server's menu for this window at (x, y) in it, with the
    /// window's commands (as last sent): a right click the program wants
    /// answered in the desktop's own menu style (the desktop's icons; step 9).
    ContextMenu {
        window: u32,
        x: f64,
        y: f64,
    },
    /// Open the server's menu for this window at (x, y) in it with these
    /// lines (bus v20): a right click on something in the window (a file
    /// in File Viewer, the ground around them). The window's own menu, not
    /// its app menu's commands; a chosen line comes back as
    /// [`WinEvent::Command`] with its id.
    Menu {
        window: u32,
        x: f64,
        y: f64,
        lines: Vec<MenuLine>,
    },
    /// The desktop's bare ground was clicked (bus v23, overview milestone
    /// 1, step 6): the windows move aside, or come back. Heard only from a
    /// window of the Desktop kind; any other's is ignored.
    GroundClicked {
        window: u32,
    },
    /// The answer to [`WinEvent::Act`] with this serial (bus v24): done, or
    /// why not ("it is off", "no such node").
    Acted {
        window: u32,
        serial: u32,
        ok: bool,
        why: String,
    },
    /// The person stopped an agent in this window (bus v38, terminal agent
    /// milestone 1: Escape twice, or a click on the agent's mark): its grant
    /// for the scope node `node` (a terminal's tab) ends now. `agent` is the
    /// name the window was told ([`WinEvent::AgentActing`]).
    RevokeAgent {
        window: u32,
        node: u64,
        agent: String,
    },
    /// The whole screen, with no bar and no dock, or back to its place (bus
    /// v41, AlbumWall's F11): the screen the window is mostly on.
    Fullscreen {
        window: u32,
        on: bool,
    },
    /// Every key to this window while it has the keyboard, the server's own
    /// shortcuts included (bus v40, remote milestone 1: Vergence Remote's
    /// viewer), or no longer. The server answers [`WinEvent::KeysCaptured`];
    /// Ctrl+Alt pressed and let go alone gives the keyboard back.
    CaptureKeys {
        window: u32,
        on: bool,
    },
    /// The answer to [`WinEvent::ReadText`] with this serial (bus v37): the
    /// node's text from `offset` (in characters), `total` characters in all;
    /// `found` false when there is no such node now.
    TextPage {
        window: u32,
        serial: u32,
        node: u64,
        found: bool,
        offset: u32,
        total: u32,
        text: String,
    },
    /// The window as a display list (display lists milestone 1, bus v30):
    /// a scene in vergence-scene's keyed wire form (version 2), `len` bytes
    /// at the start of the memfd `fd`, read when the server reads this
    /// message (the client may reuse it at once). Pictures sent whole in it
    /// are kept by their keys until `forget` names them; a later scene may
    /// name them by key alone. In place of a picture (Attach) until the next
    /// Attach or Scene.
    Scene {
        window: u32,
        fd: Fd,
        len: u32,
        forget: Vec<u64>,
    },
}

/// A line of a window's right-click menu ([`WinRequest::Menu`]).
#[derive(Debug, Clone, PartialEq)]
pub enum MenuLine {
    /// Something to do: its id (sent back), what it is called, its keys as
    /// shown at the right, whether it can be done now (faint if not), and
    /// whether it is a setting that is on or off.
    Item {
        id: u32,
        label: String,
        keys: String,
        enabled: bool,
        checked: Option<bool>,
    },
    Separator,
    /// A submenu (Open With ▸): its name and its lines, which hold no
    /// submenus of their own.
    Sub {
        label: String,
        lines: Vec<MenuLine>,
    },
}

impl MenuLine {
    pub fn item(id: u32, label: &str, keys: &str) -> MenuLine {
        MenuLine::Item { id, label: label.into(), keys: keys.into(), enabled: true, checked: None }
    }

    /// Faint unless `on`.
    pub fn enabled(mut self, on: bool) -> MenuLine {
        if let MenuLine::Item { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    pub fn checked(mut self, on: bool) -> MenuLine {
        if let MenuLine::Item { checked, .. } = &mut self {
            *checked = Some(on);
        }
        self
    }
}

/// The most lines a right-click menu, or one of its submenus, may have.
pub const MAX_MENU_LINES: usize = 64;

/// A line's id in a right-click menu of the desktop's (the Desktop kind)
/// that marks where the desktop's own lines go (the settings, "Exit
/// Vergence…", apps desktop.toml names): the server puts them in its place
/// and carries them out itself, sending the program nothing. In any other
/// window's menu the line is left out.
pub const MENU_DESKTOP_LINES: u32 = u32::MAX;

/// Ids from here up are the server's own lines in a right-click menu (the
/// desktop's, [`MENU_DESKTOP_LINES`]); a program's lines keep below.
pub const MENU_SERVER_IDS: u32 = 0xFFFF_0000;

fn put_menu_lines(e: &mut Encoder, lines: &[MenuLine]) {
    e.count(lines.len());
    for l in lines {
        match l {
            MenuLine::Item { id, label, keys, enabled, checked } => {
                e.u8(0).u32(*id).str(label).str(keys).bool(*enabled).u8(match checked {
                    None => 0,
                    Some(false) => 1,
                    Some(true) => 2,
                });
            }
            MenuLine::Separator => {
                e.u8(1);
            }
            MenuLine::Sub { label, lines } => {
                e.u8(2).str(label);
                put_menu_lines(e, lines);
            }
        }
    }
}

fn get_menu_lines(d: &mut Decoder, nested: bool) -> Result<Vec<MenuLine>, WireError> {
    use crate::commands::MAX_LABEL;
    let n = d.count(1)?;
    if n > MAX_MENU_LINES {
        return Err(WireError::TooLong(n));
    }
    let text = |d: &mut Decoder| -> Result<String, WireError> {
        let t = d.str()?;
        if t.len() > MAX_LABEL || t.chars().any(char::is_control) {
            Err(WireError::BadValue("menu text"))
        } else {
            Ok(t)
        }
    };
    let mut lines = Vec::with_capacity(n);
    for _ in 0..n {
        lines.push(match d.u8()? {
            0 => {
                let id = d.u32()?;
                let (label, keys, enabled) = (text(d)?, text(d)?, d.bool()?);
                let checked = match d.u8()? {
                    0 => None,
                    1 => Some(false),
                    2 => Some(true),
                    _ => return Err(WireError::BadValue("menu switch")),
                };
                if id == 0 || label.is_empty() {
                    return Err(WireError::BadValue("menu item"));
                }
                MenuLine::Item { id, label, keys, enabled, checked }
            }
            1 => MenuLine::Separator,
            2 if !nested => {
                let label = text(d)?;
                if label.is_empty() {
                    return Err(WireError::BadValue("menu item"));
                }
                MenuLine::Sub { label, lines: get_menu_lines(d, true)? }
            }
            _ => return Err(WireError::BadValue("menu line")),
        });
    }
    Ok(lines)
}

/// A drag's data, all kinds together, at the most (a frame is 64 KB).
pub const MAX_DRAG_BYTES: usize = 48 * 1024;
/// What a drop does.
pub const DROP_NONE: u8 = 0;
pub const DROP_COPY: u8 = 1;
pub const DROP_MOVE: u8 = 2;
/// Modifier bits a drag's events carry.
pub const DRAG_CTRL: u8 = 1;
pub const DRAG_SHIFT: u8 = 2;

fn put_items(e: &mut Encoder, items: &[(String, Vec<u8>)]) {
    e.count(items.len());
    for (k, v) in items {
        e.str(k).bytes(v);
    }
}

fn get_items(d: &mut Decoder) -> Result<Vec<(String, Vec<u8>)>, WireError> {
    let n = d.count(8)?;
    if n > MAX_KINDS {
        return Err(WireError::TooLong(n));
    }
    let mut total = 0usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let k = d.str()?;
        let v = d.bytes()?.to_vec();
        total += v.len();
        if total > MAX_DRAG_BYTES {
            return Err(WireError::TooLong(total));
        }
        out.push((k, v));
    }
    Ok(out)
}

/// A window's bar style: the desktop's, or the app's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BarStyle {
    #[default]
    Desktop,
    Dark,
    Light,
}

/// A window's kind (window styles milestone 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WinKind {
    /// A window of its own, as every window was before.
    #[default]
    Normal,
    /// Belongs to its parent (Create's `parent`): a slim bar, close only,
    /// over the parent and going with it.
    Dialog,
    /// A tool panel of its application: a thin bar, above that
    /// application's windows only.
    Palette,
    /// Content running up under the frosted bar, the program's own buttons
    /// and the title in it (the settings app).
    Sidebar,
    /// The desktop's icons (files milestone 1, step 9): the server sizes it
    /// to the main screen's work area and keeps it under every window, with
    /// no bar, out of the dock and Alt+Tab.
    Desktop,
}

impl WinKind {
    fn byte(self) -> u8 {
        match self {
            WinKind::Normal => 0,
            WinKind::Dialog => 1,
            WinKind::Palette => 2,
            WinKind::Sidebar => 3,
            WinKind::Desktop => 4,
        }
    }

    fn from_byte(b: u8) -> Result<WinKind, WireError> {
        Ok(match b {
            0 => WinKind::Normal,
            1 => WinKind::Dialog,
            2 => WinKind::Palette,
            3 => WinKind::Sidebar,
            4 => WinKind::Desktop,
            _ => return Err(WireError::BadValue("window kind")),
        })
    }
}

/// A size limit as sent: finite, not negative (0: none).
fn limit(v: f64, what: &'static str) -> Result<f64, WireError> {
    let v = finite(v, what)?;
    if v < 0.0 { Err(WireError::BadValue(what)) } else { Ok(v) }
}

/// The tallest strip a window may be asked to draw under its bar, in points.
pub const MAX_INSET: f64 = 256.0;
/// The text factor's range (`WinEvent::Configure::text`).
pub const MIN_TEXT: f64 = 0.5;
pub const MAX_TEXT: f64 = 2.0;

/// What the server tells a client about its windows.
#[derive(Debug, Clone, PartialEq)]
pub enum WinEvent {
    /// The window exists; `token` names it on later runs (Contract 2).
    Created {
        serial: u32,
        window: u32,
        token: String,
    },
    /// Draw at this size, for this scale (Contract 1); answer with AckConfigure.
    /// `inset`: the strip at the top of the window's picture under the
    /// server's bar (points; 0 unless it asked with [`WinRequest::UnderBar`]):
    /// the picture is `height + inset` tall and the content below the strip.
    /// `text`: how much larger than its sizes say to draw text on this
    /// screen (version 21): the screen's calculated scale over the one it is
    /// shown at, so a size looks the same on every screen (1 = as given).
    Configure {
        window: u32,
        serial: u32,
        width: f64,
        height: f64,
        scale: f64,
        maximized: bool,
        activated: bool,
        inset: f64,
        text: f64,
    },
    /// The user asked for the window to close.
    Close {
        window: u32,
    },
    /// The last picture is on the screen: draw the next when there is one.
    Frame {
        window: u32,
        time_ms: u32,
    },
    PointerEnter {
        window: u32,
        serial: u32,
        x: f64,
        y: f64,
    },
    PointerLeave {
        window: u32,
    },
    PointerMotion {
        window: u32,
        time_ms: u32,
        x: f64,
        y: f64,
    },
    PointerButton {
        window: u32,
        serial: u32,
        time_ms: u32,
        button: u32,
        pressed: bool,
    },
    /// Scrolling: `source` 0 wheel, 1 finger, 2 continuous, 3 wheel tilt;
    /// the amounts in reference units, and in 120ths of a notch for a wheel.
    PointerAxis {
        window: u32,
        time_ms: u32,
        source: u8,
        dx: f64,
        dy: f64,
        v120x: i32,
        v120y: i32,
    },
    /// The keymap (xkb's text format, `size` bytes, in a descriptor).
    Keymap {
        fd: Fd,
        size: u32,
    },
    KeyboardEnter {
        window: u32,
        serial: u32,
    },
    KeyboardLeave {
        window: u32,
    },
    Key {
        window: u32,
        serial: u32,
        time_ms: u32,
        key: u32,
        pressed: bool,
    },
    Modifiers {
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    },
    /// Repeats a second (0: none) and the delay before the first, in ms.
    Repeat {
        rate: i32,
        delay: i32,
    },
    /// A finger: `phase` 0 down, 1 moved, 2 up, 3 canceled.
    Touch {
        window: u32,
        time_ms: u32,
        id: i32,
        phase: u8,
        x: f64,
        y: f64,
    },
    /// There is something to paste, in these kinds.
    ClipboardOffer {
        kinds: Vec<String>,
    },
    /// Someone is pasting what this client copied: write it in `kind` to `fd`, then close it.
    ClipboardSend {
        kind: String,
        fd: Fd,
    },
    /// What was asked for with Paste, to be read from `fd`.
    ClipboardData {
        serial: u32,
        fd: Fd,
    },
    // The bar's events (bar milestone 1): what the user did with the tabs
    // the server drew. The program decides what each means.
    /// A tab was chosen.
    TabChosen {
        window: u32,
        tab: u32,
    },
    /// A tab's close button, its middle click or its menu's Close.
    TabClose {
        window: u32,
        tab: u32,
    },
    /// "Close the others" from a tab's menu.
    TabCloseOthers {
        window: u32,
        tab: u32,
    },
    /// The + was pressed.
    TabNew {
        window: u32,
    },
    /// A tab was dragged to another place in the row (`to` counted with the
    /// tab taken out first).
    TabMoved {
        window: u32,
        tab: u32,
        to: u32,
    },
    /// A tab was dragged out of the window, or "Move to a new window" was
    /// chosen: open a window for it there (`place` for its Create).
    TabOut {
        window: u32,
        tab: u32,
        place: Place,
    },
    /// A tab was dropped on another window of this program's bar, at `to`.
    TabJoin {
        window: u32,
        tab: u32,
        into: u32,
        to: u32,
    },
    /// One of the program's own tab-menu entries was chosen.
    TabMenu {
        window: u32,
        tab: u32,
        entry: u32,
    },
    /// One of the program's own bar buttons ([`Bar::Tools`]) was pressed.
    BarButton {
        window: u32,
        id: u32,
    },
    /// The wallpaper's average color behind the window (0xRRGGBB), for it to
    /// mix a little into its grounds; None: no tint (light style, the switch
    /// off, Reduce transparency or High contrast).
    Tint {
        window: u32,
        rgb: Option<u32>,
    },
    /// A command of the window's app menu was chosen (its id), from the menu
    /// or its search.
    Command {
        window: u32,
        id: u32,
    },
    /// A drag is over this window at (x, y), with these kinds; `mods`: the
    /// keys held ([`DRAG_CTRL`], [`DRAG_SHIFT`]). Answer with DragAccept.
    DragOver {
        window: u32,
        x: f64,
        y: f64,
        kinds: Vec<String>,
        mods: u8,
    },
    /// The drag left this window (or ended elsewhere).
    DragLeave {
        window: u32,
    },
    /// Dropped here: what to do (as this window accepted it) and the data.
    DragDrop {
        window: u32,
        x: f64,
        y: f64,
        action: u8,
        items: Vec<(String, Vec<u8>)>,
    },
    /// To the window the drag started from: it ended, with this action
    /// ([`DROP_NONE`]: nowhere took it).
    DragEnded {
        window: u32,
        action: u8,
    },
    /// Do `action` to the widget whose meaning node is `node` (Contract 3,
    /// bus v24): answered by [`WinRequest::Acted`] with the same serial.
    Act {
        window: u32,
        serial: u32,
        node: u64,
        action: Action,
    },
    /// The server will not draw this window's scenes any more (bus v31,
    /// display lists step 6: one too costly, or drawing it too slow): send
    /// pixels for it from now on. `why` is for the log.
    ScenesOff {
        window: u32,
        why: String,
    },
    /// The session is ending (bus v33, session milestone 1): the Close that
    /// follows is the session's, not the user's, so what the program keeps
    /// of this window stays for the next session.
    SessionEnding {
        window: u32,
    },
    /// An agent acts on meaning node `node` now (bus v38): the name the person
    /// knows it by, sent before each [`Action::Type`] and [`Action::Key`] so
    /// the window can show who types (a terminal's tab mark) and stop it.
    AgentActing {
        window: u32,
        node: u64,
        agent: String,
    },
    /// The tabs agents hold a grant for in this window now (bus v39), sent
    /// whenever that changes: each tab's meaning node, the agent's name and
    /// whether it may type there (else only read). The window's stop keys
    /// (Escape twice) are for these tabs, used or not.
    AgentTabs {
        window: u32,
        tabs: Vec<AgentTab>,
    },
    /// Whether the window now has every key (bus v40): the answer to
    /// [`WinRequest::CaptureKeys`], and `false` when Ctrl+Alt gave the
    /// keyboard back.
    KeysCaptured {
        window: u32,
        on: bool,
    },
    /// Whether the window can be seen (bus v42, Mind the battery): `false`
    /// while it is minimized or the session is locked (not under the remote
    /// curtain, which is watched from elsewhere), `true` when it can be seen
    /// again. Sent only when it changes; a new window can be seen. A window
    /// that cannot be seen draws nothing and wakes for nothing it would show.
    Seen {
        window: u32,
        seen: bool,
    },
    /// Send the text of meaning node `node` (its value, else its name) from
    /// character `offset`, at most `len` characters (bus v37, paged reading:
    /// the tree cuts every text at [`MEANING_TEXT_MAX`] characters). Answered
    /// by [`WinRequest::TextPage`] with the same serial.
    ReadText {
        window: u32,
        serial: u32,
        node: u64,
        offset: u32,
        len: u32,
    },
}

/// The most characters a page of text carries ([`WinEvent::ReadText`]): with
/// four bytes a character at worst, well inside a frame.
pub const TEXT_PAGE_MAX: u32 = 8192;

/// A tab an agent holds a grant for ([`WinEvent::AgentTabs`], bus v39).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTab {
    pub node: u64,
    pub agent: String,
    pub typing: bool,
}

/// Room kept for an AgentTabs list before it is read (the count says more
/// only if the frame holds that many).
const AGENT_TABS_MAX: usize = 256;
/// What the tree cuts every name and value at (the toolkit's meaning request).
pub const MEANING_TEXT_MAX: usize = 512;

pub(crate) mod op {
    // Client to server.
    pub const CREATE: u16 = 100;
    pub const DESTROY: u16 = 101;
    pub const TITLE: u16 = 102;
    pub const ACK_CONFIGURE: u16 = 103;
    pub const ATTACH: u16 = 104;
    pub const CURSOR: u16 = 105;
    // 106 and 107 were Move and Resize, for windows that drew their own bar
    // (gone with bar milestone 1, step 8: the server draws every bar).
    pub const MAXIMIZE: u16 = 108;
    pub const MINIMIZE: u16 = 109;
    pub const PLACE: u16 = 110;
    pub const MEANING: u16 = 111;
    pub const COPY: u16 = 112;
    pub const PASTE: u16 = 113;
    pub const BAR: u16 = 114;
    pub const UNDER_BAR: u16 = 115;
    pub const STYLE: u16 = 116;
    pub const COMMANDS: u16 = 117;
    pub const BAR_STYLE: u16 = 118;
    pub const SIZE: u16 = 119;
    pub const DRAG_START: u16 = 120;
    pub const DRAG_ACCEPT: u16 = 121;
    pub const ACTIVATE: u16 = 122;
    pub const CONTEXT_MENU: u16 = 123;
    pub const MENU: u16 = 124;
    pub const GROUND_CLICKED: u16 = 125;
    pub const ACTED: u16 = 126;
    pub const SCENE: u16 = 127;
    pub const TEXT_PAGE: u16 = 128;
    pub const REVOKE_AGENT: u16 = 129;
    pub const CAPTURE_KEYS: u16 = 130;
    pub const FULLSCREEN: u16 = 131;
    // Server to client.
    pub const CREATED: u16 = 100;
    pub const CONFIGURE: u16 = 101;
    pub const CLOSE: u16 = 102;
    pub const FRAME: u16 = 103;
    pub const POINTER_ENTER: u16 = 104;
    pub const POINTER_LEAVE: u16 = 105;
    pub const POINTER_MOTION: u16 = 106;
    pub const POINTER_BUTTON: u16 = 107;
    pub const POINTER_AXIS: u16 = 108;
    pub const KEYMAP: u16 = 109;
    pub const KEYBOARD_ENTER: u16 = 110;
    pub const KEYBOARD_LEAVE: u16 = 111;
    pub const KEY: u16 = 112;
    pub const MODIFIERS: u16 = 113;
    pub const REPEAT: u16 = 114;
    pub const TOUCH: u16 = 115;
    pub const CLIPBOARD_OFFER: u16 = 116;
    pub const CLIPBOARD_SEND: u16 = 117;
    pub const CLIPBOARD_DATA: u16 = 118;
    pub const TAB_CHOSEN: u16 = 119;
    pub const TAB_CLOSE: u16 = 120;
    pub const TAB_CLOSE_OTHERS: u16 = 121;
    pub const TAB_NEW: u16 = 122;
    pub const TAB_MOVED: u16 = 123;
    pub const TAB_OUT: u16 = 124;
    pub const TAB_JOIN: u16 = 125;
    pub const TAB_MENU: u16 = 126;
    pub const BAR_BUTTON: u16 = 127;
    pub const TINT: u16 = 128;
    pub const COMMAND: u16 = 129;
    pub const DRAG_OVER: u16 = 130;
    pub const DRAG_LEAVE: u16 = 131;
    pub const DRAG_DROP: u16 = 132;
    pub const DRAG_ENDED: u16 = 133;
    pub const ACT: u16 = 134;
    pub const SCENES_OFF: u16 = 135;
    pub const SESSION_ENDING: u16 = 136;
    pub const READ_TEXT: u16 = 137;
    pub const AGENT_ACTING: u16 = 138;
    pub const AGENT_TABS: u16 = 139;
    pub const KEYS_CAPTURED: u16 = 140;
    pub const SEEN: u16 = 141;

    /// Whether an opcode is a window message's (either way).
    pub fn is_window(opcode: u16) -> bool {
        (100..200).contains(&opcode)
    }
}

pub use op::is_window as is_window_opcode;

fn take(fds: &mut dyn FdSource, d: &mut Decoder) -> Result<Vec<Fd>, WireError> {
    let n = d.u8()? as usize;
    if n > crate::sock::MAX_FDS {
        return Err(WireError::BadValue("descriptor count"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(Fd::new(fds.take_fd().ok_or(WireError::BadValue("a descriptor that did not come"))?));
    }
    Ok(out)
}

/// Meaning nodes on the wire (Meaning, and the agent interface's replies).
pub(crate) fn put_nodes(e: &mut Encoder, nodes: &[Node]) {
    e.count(nodes.len());
    for n in nodes {
        e.u64(n.id).u64(n.parent).str(&n.role).str(&n.name);
        match &n.value {
            Some(v) => e.bool(true).str(v),
            None => e.bool(false),
        };
        e.count(n.states.len());
        for s in &n.states {
            e.str(s);
        }
        e.f64(n.x).f64(n.y).f64(n.w).f64(n.h);
        e.count(n.items.len());
        for i in &n.items {
            e.str(i);
        }
    }
}

pub(crate) fn get_nodes(d: &mut Decoder) -> Result<Vec<Node>, WireError> {
    let n = d.count(16 + 8 + 1 + 4 + 32 + 4)?;
    if n > MAX_NODES {
        return Err(WireError::TooLong(n));
    }
    let mut nodes = Vec::with_capacity(n);
    for _ in 0..n {
        let (id, parent, role, name) = (d.u64()?, d.u64()?, d.str()?, d.str()?);
        let value = if d.bool()? { Some(d.str()?) } else { None };
        let ns = d.count(4)?;
        if ns > MAX_STATES {
            return Err(WireError::TooLong(ns));
        }
        let states = (0..ns).map(|_| d.str()).collect::<Result<_, _>>()?;
        let (x, y, w, h) = (
            finite(d.f64()?, "bounds")?,
            finite(d.f64()?, "bounds")?,
            finite(d.f64()?, "bounds")?,
            finite(d.f64()?, "bounds")?,
        );
        let ni = d.count(4)?;
        if ni > MAX_ITEMS {
            return Err(WireError::TooLong(ni));
        }
        let items = (0..ni).map(|_| d.str()).collect::<Result<_, _>>()?;
        nodes.push(Node { id, parent, role, name, value, states, x, y, w, h, items });
    }
    Ok(nodes)
}

pub(crate) fn put_action(e: &mut Encoder, action: &Action) {
    match action {
        Action::Press => e.u8(0),
        Action::Focus => e.u8(1),
        Action::Set(v) => e.u8(2).str(v),
        Action::Choose(v) => e.u8(3).str(v),
        Action::Reveal => e.u8(4),
        Action::Type(t) => e.u8(5).str(t),
        Action::Key(k) => e.u8(6).str(k),
    };
}

pub(crate) fn get_action(d: &mut Decoder) -> Result<Action, WireError> {
    Ok(match d.u8()? {
        0 => Action::Press,
        1 => Action::Focus,
        2 => Action::Set(d.str()?),
        3 => Action::Choose(d.str()?),
        4 => Action::Reveal,
        5 => {
            let t = d.str()?;
            if t.len() > TYPE_MAX {
                return Err(WireError::TooLong(t.len()));
            }
            Action::Type(t)
        }
        6 => {
            let k = d.str()?;
            if !key_name_ok(&k) {
                return Err(WireError::BadValue("key name"));
            }
            Action::Key(k)
        }
        _ => return Err(WireError::BadValue("action")),
    })
}

pub(crate) fn finite(v: f64, what: &'static str) -> Result<f64, WireError> {
    if v.is_finite() && v.abs() <= 1e7 { Ok(v) } else { Err(WireError::BadValue(what)) }
}

fn put_place(e: &mut Encoder, p: &Place) {
    match p {
        Place::Anywhere => e.u8(0),
        Place::Monitor { id, x, y } => e.u8(1).str(id).f64(*x).f64(*y),
        Place::Parent { x, y } => e.u8(2).f64(*x).f64(*y),
    };
}

fn get_place(d: &mut Decoder) -> Result<Place, WireError> {
    Ok(match d.u8()? {
        0 => Place::Anywhere,
        1 => Place::Monitor { id: d.str()?, x: finite(d.f64()?, "place")?, y: finite(d.f64()?, "place")? },
        2 => Place::Parent { x: finite(d.f64()?, "place")?, y: finite(d.f64()?, "place")? },
        _ => return Err(WireError::BadValue("place")),
    })
}

fn get_kinds(d: &mut Decoder) -> Result<Vec<String>, WireError> {
    let n = d.count(4)?;
    if n > MAX_KINDS {
        return Err(WireError::TooLong(n));
    }
    (0..n).map(|_| d.str()).collect()
}

fn put_kinds(e: &mut Encoder, kinds: &[String]) {
    e.count(kinds.len());
    for k in kinds {
        e.str(k);
    }
}

fn put_bar(e: &mut Encoder, bar: &Bar) {
    match bar {
        Bar::Plain => {
            e.u8(0);
        }
        Bar::Tabs { tabs, front, plus, accent, menu } => {
            e.u8(1).count(tabs.len());
            for t in tabs {
                e.u32(t.id).str(&t.title).bool(t.closable).bool(t.alert).bool(t.edited);
            }
            e.u32(*front).bool(*plus);
            match accent {
                Some(c) => e.bool(true).u32(*c),
                None => e.bool(false),
            };
            e.count(menu.len());
            for m in menu {
                e.u32(m.id).str(&m.label);
            }
        }
        Bar::Tools { buttons, from } => {
            e.u8(2).count(buttons.len());
            for b in buttons {
                e.u32(b.id)
                    .u8(match b.icon {
                        BarIcon::Back => 0,
                        BarIcon::Forward => 1,
                        BarIcon::Label => 2,
                    })
                    .str(&b.name)
                    .bool(b.enabled);
            }
            e.f64(*from);
        }
    }
}

/// A bar description, checked: the limits, ids unique and not 0, the tab in
/// front one of the tabs (or 0 with none).
fn get_bar(d: &mut Decoder) -> Result<Bar, WireError> {
    let title = |d: &mut Decoder| -> Result<String, WireError> {
        let t = d.str()?;
        if t.len() > MAX_TAB_TITLE { Err(WireError::TooLong(t.len())) } else { Ok(t) }
    };
    Ok(match d.u8()? {
        0 => Bar::Plain,
        1 => {
            let n = d.count(9)?;
            if n > MAX_TABS {
                return Err(WireError::TooLong(n));
            }
            let mut tabs: Vec<Tab> = Vec::with_capacity(n);
            for _ in 0..n {
                let (id, t, closable, alert, edited) = (d.u32()?, title(d)?, d.bool()?, d.bool()?, d.bool()?);
                if id == 0 || tabs.iter().any(|x| x.id == id) {
                    return Err(WireError::BadValue("tab id"));
                }
                tabs.push(Tab { id, title: t, closable, alert, edited });
            }
            let (front, plus) = (d.u32()?, d.bool()?);
            if (tabs.is_empty() && front != 0) || (!tabs.is_empty() && !tabs.iter().any(|t| t.id == front)) {
                return Err(WireError::BadValue("tab in front"));
            }
            let accent = if d.bool()? { Some(d.u32()? & 0xff_ffff) } else { None };
            let m = d.count(8)?;
            if m > MAX_MENU {
                return Err(WireError::TooLong(m));
            }
            let mut menu: Vec<MenuEntry> = Vec::with_capacity(m);
            for _ in 0..m {
                let (id, label) = (d.u32()?, title(d)?);
                if menu.iter().any(|x| x.id == id) {
                    return Err(WireError::BadValue("menu entry id"));
                }
                menu.push(MenuEntry { id, label });
            }
            Bar::Tabs { tabs, front, plus, accent, menu }
        }
        2 => {
            let n = d.count(7)?;
            if n > MAX_BAR_BUTTONS {
                return Err(WireError::TooLong(n));
            }
            let mut buttons: Vec<BarButton> = Vec::with_capacity(n);
            for _ in 0..n {
                let id = d.u32()?;
                let icon = match d.u8()? {
                    0 => BarIcon::Back,
                    1 => BarIcon::Forward,
                    2 => BarIcon::Label,
                    _ => return Err(WireError::BadValue("bar button icon")),
                };
                let (name, enabled) = (title(d)?, d.bool()?);
                if id == 0 || buttons.iter().any(|b| b.id == id) {
                    return Err(WireError::BadValue("bar button id"));
                }
                buttons.push(BarButton { id, icon, name, enabled });
            }
            let from = finite(d.f64()?, "where the title starts")?;
            if !(0.0..=4096.0).contains(&from) {
                return Err(WireError::BadValue("where the title starts"));
            }
            Bar::Tools { buttons, from }
        }
        _ => return Err(WireError::BadValue("bar")),
    })
}

fn size(w: u32, h: u32) -> Result<(), WireError> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE { Err(WireError::BadValue("picture size")) } else { Ok(()) }
}

impl WinRequest {
    /// The frame, and the descriptors to send beside it (borrowed from the
    /// message, which must outlive the write).
    pub fn encode(&self) -> Result<(Vec<u8>, Vec<RawFd>), WireError> {
        let mut fds = Vec::new();
        let e = match self {
            WinRequest::Create { serial, window, role, token, title, width, height, parent, place } => {
                let mut e = Encoder::frame(op::CREATE);
                e.u32(*serial).u32(*window).str(role).str(token).str(title).f64(*width).f64(*height).u32(*parent);
                put_place(&mut e, place);
                e
            }
            WinRequest::Destroy { window } => {
                let mut e = Encoder::frame(op::DESTROY);
                e.u32(*window);
                e
            }
            WinRequest::Title { window, title } => {
                let mut e = Encoder::frame(op::TITLE);
                e.u32(*window).str(title);
                e
            }
            WinRequest::AckConfigure { window, serial } => {
                let mut e = Encoder::frame(op::ACK_CONFIGURE);
                e.u32(*window).u32(*serial);
                e
            }
            WinRequest::Attach { window, buffer, damage } => {
                let mut e = Encoder::frame(op::ATTACH);
                match buffer {
                    Buffer::Shm { fd, width, height, stride, format } => {
                        fds.push(fd.raw());
                        e.u8(1).u32(*window).u8(0).u32(*width).u32(*height).u32(*stride).u32(*format);
                    }
                    Buffer::Dmabuf { planes, width, height, format, modifier } => {
                        fds.extend(planes.iter().map(|p| p.fd.raw()));
                        e.u8(planes.len() as u8)
                            .u32(*window)
                            .u8(1)
                            .u32(*width)
                            .u32(*height)
                            .u32(*format)
                            .u64(*modifier);
                        for p in planes {
                            e.u32(p.offset).u32(p.stride);
                        }
                    }
                }
                e.count(damage.len());
                for d in damage {
                    e.i32(d.x).i32(d.y).i32(d.w).i32(d.h);
                }
                e
            }
            WinRequest::Cursor { window, shape } => {
                let mut e = Encoder::frame(op::CURSOR);
                e.u32(*window).u16(*shape);
                e
            }
            WinRequest::Maximize { window, on } => {
                let mut e = Encoder::frame(op::MAXIMIZE);
                e.u32(*window).bool(*on);
                e
            }
            WinRequest::Minimize { window } => {
                let mut e = Encoder::frame(op::MINIMIZE);
                e.u32(*window);
                e
            }
            WinRequest::Place { window, place } => {
                let mut e = Encoder::frame(op::PLACE);
                e.u32(*window);
                put_place(&mut e, place);
                e
            }
            WinRequest::Meaning { window, focus, caret, nodes } => {
                let mut e = Encoder::frame(op::MEANING);
                e.u32(*window).u64(*focus);
                match caret {
                    Some((x, y, w, h)) => e.bool(true).f64(*x).f64(*y).f64(*w).f64(*h),
                    None => e.bool(false),
                };
                put_nodes(&mut e, nodes);
                e
            }
            WinRequest::Copy { input_serial, kinds } => {
                let mut e = Encoder::frame(op::COPY);
                e.u32(*input_serial);
                put_kinds(&mut e, kinds);
                e
            }
            WinRequest::Paste { serial, kind } => {
                let mut e = Encoder::frame(op::PASTE);
                e.u32(*serial).str(kind);
                e
            }
            WinRequest::Bar { window, bar } => {
                let mut e = Encoder::frame(op::BAR);
                e.u32(*window);
                put_bar(&mut e, bar);
                e
            }
            WinRequest::UnderBar { window } => {
                let mut e = Encoder::frame(op::UNDER_BAR);
                e.u32(*window);
                e
            }
            WinRequest::Style { window, kind, modal, min, max, frosted } => {
                let mut e = Encoder::frame(op::STYLE);
                e.u32(*window)
                    .u8(kind.byte())
                    .u8(*modal as u8)
                    .f64(min.0)
                    .f64(min.1)
                    .f64(max.0)
                    .f64(max.1)
                    .bool(*frosted);
                e
            }
            WinRequest::Commands { window, terminal, commands } => {
                let mut e = Encoder::frame(op::COMMANDS);
                e.u32(*window).bool(*terminal).count(commands.len());
                for c in commands {
                    e.u32(c.id).u16(c.role.code()).str(&c.menu).str(&c.label).str(&c.keys).bool(c.enabled);
                    match c.checked {
                        None => e.u8(0),
                        Some(false) => e.u8(1),
                        Some(true) => e.u8(2),
                    };
                    e.u8(c.featured as u8);
                }
                e
            }
            WinRequest::Size { window, width, height } => {
                let mut e = Encoder::frame(op::SIZE);
                e.u32(*window).f64(*width).f64(*height);
                e
            }
            WinRequest::DragStart { window, label, items } => {
                let mut e = Encoder::frame(op::DRAG_START);
                e.u32(*window).str(label);
                put_items(&mut e, items);
                e
            }
            WinRequest::DragAccept { window, action } => {
                let mut e = Encoder::frame(op::DRAG_ACCEPT);
                e.u32(*window).u8(*action);
                e
            }
            WinRequest::Activate { window } => {
                let mut e = Encoder::frame(op::ACTIVATE);
                e.u32(*window);
                e
            }
            WinRequest::ContextMenu { window, x, y } => {
                let mut e = Encoder::frame(op::CONTEXT_MENU);
                e.u32(*window).f64(*x).f64(*y);
                e
            }
            WinRequest::Menu { window, x, y, lines } => {
                let mut e = Encoder::frame(op::MENU);
                e.u32(*window).f64(*x).f64(*y);
                put_menu_lines(&mut e, lines);
                e
            }
            WinRequest::GroundClicked { window } => {
                let mut e = Encoder::frame(op::GROUND_CLICKED);
                e.u32(*window);
                e
            }
            WinRequest::Acted { window, serial, ok, why } => {
                let mut e = Encoder::frame(op::ACTED);
                e.u32(*window).u32(*serial).bool(*ok).str(why);
                e
            }
            WinRequest::RevokeAgent { window, node, agent } => {
                let mut e = Encoder::frame(op::REVOKE_AGENT);
                e.u32(*window).u64(*node).str(agent);
                e
            }
            WinRequest::CaptureKeys { window, on } => {
                let mut e = Encoder::frame(op::CAPTURE_KEYS);
                e.u32(*window).bool(*on);
                e
            }
            WinRequest::Fullscreen { window, on } => {
                let mut e = Encoder::frame(op::FULLSCREEN);
                e.u32(*window).bool(*on);
                e
            }
            WinRequest::TextPage { window, serial, node, found, offset, total, text } => {
                let mut e = Encoder::frame(op::TEXT_PAGE);
                e.u32(*window).u32(*serial).u64(*node).bool(*found).u32(*offset).u32(*total).str(text);
                e
            }
            WinRequest::Scene { window, fd, len, forget } => {
                let mut e = Encoder::frame(op::SCENE);
                fds.push(fd.raw());
                e.u8(1).u32(*window).u32(*len).u32(forget.len() as u32);
                for k in forget {
                    e.u64(*k);
                }
                e
            }
            WinRequest::BarStyle { window, style } => {
                let mut e = Encoder::frame(op::BAR_STYLE);
                e.u32(*window).u8(match style {
                    BarStyle::Desktop => 0,
                    BarStyle::Dark => 1,
                    BarStyle::Light => 2,
                });
                e
            }
        };
        Ok((e.finish()?, fds))
    }

    /// A window request from a frame's payload, taking the descriptors it
    /// carries from `fds`.
    pub fn decode(opcode: u16, payload: &[u8], fds: &mut dyn FdSource) -> Result<WinRequest, WireError> {
        let mut d = Decoder::new(payload);
        Ok(match opcode {
            op::CREATE => WinRequest::Create {
                serial: d.u32()?,
                window: d.u32()?,
                role: d.str()?,
                token: d.str()?,
                title: d.str()?,
                width: finite(d.f64()?, "width")?,
                height: finite(d.f64()?, "height")?,
                parent: d.u32()?,
                place: get_place(&mut d)?,
            },
            op::DESTROY => WinRequest::Destroy { window: d.u32()? },
            op::TITLE => WinRequest::Title { window: d.u32()?, title: d.str()? },
            op::ACK_CONFIGURE => WinRequest::AckConfigure { window: d.u32()?, serial: d.u32()? },
            op::ATTACH => {
                let mut got = take(fds, &mut d)?;
                let window = d.u32()?;
                let buffer = match d.u8()? {
                    0 => {
                        let (width, height, stride, format) = (d.u32()?, d.u32()?, d.u32()?, d.u32()?);
                        size(width, height)?;
                        if got.len() != 1
                            || (stride as u64) < width as u64 * 4
                            || stride as u64 * height as u64 > 1 << 30
                        {
                            return Err(WireError::BadValue("shared memory picture"));
                        }
                        Buffer::Shm { fd: got.remove(0), width, height, stride, format }
                    }
                    1 => {
                        let (width, height, format, modifier) = (d.u32()?, d.u32()?, d.u32()?, d.u64()?);
                        size(width, height)?;
                        if got.is_empty() || got.len() > MAX_PLANES {
                            return Err(WireError::BadValue("GPU picture planes"));
                        }
                        let mut planes = Vec::with_capacity(got.len());
                        for fd in got {
                            planes.push(Plane { fd, offset: d.u32()?, stride: d.u32()? });
                        }
                        Buffer::Dmabuf { planes, width, height, format, modifier }
                    }
                    _ => return Err(WireError::BadValue("picture kind")),
                };
                let n = d.count(16)?;
                if n > MAX_DAMAGE {
                    return Err(WireError::TooLong(n));
                }
                let damage = (0..n)
                    .map(|_| Ok(Damage { x: d.i32()?, y: d.i32()?, w: d.i32()?, h: d.i32()? }))
                    .collect::<Result<_, WireError>>()?;
                WinRequest::Attach { window, buffer, damage }
            }
            op::CURSOR => WinRequest::Cursor { window: d.u32()?, shape: d.u16()? },
            op::MAXIMIZE => WinRequest::Maximize { window: d.u32()?, on: d.bool()? },
            op::MINIMIZE => WinRequest::Minimize { window: d.u32()? },
            op::PLACE => WinRequest::Place { window: d.u32()?, place: get_place(&mut d)? },
            op::MEANING => {
                let window = d.u32()?;
                let focus = d.u64()?;
                let caret = if d.bool()? {
                    Some((
                        finite(d.f64()?, "caret")?,
                        finite(d.f64()?, "caret")?,
                        finite(d.f64()?, "caret")?,
                        finite(d.f64()?, "caret")?,
                    ))
                } else {
                    None
                };
                let nodes = get_nodes(&mut d)?;
                WinRequest::Meaning { window, focus, caret, nodes }
            }
            op::COPY => WinRequest::Copy { input_serial: d.u32()?, kinds: get_kinds(&mut d)? },
            op::PASTE => WinRequest::Paste { serial: d.u32()?, kind: d.str()? },
            op::BAR => WinRequest::Bar { window: d.u32()?, bar: get_bar(&mut d)? },
            op::UNDER_BAR => WinRequest::UnderBar { window: d.u32()? },
            op::STYLE => {
                let window = d.u32()?;
                let kind = WinKind::from_byte(d.u8()?)?;
                let modal = d.u8()? != 0;
                let min = (limit(d.f64()?, "smallest width")?, limit(d.f64()?, "smallest height")?);
                let max = (limit(d.f64()?, "largest width")?, limit(d.f64()?, "largest height")?);
                // A largest size below the smallest on an axis that has both is no limit at all.
                if (max.0 > 0.0 && max.0 < min.0) || (max.1 > 0.0 && max.1 < min.1) {
                    return Err(WireError::BadValue("largest size below the smallest"));
                }
                let frosted = d.bool()?;
                WinRequest::Style { window, kind, modal: modal && kind == WinKind::Dialog, min, max, frosted }
            }
            op::COMMANDS => {
                use crate::commands::{Command, MAX_COMMANDS, MAX_LABEL, Role};
                let window = d.u32()?;
                let terminal = d.bool()?;
                let n = d.count(16)?;
                if n > MAX_COMMANDS {
                    return Err(WireError::TooLong(n));
                }
                let text = |d: &mut Decoder| -> Result<String, WireError> {
                    let t = d.str()?;
                    if t.len() > MAX_LABEL || t.chars().any(char::is_control) {
                        Err(WireError::BadValue("command text"))
                    } else {
                        Ok(t)
                    }
                };
                let mut commands: Vec<Command> = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = d.u32()?;
                    let role = Role::from_code(d.u16()?).ok_or(WireError::BadValue("command role"))?;
                    let (menu, label, keys, enabled) = (text(&mut d)?, text(&mut d)?, text(&mut d)?, d.bool()?);
                    let checked = match d.u8()? {
                        0 => None,
                        1 => Some(false),
                        2 => Some(true),
                        _ => return Err(WireError::BadValue("command switch")),
                    };
                    let featured = match d.u8()? {
                        0 => false,
                        1 => true,
                        _ => return Err(WireError::BadValue("command featured")),
                    };
                    if id == 0 || commands.iter().any(|c| c.id == id) {
                        return Err(WireError::BadValue("command id"));
                    }
                    if role == Role::Own && label.is_empty() {
                        return Err(WireError::BadValue("command label"));
                    }
                    commands.push(Command { id, role, menu, label, keys, enabled, checked, featured });
                }
                WinRequest::Commands { window, terminal, commands }
            }
            op::SIZE => {
                let window = d.u32()?;
                let (width, height) = (finite(d.f64()?, "width")?, finite(d.f64()?, "height")?);
                if !(1.0..=16384.0).contains(&width) || !(1.0..=16384.0).contains(&height) {
                    return Err(WireError::BadValue("size"));
                }
                WinRequest::Size { window, width, height }
            }
            op::BAR_STYLE => {
                let window = d.u32()?;
                let style = match d.u8()? {
                    0 => BarStyle::Desktop,
                    1 => BarStyle::Dark,
                    2 => BarStyle::Light,
                    _ => return Err(WireError::BadValue("bar style")),
                };
                WinRequest::BarStyle { window, style }
            }
            op::DRAG_START => WinRequest::DragStart { window: d.u32()?, label: d.str()?, items: get_items(&mut d)? },
            op::DRAG_ACCEPT => {
                let window = d.u32()?;
                let action = d.u8()?;
                if action > DROP_MOVE {
                    return Err(WireError::BadValue("drop action"));
                }
                WinRequest::DragAccept { window, action }
            }
            op::ACTIVATE => WinRequest::Activate { window: d.u32()? },
            op::CONTEXT_MENU => {
                WinRequest::ContextMenu { window: d.u32()?, x: finite(d.f64()?, "x")?, y: finite(d.f64()?, "y")? }
            }
            op::MENU => {
                let (window, x, y) = (d.u32()?, finite(d.f64()?, "x")?, finite(d.f64()?, "y")?);
                WinRequest::Menu { window, x, y, lines: get_menu_lines(&mut d, false)? }
            }
            op::GROUND_CLICKED => WinRequest::GroundClicked { window: d.u32()? },
            op::ACTED => WinRequest::Acted { window: d.u32()?, serial: d.u32()?, ok: d.bool()?, why: d.str()? },
            op::REVOKE_AGENT => WinRequest::RevokeAgent { window: d.u32()?, node: d.u64()?, agent: d.str()? },
            op::CAPTURE_KEYS => WinRequest::CaptureKeys { window: d.u32()?, on: d.bool()? },
            op::FULLSCREEN => WinRequest::Fullscreen { window: d.u32()?, on: d.bool()? },
            op::TEXT_PAGE => {
                let (window, serial, node, found, offset, total) =
                    (d.u32()?, d.u32()?, d.u64()?, d.bool()?, d.u32()?, d.u32()?);
                let text = d.str()?;
                if text.chars().count() > TEXT_PAGE_MAX as usize {
                    return Err(WireError::TooLong(text.len()));
                }
                WinRequest::TextPage { window, serial, node, found, offset, total, text }
            }
            op::SCENE => {
                let mut got = take(fds, &mut d)?;
                if got.len() != 1 {
                    return Err(WireError::BadValue("a scene's descriptor"));
                }
                let (window, len) = (d.u32()?, d.u32()?);
                if len > MAX_SCENE_BYTES {
                    return Err(WireError::TooLong(len as usize));
                }
                let n = d.count(8)?;
                if n > MAX_FORGET {
                    return Err(WireError::TooLong(n));
                }
                let forget = (0..n).map(|_| d.u64()).collect::<Result<_, WireError>>()?;
                WinRequest::Scene { window, fd: got.remove(0), len, forget }
            }
            other => return Err(WireError::UnknownOpcode(other)),
        })
    }

    /// The serial of a request that has a reply.
    pub fn serial(&self) -> Option<u32> {
        match self {
            WinRequest::Create { serial, .. } | WinRequest::Paste { serial, .. } => Some(*serial),
            _ => None,
        }
    }
}

impl WinEvent {
    pub fn encode(&self) -> Result<(Vec<u8>, Vec<RawFd>), WireError> {
        let mut fds = Vec::new();
        let e = match self {
            WinEvent::Created { serial, window, token } => {
                let mut e = Encoder::frame(op::CREATED);
                e.u32(*serial).u32(*window).str(token);
                e
            }
            WinEvent::Configure { window, serial, width, height, scale, maximized, activated, inset, text } => {
                let mut e = Encoder::frame(op::CONFIGURE);
                e.u32(*window)
                    .u32(*serial)
                    .f64(*width)
                    .f64(*height)
                    .f64(*scale)
                    .bool(*maximized)
                    .bool(*activated)
                    .f64(*inset)
                    .f64(*text);
                e
            }
            WinEvent::Close { window } => {
                let mut e = Encoder::frame(op::CLOSE);
                e.u32(*window);
                e
            }
            WinEvent::Frame { window, time_ms } => {
                let mut e = Encoder::frame(op::FRAME);
                e.u32(*window).u32(*time_ms);
                e
            }
            WinEvent::PointerEnter { window, serial, x, y } => {
                let mut e = Encoder::frame(op::POINTER_ENTER);
                e.u32(*window).u32(*serial).f64(*x).f64(*y);
                e
            }
            WinEvent::PointerLeave { window } => {
                let mut e = Encoder::frame(op::POINTER_LEAVE);
                e.u32(*window);
                e
            }
            WinEvent::PointerMotion { window, time_ms, x, y } => {
                let mut e = Encoder::frame(op::POINTER_MOTION);
                e.u32(*window).u32(*time_ms).f64(*x).f64(*y);
                e
            }
            WinEvent::PointerButton { window, serial, time_ms, button, pressed } => {
                let mut e = Encoder::frame(op::POINTER_BUTTON);
                e.u32(*window).u32(*serial).u32(*time_ms).u32(*button).bool(*pressed);
                e
            }
            WinEvent::PointerAxis { window, time_ms, source, dx, dy, v120x, v120y } => {
                let mut e = Encoder::frame(op::POINTER_AXIS);
                e.u32(*window).u32(*time_ms).u8(*source).f64(*dx).f64(*dy).i32(*v120x).i32(*v120y);
                e
            }
            WinEvent::Keymap { fd, size } => {
                fds.push(fd.raw());
                let mut e = Encoder::frame(op::KEYMAP);
                e.u8(1).u32(*size);
                e
            }
            WinEvent::KeyboardEnter { window, serial } => {
                let mut e = Encoder::frame(op::KEYBOARD_ENTER);
                e.u32(*window).u32(*serial);
                e
            }
            WinEvent::KeyboardLeave { window } => {
                let mut e = Encoder::frame(op::KEYBOARD_LEAVE);
                e.u32(*window);
                e
            }
            WinEvent::Key { window, serial, time_ms, key, pressed } => {
                let mut e = Encoder::frame(op::KEY);
                e.u32(*window).u32(*serial).u32(*time_ms).u32(*key).bool(*pressed);
                e
            }
            WinEvent::Modifiers { depressed, latched, locked, group } => {
                let mut e = Encoder::frame(op::MODIFIERS);
                e.u32(*depressed).u32(*latched).u32(*locked).u32(*group);
                e
            }
            WinEvent::Repeat { rate, delay } => {
                let mut e = Encoder::frame(op::REPEAT);
                e.i32(*rate).i32(*delay);
                e
            }
            WinEvent::Touch { window, time_ms, id, phase, x, y } => {
                let mut e = Encoder::frame(op::TOUCH);
                e.u32(*window).u32(*time_ms).i32(*id).u8(*phase).f64(*x).f64(*y);
                e
            }
            WinEvent::ClipboardOffer { kinds } => {
                let mut e = Encoder::frame(op::CLIPBOARD_OFFER);
                put_kinds(&mut e, kinds);
                e
            }
            WinEvent::ClipboardSend { kind, fd } => {
                fds.push(fd.raw());
                let mut e = Encoder::frame(op::CLIPBOARD_SEND);
                e.u8(1).str(kind);
                e
            }
            WinEvent::ClipboardData { serial, fd } => {
                fds.push(fd.raw());
                let mut e = Encoder::frame(op::CLIPBOARD_DATA);
                e.u8(1).u32(*serial);
                e
            }
            WinEvent::TabChosen { window, tab } => {
                let mut e = Encoder::frame(op::TAB_CHOSEN);
                e.u32(*window).u32(*tab);
                e
            }
            WinEvent::TabClose { window, tab } => {
                let mut e = Encoder::frame(op::TAB_CLOSE);
                e.u32(*window).u32(*tab);
                e
            }
            WinEvent::TabCloseOthers { window, tab } => {
                let mut e = Encoder::frame(op::TAB_CLOSE_OTHERS);
                e.u32(*window).u32(*tab);
                e
            }
            WinEvent::TabNew { window } => {
                let mut e = Encoder::frame(op::TAB_NEW);
                e.u32(*window);
                e
            }
            WinEvent::TabMoved { window, tab, to } => {
                let mut e = Encoder::frame(op::TAB_MOVED);
                e.u32(*window).u32(*tab).u32(*to);
                e
            }
            WinEvent::TabOut { window, tab, place } => {
                let mut e = Encoder::frame(op::TAB_OUT);
                e.u32(*window).u32(*tab);
                put_place(&mut e, place);
                e
            }
            WinEvent::TabJoin { window, tab, into, to } => {
                let mut e = Encoder::frame(op::TAB_JOIN);
                e.u32(*window).u32(*tab).u32(*into).u32(*to);
                e
            }
            WinEvent::BarButton { window, id } => {
                let mut e = Encoder::frame(op::BAR_BUTTON);
                e.u32(*window).u32(*id);
                e
            }
            WinEvent::Command { window, id } => {
                let mut e = Encoder::frame(op::COMMAND);
                e.u32(*window).u32(*id);
                e
            }
            WinEvent::DragOver { window, x, y, kinds, mods } => {
                let mut e = Encoder::frame(op::DRAG_OVER);
                e.u32(*window).f64(*x).f64(*y).u8(*mods);
                put_kinds(&mut e, kinds);
                e
            }
            WinEvent::DragLeave { window } => {
                let mut e = Encoder::frame(op::DRAG_LEAVE);
                e.u32(*window);
                e
            }
            WinEvent::DragDrop { window, x, y, action, items } => {
                let mut e = Encoder::frame(op::DRAG_DROP);
                e.u32(*window).f64(*x).f64(*y).u8(*action);
                put_items(&mut e, items);
                e
            }
            WinEvent::DragEnded { window, action } => {
                let mut e = Encoder::frame(op::DRAG_ENDED);
                e.u32(*window).u8(*action);
                e
            }
            WinEvent::Act { window, serial, node, action } => {
                let mut e = Encoder::frame(op::ACT);
                e.u32(*window).u32(*serial).u64(*node);
                put_action(&mut e, action);
                e
            }
            WinEvent::SessionEnding { window } => {
                let mut e = Encoder::frame(op::SESSION_ENDING);
                e.u32(*window);
                e
            }
            WinEvent::AgentActing { window, node, agent } => {
                let mut e = Encoder::frame(op::AGENT_ACTING);
                e.u32(*window).u64(*node).str(agent);
                e
            }
            WinEvent::KeysCaptured { window, on } => {
                let mut e = Encoder::frame(op::KEYS_CAPTURED);
                e.u32(*window).bool(*on);
                e
            }
            WinEvent::Seen { window, seen } => {
                let mut e = Encoder::frame(op::SEEN);
                e.u32(*window).bool(*seen);
                e
            }
            WinEvent::AgentTabs { window, tabs } => {
                let mut e = Encoder::frame(op::AGENT_TABS);
                e.u32(*window).count(tabs.len());
                for t in tabs {
                    e.u64(t.node).str(&t.agent).bool(t.typing);
                }
                e
            }
            WinEvent::ReadText { window, serial, node, offset, len } => {
                let mut e = Encoder::frame(op::READ_TEXT);
                e.u32(*window).u32(*serial).u64(*node).u32(*offset).u32(*len);
                e
            }
            WinEvent::ScenesOff { window, why } => {
                let mut e = Encoder::frame(op::SCENES_OFF);
                e.u32(*window).str(why);
                e
            }
            WinEvent::Tint { window, rgb } => {
                let mut e = Encoder::frame(op::TINT);
                e.u32(*window);
                match rgb {
                    Some(c) => e.bool(true).u32(*c & 0xff_ffff),
                    None => e.bool(false),
                };
                e
            }
            WinEvent::TabMenu { window, tab, entry } => {
                let mut e = Encoder::frame(op::TAB_MENU);
                e.u32(*window).u32(*tab).u32(*entry);
                e
            }
        };
        Ok((e.finish()?, fds))
    }

    pub fn decode(opcode: u16, payload: &[u8], fds: &mut dyn FdSource) -> Result<WinEvent, WireError> {
        let mut d = Decoder::new(payload);
        let one = |fds: &mut dyn FdSource, d: &mut Decoder| -> Result<Fd, WireError> {
            let mut got = take(fds, d)?;
            if got.len() != 1 {
                return Err(WireError::BadValue("descriptor count"));
            }
            Ok(got.remove(0))
        };
        Ok(match opcode {
            op::CREATED => WinEvent::Created { serial: d.u32()?, window: d.u32()?, token: d.str()? },
            op::CONFIGURE => WinEvent::Configure {
                window: d.u32()?,
                serial: d.u32()?,
                width: finite(d.f64()?, "width")?,
                height: finite(d.f64()?, "height")?,
                scale: finite(d.f64()?, "scale")?,
                maximized: d.bool()?,
                activated: d.bool()?,
                inset: {
                    let inset = finite(d.f64()?, "inset")?;
                    if !(0.0..=MAX_INSET).contains(&inset) {
                        return Err(WireError::BadValue("inset"));
                    }
                    inset
                },
                text: {
                    let text = finite(d.f64()?, "text")?;
                    if !(MIN_TEXT..=MAX_TEXT).contains(&text) {
                        return Err(WireError::BadValue("text"));
                    }
                    text
                },
            },
            op::CLOSE => WinEvent::Close { window: d.u32()? },
            op::FRAME => WinEvent::Frame { window: d.u32()?, time_ms: d.u32()? },
            op::POINTER_ENTER => {
                WinEvent::PointerEnter { window: d.u32()?, serial: d.u32()?, x: d.f64()?, y: d.f64()? }
            }
            op::POINTER_LEAVE => WinEvent::PointerLeave { window: d.u32()? },
            op::POINTER_MOTION => {
                WinEvent::PointerMotion { window: d.u32()?, time_ms: d.u32()?, x: d.f64()?, y: d.f64()? }
            }
            op::POINTER_BUTTON => WinEvent::PointerButton {
                window: d.u32()?,
                serial: d.u32()?,
                time_ms: d.u32()?,
                button: d.u32()?,
                pressed: d.bool()?,
            },
            op::POINTER_AXIS => WinEvent::PointerAxis {
                window: d.u32()?,
                time_ms: d.u32()?,
                source: d.u8()?,
                dx: d.f64()?,
                dy: d.f64()?,
                v120x: d.i32()?,
                v120y: d.i32()?,
            },
            op::KEYMAP => {
                let fd = one(fds, &mut d)?;
                WinEvent::Keymap { fd, size: d.u32()? }
            }
            op::KEYBOARD_ENTER => WinEvent::KeyboardEnter { window: d.u32()?, serial: d.u32()? },
            op::KEYBOARD_LEAVE => WinEvent::KeyboardLeave { window: d.u32()? },
            op::KEY => WinEvent::Key {
                window: d.u32()?,
                serial: d.u32()?,
                time_ms: d.u32()?,
                key: d.u32()?,
                pressed: d.bool()?,
            },
            op::MODIFIERS => {
                WinEvent::Modifiers { depressed: d.u32()?, latched: d.u32()?, locked: d.u32()?, group: d.u32()? }
            }
            op::REPEAT => WinEvent::Repeat { rate: d.i32()?, delay: d.i32()? },
            op::TOUCH => WinEvent::Touch {
                window: d.u32()?,
                time_ms: d.u32()?,
                id: d.i32()?,
                phase: d.u8()?,
                x: d.f64()?,
                y: d.f64()?,
            },
            op::CLIPBOARD_OFFER => WinEvent::ClipboardOffer { kinds: get_kinds(&mut d)? },
            op::CLIPBOARD_SEND => {
                let fd = one(fds, &mut d)?;
                WinEvent::ClipboardSend { kind: d.str()?, fd }
            }
            op::CLIPBOARD_DATA => {
                let fd = one(fds, &mut d)?;
                WinEvent::ClipboardData { serial: d.u32()?, fd }
            }
            op::TAB_CHOSEN => WinEvent::TabChosen { window: d.u32()?, tab: d.u32()? },
            op::TAB_CLOSE => WinEvent::TabClose { window: d.u32()?, tab: d.u32()? },
            op::TAB_CLOSE_OTHERS => WinEvent::TabCloseOthers { window: d.u32()?, tab: d.u32()? },
            op::TAB_NEW => WinEvent::TabNew { window: d.u32()? },
            op::TAB_MOVED => WinEvent::TabMoved { window: d.u32()?, tab: d.u32()?, to: d.u32()? },
            op::TAB_OUT => WinEvent::TabOut { window: d.u32()?, tab: d.u32()?, place: get_place(&mut d)? },
            op::TAB_JOIN => WinEvent::TabJoin { window: d.u32()?, tab: d.u32()?, into: d.u32()?, to: d.u32()? },
            op::TAB_MENU => WinEvent::TabMenu { window: d.u32()?, tab: d.u32()?, entry: d.u32()? },
            op::BAR_BUTTON => WinEvent::BarButton { window: d.u32()?, id: d.u32()? },
            op::TINT => {
                let window = d.u32()?;
                let rgb = if d.bool()? { Some(d.u32()? & 0xff_ffff) } else { None };
                WinEvent::Tint { window, rgb }
            }
            op::COMMAND => WinEvent::Command { window: d.u32()?, id: d.u32()? },
            op::DRAG_OVER => {
                let (window, x, y, mods) = (d.u32()?, finite(d.f64()?, "x")?, finite(d.f64()?, "y")?, d.u8()?);
                WinEvent::DragOver { window, x, y, kinds: get_kinds(&mut d)?, mods }
            }
            op::DRAG_LEAVE => WinEvent::DragLeave { window: d.u32()? },
            op::DRAG_DROP => {
                let (window, x, y, action) = (d.u32()?, finite(d.f64()?, "x")?, finite(d.f64()?, "y")?, d.u8()?);
                WinEvent::DragDrop { window, x, y, action, items: get_items(&mut d)? }
            }
            op::DRAG_ENDED => WinEvent::DragEnded { window: d.u32()?, action: d.u8()? },
            op::ACT => {
                let (window, serial, node) = (d.u32()?, d.u32()?, d.u64()?);
                let action = get_action(&mut d)?;
                WinEvent::Act { window, serial, node, action }
            }
            op::SCENES_OFF => WinEvent::ScenesOff { window: d.u32()?, why: d.str()? },
            op::SESSION_ENDING => WinEvent::SessionEnding { window: d.u32()? },
            op::AGENT_ACTING => WinEvent::AgentActing { window: d.u32()?, node: d.u64()?, agent: d.str()? },
            op::KEYS_CAPTURED => WinEvent::KeysCaptured { window: d.u32()?, on: d.bool()? },
            op::SEEN => WinEvent::Seen { window: d.u32()?, seen: d.bool()? },
            op::AGENT_TABS => {
                let window = d.u32()?;
                let n = d.count(13)?;
                let mut tabs = Vec::with_capacity(n.min(AGENT_TABS_MAX));
                for _ in 0..n {
                    tabs.push(AgentTab { node: d.u64()?, agent: d.str()?, typing: d.bool()? });
                }
                WinEvent::AgentTabs { window, tabs }
            }
            op::READ_TEXT => WinEvent::ReadText {
                window: d.u32()?,
                serial: d.u32()?,
                node: d.u64()?,
                offset: d.u32()?,
                len: d.u32()?,
            },
            other => return Err(WireError::UnknownOpcode(other)),
        })
    }

    /// The serial of an event that is a reply.
    pub fn reply_serial(&self) -> Option<u32> {
        match self {
            WinEvent::Created { serial, .. } | WinEvent::ClipboardData { serial, .. } => Some(*serial),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sock;
    use crate::wire::FrameReader;
    use std::collections::VecDeque;
    use std::os::unix::net::UnixStream;

    /// Sends `bytes` and `fds` across a socket, and reads them back as one frame and its descriptors.
    fn cross(bytes: &[u8], fds: &[RawFd]) -> (u16, Vec<u8>, VecDeque<OwnedFd>) {
        let (a, b) = UnixStream::pair().unwrap();
        sock::send_all(&a, bytes, fds).unwrap();
        let mut got = VecDeque::new();
        let mut reader = FrameReader::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = sock::recv_with_fds(&b, &mut buf, &mut got, 64).unwrap();
            reader.push(&buf[..n]);
            if let Some(f) = reader.next_frame().unwrap() {
                return (f.opcode, f.payload, got);
            }
        }
    }

    fn shm() -> Fd {
        Fd::new(sock::pipe().unwrap().0)
    }

    #[test]
    fn a_bad_style_is_refused_and_only_a_dialog_is_modal() {
        let decode = |m: WinRequest| {
            let (bytes, _) = m.encode().unwrap();
            WinRequest::decode(op::STYLE, &bytes[6..], &mut NoFds)
        };
        let style = |kind, modal, min: (f64, f64), max: (f64, f64)| WinRequest::Style {
            window: 1,
            kind,
            modal,
            min,
            max,
            frosted: false,
        };
        assert!(decode(style(WinKind::Normal, false, (0.0, 0.0), (0.0, 0.0))).is_ok());
        assert!(decode(style(WinKind::Normal, false, (-1.0, 0.0), (0.0, 0.0))).is_err(), "a negative size");
        assert!(decode(style(WinKind::Normal, false, (0.0, f64::NAN), (0.0, 0.0))).is_err(), "not a number");
        assert!(decode(style(WinKind::Normal, false, (500.0, 0.0), (400.0, 0.0))).is_err(), "largest below smallest");
        assert!(
            decode(style(WinKind::Normal, false, (500.0, 300.0), (0.0, 200.0))).is_err(),
            "largest height below smallest"
        );
        // Fixed size: smallest and largest the same.
        assert!(decode(style(WinKind::Normal, false, (400.0, 300.0), (400.0, 300.0))).is_ok());
        // Modal is a dialog's only.
        assert_eq!(
            decode(style(WinKind::Palette, true, (0.0, 0.0), (0.0, 0.0))).unwrap(),
            style(WinKind::Palette, false, (0.0, 0.0), (0.0, 0.0))
        );
        assert_eq!(
            decode(style(WinKind::Dialog, true, (0.0, 0.0), (0.0, 0.0))).unwrap(),
            style(WinKind::Dialog, true, (0.0, 0.0), (0.0, 0.0))
        );
        // An unknown kind.
        let (mut bytes, _) = style(WinKind::Normal, false, (0.0, 0.0), (0.0, 0.0)).encode().unwrap();
        bytes[6 + 4] = 9;
        assert!(WinRequest::decode(op::STYLE, &bytes[6..], &mut NoFds).is_err());
    }

    #[test]
    fn a_bad_menu_is_refused() {
        let menu = |lines: Vec<MenuLine>| WinRequest::Menu { window: 1, x: 1.0, y: 2.0, lines };
        let decode = |m: WinRequest| {
            let (bytes, _) = m.encode().unwrap();
            WinRequest::decode(op::MENU, &bytes[6..], &mut NoFds)
        };
        assert!(decode(menu(vec![])).is_ok());
        assert!(decode(menu(vec![MenuLine::item(0, "Open", "")])).is_err(), "id 0");
        assert!(decode(menu(vec![MenuLine::item(1, "", "")])).is_err(), "no label");
        assert!(decode(menu(vec![MenuLine::item(1, "a\nb", "")])).is_err(), "a control character");
        assert!(
            decode(menu(vec![MenuLine::item(1, &"x".repeat(crate::commands::MAX_LABEL + 1), "")])).is_err(),
            "label too long"
        );
        let many: Vec<MenuLine> = (1..=MAX_MENU_LINES as u32 + 1).map(|i| MenuLine::item(i, "x", "")).collect();
        assert!(decode(menu(many.clone())).is_err(), "too many lines");
        assert!(decode(menu(vec![MenuLine::Sub { label: "S".into(), lines: many }])).is_err(), "too many in a submenu");
        let inner = MenuLine::Sub { label: "In".into(), lines: vec![MenuLine::item(2, "x", "")] };
        assert!(
            decode(menu(vec![MenuLine::Sub { label: "Out".into(), lines: vec![inner] }])).is_err(),
            "a submenu in a submenu"
        );
    }

    #[test]
    fn a_bad_bar_is_refused() {
        let tabs = |ids: &[u32], front: u32| WinRequest::Bar {
            window: 1,
            bar: Bar::Tabs {
                tabs: ids
                    .iter()
                    .map(|&id| Tab { id, title: "t".into(), closable: true, alert: false, edited: false })
                    .collect(),
                front,
                plus: true,
                accent: None,
                menu: vec![],
            },
        };
        let decode = |m: WinRequest| {
            let (bytes, _) = m.encode().unwrap();
            WinRequest::decode(op::BAR, &bytes[6..], &mut NoFds)
        };
        assert!(decode(tabs(&[1, 2], 2)).is_ok());
        assert!(decode(tabs(&[], 0)).is_ok());
        assert!(decode(tabs(&[1, 1], 1)).is_err(), "the same id twice");
        assert!(decode(tabs(&[0, 2], 2)).is_err(), "id 0");
        assert!(decode(tabs(&[1, 2], 3)).is_err(), "front is not a tab");
        assert!(decode(tabs(&[], 5)).is_err(), "front with no tabs");
        let many: Vec<u32> = (1..=MAX_TABS as u32 + 1).collect();
        assert!(decode(tabs(&many, 1)).is_err(), "too many tabs");
        let long = WinRequest::Bar {
            window: 1,
            bar: Bar::Tabs {
                tabs: vec![Tab {
                    id: 1,
                    title: "x".repeat(MAX_TAB_TITLE + 1),
                    closable: true,
                    alert: false,
                    edited: false,
                }],
                front: 1,
                plus: false,
                accent: None,
                menu: vec![],
            },
        };
        assert!(decode(long).is_err(), "title too long");
    }

    #[test]
    fn every_request_survives_the_wire() {
        let a = shm();
        let planes = vec![Plane { fd: shm(), offset: 0, stride: 1024 }, Plane { fd: shm(), offset: 4096, stride: 512 }];
        let all = vec![
            WinRequest::Create {
                serial: 7,
                window: 1,
                role: "main".into(),
                token: "".into(),
                title: "Terminal".into(),
                width: 880.0,
                height: 640.0,
                parent: 0,
                place: Place::Monitor { id: "abc".into(), x: 10.0, y: 20.5 },
            },
            WinRequest::Destroy { window: 1 },
            WinRequest::Title { window: 1, title: "x".into() },
            WinRequest::AckConfigure { window: 1, serial: 9 },
            WinRequest::Attach {
                window: 1,
                buffer: Buffer::Shm { fd: a.clone(), width: 100, height: 50, stride: 400, format: 0 },
                damage: vec![Damage { x: 0, y: 0, w: 10, h: 10 }],
            },
            WinRequest::Attach {
                window: 1,
                buffer: Buffer::Dmabuf { planes, width: 256, height: 128, format: 0x3432_5258, modifier: 1 << 56 },
                damage: vec![],
            },
            WinRequest::Cursor { window: 1, shape: 2 },
            WinRequest::Maximize { window: 1, on: true },
            WinRequest::Minimize { window: 1 },
            WinRequest::Place { window: 1, place: Place::Parent { x: 5.0, y: 6.0 } },
            WinRequest::Meaning {
                window: 1,
                focus: 3,
                caret: Some((1.0, 2.0, 1.5, 14.0)),
                nodes: vec![
                    Node {
                        id: 3,
                        parent: 0,
                        role: "button".into(),
                        name: "Save".into(),
                        value: None,
                        states: vec!["focused".into()],
                        x: 1.0,
                        y: 2.0,
                        w: 30.0,
                        h: 20.0,
                        items: vec![],
                    },
                    Node {
                        id: 4,
                        parent: 0,
                        role: "choice".into(),
                        name: "Font".into(),
                        value: Some("Inter".into()),
                        states: vec![],
                        x: 1.0,
                        y: 30.0,
                        w: 90.0,
                        h: 20.0,
                        items: vec!["Inter".into(), "JetBrains Mono".into()],
                    },
                ],
            },
            WinRequest::Copy { input_serial: 5, kinds: vec!["text/plain;charset=utf-8".into()] },
            WinRequest::Paste { serial: 8, kind: "text/plain".into() },
            WinRequest::Bar { window: 1, bar: Bar::Plain },
            WinRequest::Bar {
                window: 1,
                bar: Bar::Tools {
                    buttons: vec![
                        BarButton { id: 1, icon: BarIcon::Back, name: "Back".into(), enabled: false },
                        BarButton { id: 2, icon: BarIcon::Forward, name: "Forward".into(), enabled: true },
                        BarButton { id: 3, icon: BarIcon::Label, name: "Open with Reader".into(), enabled: true },
                    ],
                    from: 220.0,
                },
            },
            WinRequest::UnderBar { window: 1 },
            WinRequest::Style {
                window: 1,
                kind: WinKind::Dialog,
                modal: true,
                min: (320.0, 200.0),
                max: (0.0, 0.0),
                frosted: false,
            },
            WinRequest::Style {
                window: 2,
                kind: WinKind::Sidebar,
                modal: false,
                min: (760.0, 480.0),
                max: (1100.0, 0.0),
                frosted: false,
            },
            WinRequest::Style {
                window: 3,
                kind: WinKind::Dialog,
                modal: false,
                min: (240.0, 160.0),
                max: (0.0, 0.0),
                frosted: true,
            },
            WinRequest::Commands {
                window: 1,
                terminal: true,
                commands: vec![
                    crate::commands::Command::role(1, crate::commands::Role::Copy).enabled(false),
                    crate::commands::Command::own(2, "Tab", "Split side by side", "Ctrl+Shift+R").featured(),
                    crate::commands::Command::own(3, "View", "Show scroll bar", "").checked(true),
                ],
            },
            WinRequest::BarStyle { window: 1, style: BarStyle::Light },
            WinRequest::Size { window: 1, width: 680.0, height: 560.0 },
            WinRequest::DragStart {
                window: 1,
                label: "2 items".into(),
                items: vec![
                    ("text/uri-list".into(), b"file:///a\r\nfile:///b\r\n".to_vec()),
                    ("text/plain".into(), b"/a\n/b".to_vec()),
                ],
            },
            WinRequest::DragAccept { window: 1, action: DROP_MOVE },
            WinRequest::Activate { window: 1 },
            WinRequest::ContextMenu { window: 1, x: 40.0, y: 60.5 },
            WinRequest::GroundClicked { window: 1 },
            WinRequest::Acted { window: 1, serial: 7, ok: false, why: "it is off".into() },
            WinRequest::Acted { window: 2, serial: 8, ok: true, why: String::new() },
            WinRequest::RevokeAgent { window: 3, node: 1 << 60 | 9, agent: "Claude".into() },
            WinRequest::CaptureKeys { window: 3, on: true },
            WinRequest::Fullscreen { window: 3, on: true },
            WinRequest::Fullscreen { window: 4, on: false },
            WinRequest::TextPage {
                window: 1,
                serial: 9,
                node: 1 << 40,
                found: true,
                offset: 512,
                total: 2000,
                text: "wörld ".repeat(40),
            },
            WinRequest::TextPage {
                window: 1,
                serial: 10,
                node: 7,
                found: false,
                offset: 0,
                total: 0,
                text: String::new(),
            },
            WinRequest::Scene { window: 1, fd: a.clone(), len: 4096, forget: vec![7, 1 << 63] },
            WinRequest::Menu {
                window: 1,
                x: 12.0,
                y: 30.5,
                lines: vec![
                    MenuLine::item(1, "Open", "Enter"),
                    MenuLine::Sub {
                        label: "Open With".into(),
                        lines: vec![
                            MenuLine::item(100, "Edit", ""),
                            MenuLine::Separator,
                            MenuLine::item(101, "Other…", "").enabled(false),
                        ],
                    },
                    MenuLine::Separator,
                    MenuLine::item(2, "Hidden files", "Ctrl+H").checked(true),
                ],
            },
            WinRequest::BarStyle { window: 2, style: BarStyle::Desktop },
            WinRequest::Bar {
                window: 1,
                bar: Bar::Tabs {
                    tabs: vec![
                        Tab { id: 4, title: "~/src".into(), closable: true, alert: false, edited: true },
                        Tab { id: 9, title: "htop".into(), closable: false, alert: true, edited: false },
                    ],
                    front: 9,
                    plus: true,
                    accent: Some(0x3b82f6),
                    menu: vec![MenuEntry { id: 1, label: "Rename".into() }],
                },
            },
        ];
        for m in all {
            let (bytes, fds) = m.encode().unwrap();
            let (opcode, payload, mut got) = cross(&bytes, &fds);
            let back = WinRequest::decode(opcode, &payload, &mut got).unwrap();
            // Descriptors arrive as new ones: compare everything else.
            assert_eq!(format!("{:?}", strip(&back)), format!("{:?}", strip(&m)), "{m:?}");
            assert!(got.is_empty(), "every descriptor claimed");
        }
    }

    /// The message with its descriptors' numbers hidden.
    fn strip(m: &WinRequest) -> String {
        let s = format!("{m:?}");
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(i) = rest.find("Fd(") {
            out += &rest[..i + 3];
            let j = rest[i..].find(')').unwrap();
            out += "_";
            rest = &rest[i + j..];
        }
        out + rest
    }

    #[test]
    fn every_event_survives_the_wire() {
        let all = vec![
            WinEvent::Created { serial: 7, window: 1, token: "t1".into() },
            WinEvent::Configure {
                window: 1,
                serial: 2,
                width: 880.0,
                height: 640.0,
                scale: 1.5,
                maximized: false,
                activated: true,
                inset: 36.0,
                text: 1.27,
            },
            WinEvent::Close { window: 1 },
            WinEvent::Frame { window: 1, time_ms: 1234 },
            WinEvent::PointerEnter { window: 1, serial: 3, x: 1.5, y: 2.5 },
            WinEvent::PointerLeave { window: 1 },
            WinEvent::PointerMotion { window: 1, time_ms: 5, x: 3.0, y: 4.0 },
            WinEvent::PointerButton { window: 1, serial: 6, time_ms: 7, button: 0x110, pressed: true },
            WinEvent::PointerAxis { window: 1, time_ms: 8, source: 0, dx: 0.0, dy: 15.0, v120x: 0, v120y: 120 },
            WinEvent::Keymap { fd: shm(), size: 4096 },
            WinEvent::KeyboardEnter { window: 1, serial: 9 },
            WinEvent::KeyboardLeave { window: 1 },
            WinEvent::Key { window: 1, serial: 10, time_ms: 11, key: 30, pressed: true },
            WinEvent::Modifiers { depressed: 1, latched: 0, locked: 2, group: 0 },
            WinEvent::Repeat { rate: 30, delay: 500 },
            WinEvent::Touch { window: 1, time_ms: 12, id: 0, phase: 1, x: 5.0, y: 6.0 },
            WinEvent::ClipboardOffer { kinds: vec!["text/plain".into()] },
            WinEvent::ClipboardSend { kind: "text/plain".into(), fd: shm() },
            WinEvent::ClipboardData { serial: 13, fd: shm() },
            WinEvent::TabChosen { window: 1, tab: 4 },
            WinEvent::TabClose { window: 1, tab: 4 },
            WinEvent::TabCloseOthers { window: 1, tab: 4 },
            WinEvent::TabNew { window: 1 },
            WinEvent::TabMoved { window: 1, tab: 4, to: 2 },
            WinEvent::TabOut { window: 1, tab: 4, place: Place::Monitor { id: "8ec9".into(), x: 300.0, y: 200.0 } },
            WinEvent::TabJoin { window: 1, tab: 4, into: 2, to: 0 },
            WinEvent::TabMenu { window: 1, tab: 4, entry: 1 },
            WinEvent::BarButton { window: 1, id: 2 },
            WinEvent::Tint { window: 1, rgb: Some(0x203a5c) },
            WinEvent::Tint { window: 1, rgb: None },
            WinEvent::Command { window: 1, id: 7 },
            WinEvent::DragOver { window: 1, x: 10.5, y: 20.0, kinds: vec!["text/uri-list".into()], mods: DRAG_CTRL },
            WinEvent::DragLeave { window: 1 },
            WinEvent::DragDrop {
                window: 1,
                x: 3.0,
                y: 4.0,
                action: DROP_COPY,
                items: vec![("text/uri-list".into(), b"file:///a\r\n".to_vec())],
            },
            WinEvent::DragEnded { window: 1, action: DROP_NONE },
            WinEvent::Act { window: 1, serial: 7, node: 1 << 40, action: Action::Press },
            WinEvent::Act { window: 1, serial: 8, node: 3, action: Action::Focus },
            WinEvent::Act { window: 1, serial: 9, node: 3, action: Action::Set("Hello, wörld".into()) },
            WinEvent::Act { window: 1, serial: 10, node: 4, action: Action::Choose("JetBrains Mono".into()) },
            WinEvent::Act { window: 1, serial: 11, node: 5, action: Action::Reveal },
            WinEvent::ScenesOff { window: 1, why: "too costly to draw".into() },
            WinEvent::SessionEnding { window: 2 },
            WinEvent::ReadText { window: 2, serial: 12, node: 1 << 33, offset: 8192, len: 8192 },
            WinEvent::Act { window: 1, serial: 13, node: 7, action: Action::Type("echo hi\n".into()) },
            WinEvent::Act { window: 1, serial: 14, node: 7, action: Action::Key("ctrl+c".into()) },
            WinEvent::AgentActing { window: 1, node: 1 << 60 | 3, agent: "Claude".into() },
            WinEvent::AgentTabs { window: 1, tabs: vec![] },
            WinEvent::KeysCaptured { window: 1, on: false },
            WinEvent::Seen { window: 1, seen: false },
            WinEvent::AgentTabs {
                window: 1,
                tabs: vec![
                    AgentTab { node: 1 << 60 | 3, agent: "Claude".into(), typing: true },
                    AgentTab { node: 1 << 60 | 4, agent: "Helper".into(), typing: false },
                ],
            },
        ];
        for m in all {
            let (bytes, fds) = m.encode().unwrap();
            let (opcode, payload, mut got) = cross(&bytes, &fds);
            let back = WinEvent::decode(opcode, &payload, &mut got).unwrap();
            let hide = |s: String| {
                s.split("Fd(")
                    .enumerate()
                    .map(|(i, p)| if i == 0 { p.to_string() } else { p[p.find(')').unwrap()..].to_string() })
                    .collect::<Vec<_>>()
                    .join("Fd(")
            };
            assert_eq!(hide(format!("{back:?}")), hide(format!("{m:?}")));
            assert!(got.is_empty());
        }
    }

    #[test]
    fn a_picture_that_does_not_add_up_is_refused() {
        let ok = WinRequest::Attach {
            window: 1,
            buffer: Buffer::Shm { fd: shm(), width: 100, height: 50, stride: 400, format: 0 },
            damage: vec![],
        };
        let (bytes, _) = ok.encode().unwrap();
        let payload = &bytes[6..];
        // The descriptor did not come.
        assert!(WinRequest::decode(op::ATTACH, payload, &mut NoFds).is_err());
        // A stride too short for the width.
        let bad = WinRequest::Attach {
            window: 1,
            buffer: Buffer::Shm { fd: shm(), width: 100, height: 50, stride: 100, format: 0 },
            damage: vec![],
        };
        let (bytes, fds) = bad.encode().unwrap();
        let (opcode, payload, mut got) = cross(&bytes, &fds);
        assert!(WinRequest::decode(opcode, &payload, &mut got).is_err());
        // A side too large.
        let huge = WinRequest::Attach {
            window: 1,
            buffer: Buffer::Shm { fd: shm(), width: MAX_SIDE + 1, height: 1, stride: (MAX_SIDE + 1) * 4, format: 0 },
            damage: vec![],
        };
        let (bytes, fds) = huge.encode().unwrap();
        let (opcode, payload, mut got) = cross(&bytes, &fds);
        assert!(WinRequest::decode(opcode, &payload, &mut got).is_err());
    }
}
