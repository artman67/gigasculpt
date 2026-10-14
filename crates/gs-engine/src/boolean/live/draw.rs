//! What the live mode draws (decision 0057, section 9, as amended for LB7), as the window's
//! report carries it ([`crate::CallReport::live`]): each group's result in its base's place,
//! with its cut walls tinted over it, and how each of its parts stands in; and the issue marks.
//! Chosen at each report against the parts as they are now, so a dragged cutter shows the last
//! result with the cutter as an outline, and a stroke on an operand draws the operands plainly.
//! Beside that, each group with no result for its parts as they are now is offered to the
//! window's peel (as amended for LB8), which cuts them on the graphics card as they move.

use gs_core::MeshStore;
use gs_core::transform::Similarity;

use super::{Board, Group, Key, Settled, keyed};
use crate::boolean::groups;
use crate::document::{Document, Operation, Part};

/// What the window colors from its theme: an operation's walls and outlines, or a kind of
/// issue mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    /// An Add operand moved since its result was made, drawn as an outline.
    Union,
    /// A Subtract's walls and outline.
    Difference,
    /// An Intersect's walls and outline.
    Intersect,
    /// Show Issues' marks: open and non-manifold edges, inside-out shells, self-crossings.
    Issue,
    /// Show Coplanar's marks: faces that lie on another operand's.
    Coplanar,
}

/// How a part of a group whose result is drawn stands in for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stand {
    /// Not drawn: the result shows in its place (the base, and an Add operand).
    Hidden,
    /// Drawn as an outline only, in the tint: a Subtract or Intersect operand, or an operand
    /// moved since the result was made.
    Ghost(Tint),
}

/// A mesh the live mode draws beside the parts: a result, a cut wall or an issue mark.
pub struct LiveMesh<'a> {
    /// Changes whenever the mesh is another, so the window uploads each one once, by it.
    pub id: u64,
    pub store: &'a MeshStore,
    /// Part to scene.
    pub transform: Similarity,
    /// `None` for a result; a wall or a mark is drawn over the faces it copies, tinted.
    pub tint: Option<Tint>,
    /// The active part's mark (decision 0033): a result and its walls carry it while the active
    /// part is a part the result hides.
    pub marked: bool,
    /// Its paint shows (the base's or the operand's switch).
    pub painted: bool,
}

/// What the live mode draws: its meshes, and how parts stand in, by part id (`PartReport::id`);
/// and the groups the window may draw through the graphics card's peel instead. Empty while
/// the live mode is off.
#[derive(Default)]
pub struct LiveDraw<'a> {
    pub meshes: Vec<LiveMesh<'a>>,
    pub stands: Vec<(u64, Stand)>,
    pub peels: Vec<LivePeel>,
}

/// A group whose parts as they are now have no settled result (its run still works, or none
/// has started, as while a gizmo drags), which the window may draw from its parts through the
/// graphics card's peel (decision 0057, section 9, as amended for LB8) in place of what
/// `meshes` and `stands` draw for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LivePeel {
    /// Its parts in list order, the base first.
    pub operands: Vec<PeelOperand>,
    /// The meshes `meshes` holds for it now (the last result and its walls), which the peel
    /// replaces.
    pub replaces: Vec<u64>,
}

/// A part of a [`LivePeel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeelOperand {
    /// The part's id (`PartReport::id`).
    pub id: u64,
    /// How it joins: `Union` for the base and an Add, else its operation's tint.
    pub tint: Tint,
    /// The result drawn names it among the ladder's repairs (steps 3 to 5) and it has the same
    /// body now: drawn as a part, left out of the cut.
    pub messy: bool,
}

impl Board {
    /// What to draw for the parts of `doc` as they are now, the marks of the checks `shown`
    /// (Show Issues, Show Coplanar) asks for.
    pub(crate) fn draw(&self, doc: &Document, shown: [bool; 2]) -> LiveDraw<'_> {
        let mut out = LiveDraw::default();
        if self.groups.is_empty() {
            return out;
        }
        let active = doc.active_id();
        let is_active = |part: &Part| active.is_some() && part.props().id == active;
        for group in groups::shown(doc).iter().filter(|g| g.len() >= 2) {
            let key = keyed(doc, group);
            let before = Group::before(&self.groups, &key);
            let first = out.meshes.len();
            if let Some(settled) = before.filter(|s| s.fits(&key)) {
                out.result(settled, &key, is_active);
            }
            let current = (self.groups.iter()).any(|g| g.holds(&key) && g.outcome.is_some());
            if !current {
                let replaces = out.meshes[first..].iter().map(|m| m.id).collect();
                out.peels.push(peel(&key, before.map(|s| &**s), replaces));
            }
        }
        for group in &self.groups {
            out.marks(doc, group, shown);
        }
        out
    }
}

impl<'a> LiveDraw<'a> {
    /// `settled` in its base's place, its walls over it, and each part of `key` it was made
    /// from standing in.
    fn result(&mut self, settled: &'a Settled, key: &[Key], active: impl Fn(&Part) -> bool) {
        let mut marked = false;
        for (k, &(name, part, operation)) in key.iter().enumerate() {
            let Some(was) = settled.from.iter().find(|m| m.name == name) else {
                continue;
            };
            let moved = was.part.transform() != part.transform();
            let stand = match operation {
                _ if k == 0 => Stand::Hidden,
                Operation::Union if !moved => Stand::Hidden,
                _ => Stand::Ghost(tint(operation)),
            };
            marked |= stand == Stand::Hidden && active(part);
            self.stands.push((part.id(), stand));
        }
        let painted = !key[0].1.props().paint_hidden;
        let drawn = |part: &'a Part, tint| LiveMesh {
            id: part.id(),
            store: part.store(),
            transform: part.transform(),
            tint,
            marked,
            painted,
        };
        self.meshes.push(drawn(&settled.part, None));
        for (operation, wall) in &settled.walls {
            self.meshes.push(drawn(wall, Some(tint(*operation))));
        }
    }

    /// `group`'s marks that `shown` asks for, each where its operand is now, while its body is
    /// the one checked.
    fn marks(&mut self, doc: &Document, group: &'a Group, shown: [bool; 2]) {
        for mark in group.marks.iter() {
            let member = &group.members[mark.member];
            let now = (doc.get(&member.name)).filter(|p| p.same_body(&member.part));
            let (Some(now), true) = (now, shown[usize::from(mark.tint == Tint::Coplanar)]) else {
                continue;
            };
            self.meshes.push(LiveMesh {
                id: mark.part.id(),
                store: mark.part.store(),
                transform: now.transform(),
                tint: Some(mark.tint),
                marked: false,
                painted: !now.props().paint_hidden,
            });
        }
    }
}

impl Settled {
    /// True if `key`, a group's parts now, has every part this result was made from by the
    /// same name, body and operation, and its base where it was.
    fn fits(&self, key: &[Key]) -> bool {
        let base = (&self.from[0], key[0]);
        let same = |m: &super::Member, &(name, part, operation): &Key| {
            m.name == name && m.operation == operation && m.part.same_body(part)
        };
        same(base.0, &base.1)
            && base.0.part.transform() == base.1.1.transform()
            && (self.from.iter()).all(|m| key.iter().any(|k| same(m, k)))
    }
}

/// The peel of the group of `key`, which the result `drawn` (if any) tells the messy parts of,
/// in place of the meshes `replaces`.
fn peel(key: &[Key], drawn: Option<&Settled>, replaces: Vec<u64>) -> LivePeel {
    let messy = |name: &str, part: &Part| {
        drawn.is_some_and(|s| {
            s.repairs.iter().any(|r| r.part == name)
                && (s.from.iter()).any(|m| m.name == name && m.part.same_body(part))
        })
    };
    let operands = (key.iter().enumerate())
        .map(|(k, &(name, part, operation))| PeelOperand {
            id: part.id(),
            tint: if k == 0 { Tint::Union } else { tint(operation) },
            messy: messy(name, part),
        })
        .collect();
    LivePeel { operands, replaces }
}

/// The tint of an operation's walls and outline: `Tint`'s first three, in `Operation`'s order.
fn tint(operation: Operation) -> Tint {
    [Tint::Union, Tint::Difference, Tint::Intersect][operation as usize]
}
