//! Live tiles (dock milestone 1): what a tile on the dock is, whoever
//! publishes it. The server draws every tile and its card from these
//! values; nobody draws into the dock. Carried on the session bus from
//! step 3 of the milestone (docs/dock-milestone-1.md).

/// How a tile is doing. A tile in trouble is drawn in the warning or
/// problem color, and (step 5) comes out of the drawer by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum TileState {
    #[default]
    Fine,
    Warning,
    Problem,
}

/// One tile: a short value and a label on the dock, its card when the
/// pointer rests on it, and where a click goes.
#[derive(Debug, Clone, PartialEq, Default, Hash)]
pub struct Tile {
    /// Unique among its owner's tiles ("clock", "ups").
    pub id: String,
    /// What the dock shows, one short line each ("18:42", "Mon 16").
    pub value: String,
    pub label: String,
    pub state: TileState,
    /// Where it sorts in the drawer: "System", "Lab", "Background apps".
    pub group: String,
    pub card: Card,
    /// The settings app's page a click opens ("sound", "date & time");
    /// empty: the owner is told of the click instead.
    pub page: String,
}

/// A tile's card: a title, rows of name and value, an optional chart of
/// recent values (oldest first), the few controls used every day, and a
/// line of small print.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Card {
    pub title: String,
    /// A large line under the title (the clock's time), if any.
    pub big: String,
    pub rows: Vec<(String, String)>,
    pub chart: Vec<f32>,
    pub controls: Vec<Control>,
    pub note: String,
    /// A picture beside the card's words (Now Playing's cover; a camera
    /// snapshot, a radar image: the Dockworkers principles' next piece).
    pub picture: Option<Picture>,
}

/// A picture on a card: straight (not premultiplied) RGBA, at most
/// [`MAX_PICTURE`] pixels a side, drawn by the server at the card's scale,
/// with words for what it shows. Its bytes are hashed once, when made, so
/// the card's drawing is not hashed again at every frame.
#[derive(Debug, Clone)]
pub struct Picture {
    pub width: u16,
    pub height: u16,
    pub rgba: std::sync::Arc<Vec<u8>>,
    /// What it shows, in words ("the cover of Wish You Were Here").
    pub alt: String,
    key: u64,
}

impl Picture {
    /// None unless `rgba` holds exactly `width` × `height` pixels within the limits.
    pub fn new(width: u16, height: u16, rgba: Vec<u8>, alt: String) -> Option<Picture> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || w > MAX_PICTURE || h > MAX_PICTURE || rgba.len() != w * h * 4 {
            return None;
        }
        // FNV-1a over the bytes: a cheap key, made once.
        let mut key: u64 = 0xcbf2_9ce4_8422_2325;
        for b in rgba.iter().chain(&width.to_le_bytes()).chain(&height.to_le_bytes()) {
            key = (key ^ *b as u64).wrapping_mul(0x100_0000_01b3);
        }
        Some(Picture { width, height, rgba: std::sync::Arc::new(rgba), alt, key })
    }

    /// The key its bytes were hashed to.
    pub fn key(&self) -> u64 {
        self.key
    }
}

impl PartialEq for Picture {
    fn eq(&self, o: &Picture) -> bool {
        (self.width, self.height, self.key, &self.alt) == (o.width, o.height, o.key, &o.alt)
            && (std::sync::Arc::ptr_eq(&self.rgba, &o.rgba) || self.rgba == o.rgba)
    }
}

impl std::hash::Hash for Picture {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (self.width, self.height, self.key, &self.alt).hash(h);
    }
}

impl std::hash::Hash for Card {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (&self.title, &self.big, &self.rows, &self.note, &self.controls, &self.picture).hash(h);
        for v in &self.chart {
            v.to_bits().hash(h);
        }
    }
}

/// A control on a card. Using it tells the tile's owner (the bus event
/// `TileAction`): a button sends 1, a switch its new state as 0 or 1, a
/// slider its new value. The owner then publishes the tile again with the
/// control's new state; the server does not keep it.
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub id: String,
    pub label: String,
    pub kind: ControlKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlKind {
    Button,
    Switch(bool),
    Slider { value: f64, min: f64, max: f64, step: f64 },
}

impl std::hash::Hash for Control {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (&self.id, &self.label).hash(h);
        match self.kind {
            ControlKind::Button => 0u8.hash(h),
            ControlKind::Switch(on) => (1u8, on).hash(h),
            ControlKind::Slider { value, min, max, step } => {
                (2u8, value.to_bits(), min.to_bits(), max.to_bits(), step.to_bits()).hash(h)
            }
        }
    }
}

/// What one client may publish: tiles, and in each tile's card.
pub const MAX_TILES: usize = 32;
pub const MAX_ROWS: usize = 16;
pub const MAX_CHART: usize = 240;
pub const MAX_CONTROLS: usize = 8;
/// The longest id, and the longest value or label (the dock is 48 points wide).
pub const MAX_ID: usize = 64;
pub const MAX_SHORT: usize = 24;
pub const MAX_TEXT: usize = 200;
/// A card's picture: at most this many pixels a side (256 KiB of RGBA).
pub const MAX_PICTURE: usize = 256;

impl Tile {
    /// Whether the server takes this tile, and if not, why.
    pub fn check(&self) -> Result<(), String> {
        let long = |what: &str, s: &str, max: usize| {
            if s.chars().count() > max { Err(format!("{what} longer than {max}")) } else { Ok(()) }
        };
        if self.id.is_empty() || self.id.chars().any(char::is_control) {
            return Err("an empty id, or one with control characters".into());
        }
        long("id", &self.id, MAX_ID)?;
        long("value", &self.value, MAX_SHORT)?;
        long("label", &self.label, MAX_SHORT)?;
        long("group", &self.group, MAX_SHORT)?;
        long("page", &self.page, MAX_SHORT)?;
        let c = &self.card;
        long("title", &c.title, MAX_TEXT)?;
        long("big", &c.big, MAX_SHORT)?;
        long("note", &c.note, MAX_TEXT)?;
        if let Some(p) = &c.picture {
            long("picture's words", &p.alt, MAX_TEXT)?;
        }
        if c.rows.len() > MAX_ROWS || c.chart.len() > MAX_CHART || c.controls.len() > MAX_CONTROLS {
            return Err(format!("more than {MAX_ROWS} rows, {MAX_CHART} chart values or {MAX_CONTROLS} controls"));
        }
        for (k, v) in &c.rows {
            long("row", k, MAX_TEXT)?;
            long("row", v, MAX_TEXT)?;
        }
        if c.chart.iter().any(|v| !v.is_finite()) {
            return Err("a chart value that is not a number".into());
        }
        for k in &c.controls {
            long("control id", &k.id, MAX_ID)?;
            long("control label", &k.label, MAX_TEXT)?;
            if let ControlKind::Slider { value, min, max, step } = k.kind
                && !([value, min, max, step].iter().all(|v| v.is_finite()) && min < max && step >= 0.0)
            {
                return Err(format!("slider {} has no sensible range", k.id));
            }
        }
        Ok(())
    }

    pub fn encode(&self, e: &mut crate::wire::Encoder) {
        e.str(&self.id).str(&self.value).str(&self.label).u8(match self.state {
            TileState::Fine => 0,
            TileState::Warning => 1,
            TileState::Problem => 2,
        });
        e.str(&self.group).str(&self.page);
        let c = &self.card;
        e.str(&c.title).str(&c.big).str(&c.note).count(c.rows.len());
        for (k, v) in &c.rows {
            e.str(k).str(v);
        }
        e.count(c.chart.len());
        for v in &c.chart {
            e.f64(*v as f64);
        }
        e.count(c.controls.len());
        for k in &c.controls {
            e.str(&k.id).str(&k.label);
            match k.kind {
                ControlKind::Button => e.u8(0),
                ControlKind::Switch(on) => e.u8(1).bool(on),
                ControlKind::Slider { value, min, max, step } => e.u8(2).f64(value).f64(min).f64(max).f64(step),
            };
        }
        match &c.picture {
            None => e.u8(0),
            Some(p) => e.u8(1).u16(p.width).u16(p.height).str(&p.alt).bytes(&p.rgba),
        };
    }

    pub fn decode(d: &mut crate::wire::Decoder) -> Result<Tile, crate::wire::WireError> {
        use crate::wire::WireError;
        let id = d.str()?;
        let value = d.str()?;
        let label = d.str()?;
        let state = match d.u8()? {
            0 => TileState::Fine,
            1 => TileState::Warning,
            2 => TileState::Problem,
            _ => return Err(WireError::BadValue("tile state")),
        };
        let group = d.str()?;
        let page = d.str()?;
        let (title, big, note) = (d.str()?, d.str()?, d.str()?);
        let n = d.count(8)?;
        let mut rows = Vec::with_capacity(n.min(MAX_ROWS));
        for _ in 0..n {
            rows.push((d.str()?, d.str()?));
        }
        let n = d.count(8)?;
        let mut chart = Vec::with_capacity(n.min(MAX_CHART));
        for _ in 0..n {
            chart.push(d.f64()? as f32);
        }
        let n = d.count(9)?;
        let mut controls = Vec::with_capacity(n.min(MAX_CONTROLS));
        for _ in 0..n {
            let (id, label) = (d.str()?, d.str()?);
            let kind = match d.u8()? {
                0 => ControlKind::Button,
                1 => ControlKind::Switch(d.bool()?),
                2 => ControlKind::Slider { value: d.f64()?, min: d.f64()?, max: d.f64()?, step: d.f64()? },
                _ => return Err(WireError::BadValue("control kind")),
            };
            controls.push(Control { id, label, kind });
        }
        let picture = match d.u8()? {
            0 => None,
            1 => {
                let (w, h, alt) = (d.u16()?, d.u16()?, d.str()?);
                let rgba = d.bytes()?.to_vec();
                Some(Picture::new(w, h, rgba, alt).ok_or(WireError::BadValue("picture"))?)
            }
            _ => return Err(WireError::BadValue("picture")),
        };
        Ok(Tile {
            id,
            value,
            label,
            state,
            group,
            page,
            card: Card { title, big, rows, chart, controls, note, picture },
        })
    }
}
