//! Panes: a tab's room divided among several terminals.
//!
//! A tab is a tree. A leaf is a terminal (by its number); a split divides
//! its room between two smaller trees, side by side or one above the other,
//! at a share that the line between them can be dragged to change. Where
//! each terminal and each dividing line sits is worked out from the tree and
//! the room ([`Node::layout`]) and used both to draw and to say what is under
//! the pointer.

use vergence_scene::Rect;

/// The dividing line's thickness, and how near the pointer takes hold of it.
pub const DIVIDER: f32 = 1.0;
pub const GRAB: f32 = 4.0;
/// No pane is dragged smaller than this share of its split.
const LEAST: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side by side: the line between is upright.
    Side,
    /// One above the other.
    Stack,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(u64),
    Split {
        axis: Axis,
        /// The first part's share of the room, 0 to 1.
        share: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// A dividing line: the split it belongs to (the turns taken from the root
/// to reach it: false into a first part, true into a second), its
/// direction, where it is drawn, and the room of the split it divides.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub path: Vec<bool>,
    pub axis: Axis,
    pub line: Rect,
    pub room: Rect,
}

impl Divider {
    /// True if a point is near enough to take hold of the line.
    pub fn holds(&self, x: f32, y: f32) -> bool {
        let l = &self.line;
        match self.axis {
            Axis::Side => (x - (l.x + l.w / 2.0)).abs() <= GRAB && y >= l.y && y < l.y + l.h,
            Axis::Stack => (y - (l.y + l.h / 2.0)).abs() <= GRAB && x >= l.x && x < l.x + l.w,
        }
    }

    /// The share that puts the line under a point.
    pub fn share_at(&self, x: f32, y: f32) -> f32 {
        let part = match self.axis {
            Axis::Side => (x - self.room.x) / (self.room.w - DIVIDER).max(1.0),
            Axis::Stack => (y - self.room.y) / (self.room.h - DIVIDER).max(1.0),
        };
        part.clamp(LEAST, 1.0 - LEAST)
    }
}

/// The two parts of a room divided at a share, and the line between them.
fn divide(room: Rect, axis: Axis, share: f32) -> (Rect, Rect, Rect) {
    match axis {
        Axis::Side => {
            let w = ((room.w - DIVIDER) * share).round().max(0.0);
            (Rect::new(room.x, room.y, w, room.h), Rect::new(room.x + w + DIVIDER, room.y, (room.w - w - DIVIDER).max(0.0), room.h), Rect::new(room.x + w, room.y, DIVIDER, room.h))
        }
        Axis::Stack => {
            let h = ((room.h - DIVIDER) * share).round().max(0.0);
            (Rect::new(room.x, room.y, room.w, h), Rect::new(room.x, room.y + h + DIVIDER, room.w, (room.h - h - DIVIDER).max(0.0)), Rect::new(room.x, room.y + h, room.w, DIVIDER))
        }
    }
}

impl Node {
    /// The terminals, in reading order.
    pub fn leaves(&self) -> Vec<u64> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<u64>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    /// Where each terminal and each dividing line is, in a room.
    pub fn layout(&self, room: Rect) -> (Vec<(u64, Rect)>, Vec<Divider>) {
        let (mut panes, mut dividers) = (Vec::new(), Vec::new());
        self.place(room, &mut Vec::new(), &mut panes, &mut dividers);
        (panes, dividers)
    }

    fn place(&self, room: Rect, path: &mut Vec<bool>, panes: &mut Vec<(u64, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Node::Leaf(id) => panes.push((*id, room)),
            Node::Split { axis, share, first, second } => {
                let (a, b, line) = divide(room, *axis, *share);
                dividers.push(Divider { path: path.clone(), axis: *axis, line, room });
                path.push(false);
                first.place(a, path, panes, dividers);
                path.pop();
                path.push(true);
                second.place(b, path, panes, dividers);
                path.pop();
            }
        }
    }

    /// Divides terminal `id`'s room in two: it keeps the first half, `new`
    /// gets the second. False if there is no such terminal.
    pub fn split(&mut self, id: u64, axis: Axis, new: u64) -> bool {
        match self {
            Node::Leaf(own) if *own == id => {
                *self = Node::Split { axis, share: 0.5, first: Box::new(Node::Leaf(id)), second: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => first.split(id, axis, new) || second.split(id, axis, new),
        }
    }

    /// Takes terminal `id` out: the one it shared a split with gets the
    /// whole of that room. False if it is not here, or is the only one.
    pub fn remove(&mut self, id: u64) -> bool {
        let Node::Split { first, second, .. } = self else { return false };
        let other = if **first == Node::Leaf(id) {
            Some(std::mem::replace(&mut **second, Node::Leaf(0)))
        } else if **second == Node::Leaf(id) {
            Some(std::mem::replace(&mut **first, Node::Leaf(0)))
        } else {
            None
        };
        match other {
            Some(other) => {
                *self = other;
                true
            }
            None => first.remove(id) || second.remove(id),
        }
    }

    /// Sets the share of the split reached by `path` (a divider's).
    pub fn set_share(&mut self, path: &[bool], to: f32) {
        if let Node::Split { share, first, second, .. } = self {
            match path.split_first() {
                None => *share = to.clamp(LEAST, 1.0 - LEAST),
                Some((false, rest)) => first.set_share(rest, to),
                Some((true, rest)) => second.set_share(rest, to),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: Rect = Rect::new(0.0, 36.0, 801.0, 401.0);

    #[test]
    fn splitting_shares_the_room() {
        let mut tree = Node::Leaf(1);
        assert_eq!(tree.layout(ROOM), (vec![(1, ROOM)], vec![]));
        assert!(tree.split(1, Axis::Side, 2));
        let (panes, dividers) = tree.layout(ROOM);
        assert_eq!(panes, vec![(1, Rect::new(0.0, 36.0, 400.0, 401.0)), (2, Rect::new(401.0, 36.0, 400.0, 401.0))]);
        assert_eq!(dividers[0].line, Rect::new(400.0, 36.0, 1.0, 401.0));
        // The right one divided again, top and bottom.
        assert!(tree.split(2, Axis::Stack, 3));
        assert!(!tree.split(9, Axis::Stack, 4), "no such terminal");
        let (panes, dividers) = tree.layout(ROOM);
        assert_eq!(tree.leaves(), vec![1, 2, 3]);
        assert_eq!(panes[1], (2, Rect::new(401.0, 36.0, 400.0, 200.0)));
        assert_eq!(panes[2], (3, Rect::new(401.0, 237.0, 400.0, 200.0)));
        assert_eq!((dividers.len(), dividers[1].path.as_slice(), dividers[1].line), (2, &[true][..], Rect::new(401.0, 236.0, 400.0, 1.0)));
        // Every unit of the room is a pane's or a line's.
        let covered: f32 = panes.iter().map(|(_, r)| r.w * r.h).sum::<f32>() + dividers.iter().map(|d| d.line.w * d.line.h).sum::<f32>();
        assert_eq!(covered, ROOM.w * ROOM.h);
    }

    #[test]
    fn closing_gives_the_room_back() {
        let mut tree = Node::Leaf(1);
        assert!(!tree.remove(1), "the only one stays");
        tree.split(1, Axis::Side, 2);
        tree.split(2, Axis::Stack, 3);
        assert!(!tree.remove(9));
        assert!(tree.remove(2));
        assert_eq!(tree.leaves(), vec![1, 3]);
        assert_eq!(tree.layout(ROOM).0[1], (3, Rect::new(401.0, 36.0, 400.0, 401.0)), "its neighbour has the whole right side");
        assert!(tree.remove(1));
        assert_eq!(tree, Node::Leaf(3));
    }

    #[test]
    fn the_line_between_can_be_dragged() {
        let mut tree = Node::Leaf(1);
        tree.split(1, Axis::Side, 2);
        tree.split(2, Axis::Stack, 3);
        let (_, dividers) = tree.layout(ROOM);
        let (upright, level) = (&dividers[0], &dividers[1]);
        assert!(upright.holds(403.0, 100.0) && upright.holds(397.0, 400.0) && !upright.holds(410.0, 100.0) && !upright.holds(400.0, 20.0));
        assert!(level.holds(600.0, 238.0) && !level.holds(300.0, 236.0), "only across its own split");
        assert_eq!(upright.share_at(200.0, 0.0), 0.25);
        assert_eq!((upright.share_at(-50.0, 0.0), upright.share_at(5000.0, 0.0)), (0.1, 0.9), "neither side is squeezed out");
        tree.set_share(&upright.path, 0.25);
        tree.set_share(&level.path, level.share_at(0.0, 36.0 + 300.0));
        let (panes, _) = tree.layout(ROOM);
        assert_eq!(panes[0].1.w, 200.0);
        assert_eq!((panes[1].1.h, panes[2].1.h), (300.0, 100.0));
    }
}
