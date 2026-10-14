//! One refinement pass over a region: collapse, split, flip, relax (decision 0021, section 2,
//! steps 4 to 8). Serial, so its result depends on nothing but the mesh and its arguments.
//!
//! This file holds the pass's state and the reads every step shares; each step lives in a file
//! of its own (`collapse.rs`, `split.rs`, `flip.rs`, `relax.rs`), and the mirrored pairs in
//! `pair.rs`.

use crate::collapse::Around;
use crate::columns::{AttrDef, AttrId, AttrValue};
use crate::config::{FaceId, VertId};
use crate::math::{Vec3, arity, face_cross};
use crate::topo::TopoEditor;

use super::{LiveDetail, LiveReport, touches};

mod collapse;
mod flip;
mod pair;
mod relax;
mod split;

/// An edge's place in a queue: its squared length as bits (lengths are never negative, so the
/// bits order like the numbers), then its lower and higher end.
type Key = (u32, VertId, VertId);

pub(super) struct Pass<'e, 'a> {
    ed: &'e mut TopoEditor<'a>,
    /// The pass spheres (center, radius): collapses and splits run on the edges of faces
    /// touching one, and nothing moves outside one grown by `max`.
    spheres: &'e [(Vec3, f32)],
    max: f32,
    max2: f32,
    min2: f32,
    ld: LiveDetail,
    mask: Option<usize>,
    group: Option<usize>,
    hide: Option<usize>,
    ops: u32,
    /// The budget ran out: nothing more is done this pass.
    hit: bool,
    /// Points the pass made, kept or turned edges around: flips and relaxation look here.
    changed: Vec<VertId>,
    /// Faces of long edges that wait for room: their other edges wait too, or splitting them
    /// would cut ever thinner triangles against the long edge.
    waiting: Vec<FaceId>,
    /// The mirror map the pass keeps, as (axis, column): every split, collapse and flip then
    /// runs on an edge and its mirror image together (`pair.rs`).
    mirror: Option<(usize, usize)>,
    /// Over a picked region, its faces' corners (ascending): with the points the pass makes,
    /// the only ones in reach.
    only: Option<&'e [VertId]>,
    /// Points the splits made, ascending once the split step is over.
    made: Vec<VertId>,
}

impl<'e, 'a> Pass<'e, 'a> {
    pub(super) fn new(
        ed: &'e mut TopoEditor<'a>,
        spheres: &'e [(Vec3, f32)],
        max: f32,
        min: f32,
        ld: &LiveDetail,
        mirror: Option<(usize, usize)>,
        only: Option<&'e [VertId]>,
    ) -> Pass<'e, 'a> {
        let id = |d: AttrDef| ed.s.attr_id(&d.name).map(AttrId::index);
        let (mask, group, hide) = (
            id(AttrDef::mask()),
            id(AttrDef::face_group()),
            id(AttrDef::hide()),
        );
        Pass {
            ed,
            spheres,
            max,
            max2: max * max,
            min2: min * min,
            ld: *ld,
            mask,
            group,
            hide,
            ops: 0,
            hit: false,
            changed: Vec::new(),
            waiting: Vec::new(),
            mirror,
            only,
            made: Vec::new(),
        }
    }

    /// Run the pass over the region's faces (ascending), every edge of which is a candidate,
    /// adding what it did to `out`; over a `whole` region, every quad left is split too.
    pub(super) fn run(mut self, faces: &[FaceId], whole: bool, out: &mut LiveReport) {
        let edges = self.edges_of(faces);
        if self.ld.combined {
            self.collapse_all(&edges, out);
        }
        self.split_all(&edges, out);
        if whole {
            self.split_quads(faces, out);
        }
        self.made.sort_unstable();
        self.flip_all(out);
        self.relax_all();
        out.budget_hits += u32::from(self.hit);
        out.most_ops = out.most_ops.max(self.ops);
    }

    // ---- reads ----

    fn face_attr(&self, a: Option<usize>, f: FaceId) -> u32 {
        let Some(a) = a else { return 0 };
        let cfg = self.ed.s.cfg;
        let (n, l) = (cfg.face_node(f) as usize, cfg.face_local(f) as usize);
        match self.ed.attr(a, n, l) {
            AttrValue::U32(x) => x,
            AttrValue::F32(x) => x.to_bits(),
        }
    }
    fn hidden(&self, f: FaceId) -> bool {
        self.face_attr(self.hide, f) != 0
    }
    fn group(&self, f: FaceId) -> u32 {
        self.face_attr(self.group, f)
    }
    /// `v`'s mask from 0 to 1; not a number counts as fully masked, as in the influence stage.
    fn mask(&self, v: VertId) -> f32 {
        let Some(a) = self.mask else { return 0.0 };
        let cfg = self.ed.s.cfg;
        let (n, l) = (cfg.node_of(v) as usize, cfg.local_of(v) as usize);
        match self.ed.attr(a, n, l) {
            AttrValue::F32(m) if m.is_nan() => 1.0,
            AttrValue::F32(m) => m.clamp(0.0, 1.0),
            AttrValue::U32(_) => 0.0,
        }
    }
    /// How freely edge a-b changes: 1 unmasked, 0 when both ends are fully masked.
    fn free(&self, a: VertId, b: VertId) -> f32 {
        1.0 - f32::midpoint(self.mask(a), self.mask(b))
    }
    fn len2(&self, a: VertId, b: VertId) -> f32 {
        (self.ed.pos(a) - self.ed.pos(b)).len2()
    }
    /// `length * free > max`, squared (both sides are never negative).
    fn is_long(&self, a: VertId, b: VertId) -> bool {
        let w = self.free(a, b);
        self.len2(a, b) * w * w > self.max2
    }
    /// `length < min * free`, squared.
    fn is_short(&self, a: VertId, b: VertId) -> bool {
        let w = self.free(a, b);
        self.len2(a, b) < self.min2 * w * w
    }
    /// The key of edge a-b and its mirror image: the longer one's squared length, then the
    /// ends of the one with the lower ends ([`Pass::lead`]), which stands for both.
    fn key(&self, a: VertId, b: VertId) -> Key {
        let (lo, hi) = self.lead(a, b);
        let l = self.len2(a, b);
        let l = (self.mirrored(a, b)).map_or(l, |(c, d)| l.max(self.len2(c, d)));
        (l.to_bits(), lo, hi)
    }
    /// Whether edge a-b is a candidate: a face of it touches a pass sphere, as the region's
    /// faces do, so a brush inside one big face still splits it (review 0006, m1).
    fn near(&self, a: VertId, b: VertId) -> bool {
        (self.ed.edge_faces(a, b).into_iter())
            .any(|f| touches(self.spheres, self.ed.face(f), |v| self.ed.pos(v)))
    }
    /// Whether `v` lies inside a pass sphere grown by `max` and, over a picked region, is a
    /// corner of it or a point the pass made: the points a pass may move, remove or turn edges
    /// around. Splits reach past the region to keep the surface whole, so they add points
    /// outside it but move none there.
    fn reach(&self, v: VertId) -> bool {
        let p = self.ed.pos(v);
        let near = |&(c, r): &(Vec3, f32)| (p - c).len2() <= (r + self.max) * (r + self.max);
        let ours =
            |o: &[VertId]| o.binary_search(&v).is_ok() || self.made.binary_search(&v).is_ok();
        self.spheres.iter().any(near) && self.only.is_none_or(ours)
    }
    /// A point touching a hidden face, an open border or a face group border, or where the
    /// surface is not one fan: it is never removed or moved.
    fn feature(&self, v: VertId) -> bool {
        if self.ed.around(v) != Around::Inside {
            return true;
        }
        let ring = self.ed.ring_ref(v);
        let g0 = ring.first().map(|&f| self.group(f));
        ring.iter()
            .any(|&f| self.hidden(f) || Some(self.group(f)) != g0)
    }
    /// Whether any face of a-b is hidden.
    fn touches_hidden(&self, a: VertId, b: VertId) -> bool {
        (self.ed.edge_faces(a, b).iter()).any(|&f| self.hidden(f))
    }
    /// Whether face `f` has no area: two corners at one point, or all on one line. Only
    /// degenerate input holds one, and nothing the pass does makes one (review 0006, S3).
    fn flat(&self, f: FaceId) -> bool {
        face_cross(self.ed.face(f), |v| self.ed.pos(v)) == Vec3::ZERO
    }
    /// The edges of `faces` (consecutive corners), each once, as (lower, higher) ascending.
    fn edges_of(&self, faces: &[FaceId]) -> Vec<(VertId, VertId)> {
        let mut out = Vec::with_capacity(3 * faces.len());
        for &f in faces {
            let fc = self.ed.face(f);
            let k = arity(fc);
            for i in 0..k {
                let (a, b) = (fc[i], fc[(i + 1) % k]);
                out.push((a.min(b), a.max(b)));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
    /// Count `k` operations, or note the budget ran out and refuse.
    fn spend(&mut self, k: u32) -> bool {
        if self.ops + k > self.ld.budget {
            self.hit = true;
            return false;
        }
        self.ops += k;
        true
    }
}
