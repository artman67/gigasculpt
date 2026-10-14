//! The paired pass (decision 0021, section 10): with a mirror map, every edit runs on an edge
//! and its mirror image together, decided once from the mesh before either changes.
//!
//! An edge's image is the edge between its ends' partners. An edge that is its own image either
//! lies in the plane (both ends their own partners) or crosses it (its ends are each other's
//! partners): it is edited alone, and whatever it adds lands exactly on the plane. Every
//! function here reads the identity map when the pass keeps none.

use crate::config::{FaceId, NodeId, VertId};
use crate::math::{NO, arity, same_cycle};
use crate::topo::Edge;

use super::Pass;

impl Pass<'_, '_> {
    /// `v`'s partner, or `v` without a map.
    pub(super) fn twin(&self, v: VertId) -> VertId {
        self.mirror.map_or(v, |(_, col)| self.ed.partner(col, v))
    }

    /// The image of edge a-b, as its ends' partners, unless the pass keeps no map or the edge
    /// is its own image.
    pub(super) fn mirrored(&self, a: VertId, b: VertId) -> Option<(VertId, VertId)> {
        self.mirror?;
        let (c, d) = (self.twin(a), self.twin(b));
        let own = (c, d) == (a, b) || (c, d) == (b, a);
        (!own).then_some((c, d))
    }

    /// The ends (lower, higher) of whichever of edge a-b and its image has the lower ends: the
    /// one that stands for both in a queue.
    pub(super) fn lead(&self, a: VertId, b: VertId) -> (VertId, VertId) {
        let own = (a.min(b), a.max(b));
        self.mirrored(a, b)
            .map_or(own, |(c, d)| own.min((c.min(d), c.max(d))))
    }

    /// Edge a-b lies in the mirror plane: both ends are their own partners.
    pub(super) fn in_plane(&self, a: VertId, b: VertId) -> bool {
        self.mirror.is_some() && self.twin(a) == a && self.twin(b) == b
    }

    /// The pair collapses: both edges short (an in-plane edge never does).
    pub(super) fn short_pair(&self, a: VertId, b: VertId) -> bool {
        !self.in_plane(a, b)
            && self.is_short(a, b)
            && self.mirrored(a, b).is_none_or(|(c, d)| self.is_short(c, d))
    }

    /// The pair splits: the longer of the two is long.
    pub(super) fn long_pair(&self, a: VertId, b: VertId) -> bool {
        self.is_long(a, b) || self.mirrored(a, b).is_some_and(|(c, d)| self.is_long(c, d))
    }

    /// Whether either edge of the pair changes at all (not both ends fully masked).
    pub(super) fn free_pair(&self, a: VertId, b: VertId) -> bool {
        self.free(a, b) > 0.0
            || self
                .mirrored(a, b)
                .is_some_and(|(c, d)| self.free(c, d) > 0.0)
    }

    /// Edges a-b and c-d share no face around any of their ends, so collapsing one leaves
    /// everything the other's checks read as it was.
    pub(super) fn apart(&self, a: VertId, b: VertId, c: VertId, d: VertId) -> bool {
        let mut mine: Vec<FaceId> = [self.ed.ring_ref(a), self.ed.ring_ref(b)].concat();
        mine.sort_unstable();
        [c, d]
            .iter()
            .all(|&v| (self.ed.ring_ref(v).iter()).all(|f| mine.binary_search(f).is_err()))
    }

    /// The side of face `f` whose ends are each other's partners: where a straddling triangle
    /// crosses the plane.
    pub(super) fn crossing(&self, f: FaceId) -> Option<(VertId, VertId)> {
        let fc = self.ed.face(f);
        let k = arity(fc);
        (0..k)
            .map(|i| (fc[i], fc[(i + 1) % k]))
            .find(|&(u, w)| u != w && self.twin(u) == w)
    }

    /// The face that is `f`'s mirror image (its corners' partners, winding reversed).
    pub(super) fn mirror_face(&self, f: FaceId) -> Option<FaceId> {
        let fc = self.ed.face(f);
        let k = arity(fc);
        let mut want = [NO; 4];
        for i in 0..k {
            want[k - 1 - i] = self.twin(fc[i]);
        }
        (self.ed.ring_ref(want[0]).iter().copied()).find(|&g| same_cycle(self.ed.face(g), want))
    }

    /// Whether quad `f`, which is its own image, has a corner on the plane: then both its
    /// diagonals are their own images and either split keeps the map. A quad crossing the
    /// plane between two pairs of partners has no such diagonal and never splits.
    pub(super) fn splits_alone(&self, f: FaceId) -> bool {
        let fc = self.ed.face(f);
        fc[..arity(fc)].iter().any(|&v| self.twin(v) == v)
    }

    /// Nodes for the new points of `e` and its image `e2`: each in the node of its edge's lower
    /// end, else its higher one, with room for a point and two faces each.
    pub(super) fn rooms(&self, e: &Edge, e2: Option<&Edge>) -> Option<(NodeId, NodeId)> {
        let cfg = self.ed.s.cfg;
        let ends = |e: &Edge| [cfg.node_of(e.a.min(e.b)), cfg.node_of(e.a.max(e.b))];
        let Some(e2) = e2 else {
            let n = ends(e).into_iter().find(|&n| self.ed.has_room(n, 1, 2))?;
            return Some((n, n));
        };
        for n in ends(e) {
            for m in ends(e2) {
                let ok = if n == m {
                    self.ed.has_room(n, 2, 4)
                } else {
                    self.ed.has_room(n, 1, 2) && self.ed.has_room(m, 1, 2)
                };
                if ok {
                    return Some((n, m));
                }
            }
        }
        None
    }

    /// Whether edges `e` and its image `e2` turn together: no face in common, and the edges
    /// they would make are not one edge.
    pub(super) fn flips_apart(&self, e: &Edge, e2: &Edge) -> bool {
        if shared_face(e, e2).is_some() {
            return false;
        }
        let new = |e: &Edge| self.ed.wings(e).map(|(c, d)| (c.min(d), c.max(d)));
        new(e) != new(e2)
    }
}

/// The face the edges of a pair share, if any: a triangle straddling the plane, its far corner
/// on it.
pub(super) fn shared_face(e: &Edge, e2: &Edge) -> Option<FaceId> {
    let theirs = [Some(e2.f), e2.g];
    [Some(e.f), e.g]
        .into_iter()
        .flatten()
        .find(|f| theirs.contains(&Some(*f)))
}
