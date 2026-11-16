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
/// recent values (oldest first), and a line of small print.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Card {
    pub title: String,
    /// A large line under the title (the clock's time), if any.
    pub big: String,
    pub rows: Vec<(String, String)>,
    pub chart: Vec<f32>,
    pub note: String,
}

impl std::hash::Hash for Card {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (&self.title, &self.big, &self.rows, &self.note).hash(h);
        for v in &self.chart {
            v.to_bits().hash(h);
        }
    }
}
