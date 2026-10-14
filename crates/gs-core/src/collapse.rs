//! Edge collapse, the coarsening edit of Live Detail (decision 0021, section 2, step 4), and
//! the neighbourhood questions every Live Detail edit asks.

use crate::config::{FaceId, VertId};
use crate::math::{NO, Vec3, arity};
use crate::topo::{Edge, TopoEditor, next, prev};

/// Where a point sits in the surface, from its ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Around {
    /// Every edge from it has two faces.
    Inside,
    /// Some edge from it has one face: it lies on an open border.
    Border,
    /// Some edge from it has three or more faces, or its faces do not form one fan.
    Tangled,
}

impl TopoEditor<'_> {
    /// Distinct vertices sharing an edge with `v`, ascending.
    pub(crate) fn neighbours_of(&self, v: VertId) -> Vec<VertId> {
        let mut out = self.edge_ends(v);
        out.dedup();
        out
    }

    /// The far end of every face side at `v`, each side once per face, ascending.
    fn edge_ends(&self, v: VertId) -> Vec<VertId> {
        let mut out = Vec::with_capacity(2 * self.ring_ref(v).len());
        for &f in self.ring_ref(v) {
            let fc = self.face(f);
            out.push(next(fc, v));
            out.push(prev(fc, v));
        }
        out.sort_unstable();
        out
    }

    /// Where `v` sits: inside the surface (one closed fan), on an open border (one open fan),
    /// or at a tangle.
    pub(crate) fn around(&self, v: VertId) -> Around {
        self.fan(v).0
    }

    /// [`around`](Self::around) and the number of distinct points sharing an edge with `v`,
    /// `neighbours_of(v).len()`, from one sort of the ends of its sides, on the stack for a
    /// ring of up to 16 faces. The flip and collapse checks ask both of every point near a
    /// candidate edge, and building the lists for it was most of the flip step.
    pub(crate) fn fan(&self, v: VertId) -> (Around, usize) {
        let n = self.ring_ref(v).len();
        let mut stack = [0; 32];
        let mut heap = Vec::new();
        let ends: &mut [VertId] = if 2 * n <= stack.len() {
            &mut stack[..2 * n]
        } else {
            heap.resize(2 * n, 0);
            &mut heap
        };
        for (k, &f) in self.ring_ref(v).iter().enumerate() {
            let fc = self.face(f);
            (ends[2 * k], ends[2 * k + 1]) = (next(fc, v), prev(fc, v));
        }
        ends.sort_unstable();
        let (mut once, mut many, mut distinct) = (0, false, 0);
        for run in ends.chunk_by(|a, b| a == b) {
            distinct += 1;
            once += usize::from(run.len() == 1);
            many |= run.len() > 2;
        }
        let at = match once {
            _ if many => Around::Tangled,
            0 => Around::Inside,
            2 if distinct == n + 1 => Around::Border,
            _ => Around::Tangled,
        };
        (at, distinct)
    }

    /// The corner of triangle `f` that is neither `a` nor `b`.
    pub(crate) fn far_corner(&self, f: FaceId, a: VertId, b: VertId) -> VertId {
        let fc = self.face(f);
        let mut others = fc[..arity(fc)].iter().copied();
        others.find(|&u| u != a && u != b).unwrap_or(NO)
    }

    /// Whether collapsing edge `e` keeps the surface a manifold:
    /// - the link condition: the vertices joined to both ends are exactly the far corners of the
    ///   edge's triangles, and no other face holds both ends;
    /// - each far corner keeps at least three edges (two on a border), so no face folds flat;
    /// - an edge with two triangles does not join two border points (that would pinch the
    ///   surface into a bow tie).
    pub(crate) fn can_collapse(&self, e: &Edge) -> bool {
        let both = (self.ring_ref(e.a).iter())
            .filter(|&&f| self.face(f).contains(&e.b))
            .count();
        let ef = 1 + usize::from(e.g.is_some());
        if both != ef {
            return false;
        }
        let mut far: Vec<VertId> = std::iter::once(e.f)
            .chain(e.g)
            .map(|f| self.far_corner(f, e.a, e.b))
            .collect();
        far.sort_unstable();
        let (na, nb) = (self.neighbours_of(e.a), self.neighbours_of(e.b));
        let common: Vec<VertId> = (na.iter())
            .filter(|u| nb.binary_search(u).is_ok())
            .copied()
            .collect();
        if common != far {
            return false;
        }
        let keeps_shape = |c: VertId| match self.fan(c) {
            (Around::Inside, n) => n > 3,
            (Around::Border, n) => n > 2,
            (Around::Tangled, _) => false,
        };
        if !far.iter().all(|&c| keeps_shape(c)) {
            return false;
        }
        let border = |v| self.around(v) != Around::Inside;
        e.g.is_none() || !(border(e.a) && border(e.b))
    }

    /// Collapse edge `e` into `keep`, one of its ends, which moves to `p`: the edge's triangles
    /// go, every other face of the other end takes `keep` instead, and the other end goes. The
    /// caller checked [`TopoEditor::can_collapse`].
    pub(crate) fn collapse(&mut self, e: Edge, keep: VertId, p: Vec3) {
        let gone = if keep == e.a { e.b } else { e.a };
        let edge: Vec<FaceId> = std::iter::once(e.f).chain(e.g).collect();
        for &f in &edge {
            let c = self.far_corner(f, e.a, e.b);
            self.remove_from_ring(c, f);
        }
        let not_edge = |f: &FaceId| !edge.contains(f);
        let kept: Vec<FaceId> = self.ring(keep).into_iter().filter(not_edge).collect();
        let moved: Vec<FaceId> = self.ring(gone).into_iter().filter(not_edge).collect();
        for &f in &edge {
            self.free_face(f);
        }
        for &f in &moved {
            let fc = self.face(f).map(|u| if u == gone { keep } else { u });
            self.set_face(f, fc);
        }
        self.set_ring(keep, &[kept, moved].concat());
        self.set_pos(keep, p);
        self.free_vert(gone);
    }
}
