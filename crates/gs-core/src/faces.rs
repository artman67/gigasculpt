//! Face-set morphology (decision 0026, 6): a [`FaceSet`] grown or shrunk by one ring of faces
//! sharing a point, grown to whole pieces or whole face groups, and the faces across a face's
//! edges, which the walks here and the group makers (`crate::groups`) flood over.
//!
//! Grow and shrink follow Blender's `grow_shrink_visibility_mesh`
//! (`editors/sculpt_paint/paint_hide.cc`), one step per ring, on faces instead of points.

use std::collections::BTreeSet;

use crate::bits::Bits;
use crate::columns::AttrDef;
use crate::config::{FaceId, NodeId};
use crate::math::{NO, arity};
use crate::select::{FaceSet, Selector};
use crate::store::MeshStore;
use crate::view::{TopoRead, TopoReader};

/// The faces across `f`'s edges that `keep` accepts: every other face holding both ends of one
/// of `f`'s edges, ascending, each once. `out` is cleared first.
pub(crate) fn edge_faces(
    t: &TopoReader<'_>,
    keep: impl Fn(FaceId) -> bool,
    f: FaceId,
    out: &mut Vec<FaceId>,
) {
    out.clear();
    let fc = t.face(f);
    let k = arity(fc);
    for i in 0..k {
        let rb = t.ring(fc[(i + 1) % k]);
        for &g in t.ring(fc[i]) {
            if g != f && rb.binary_search(&g).is_ok() && keep(g) {
                out.push(g);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
}

impl MeshStore {
    /// `set` and every live face sharing a point with one of its faces.
    pub fn grow_faces(&self, set: &FaceSet) -> FaceSet {
        self.by_ring(set, true)
    }

    /// `set` without its faces that share a point with a live face outside it. A face of the
    /// set at an open border of the mesh stays: nothing outside the set is beside it.
    pub fn shrink_faces(&self, set: &FaceSet) -> FaceSet {
        self.by_ring(set, false)
    }

    /// One ring: grown, a face is in when it is in `set` or a face around one of its corners
    /// is; shrunk, when it is in `set` and every face around its corners is too.
    fn by_ring(&self, set: &FaceSet, grow: bool) -> FaceSet {
        let (t, cfg) = (self.topo(), self.cfg);
        let nodes = gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            let inside = set.node(k as NodeId);
            (node.faces.iter().enumerate())
                .map(|(j, &fc)| {
                    let around = |want: bool| {
                        (fc[..arity(fc)].iter())
                            .any(|&v| t.ring(v).iter().any(|&g| set.contains(cfg, g) == want))
                    };
                    let is_in = inside.is_some_and(|b| b.get(j));
                    match (fc[0] == NO, grow) {
                        (true, _) => false,
                        (false, true) => is_in || around(true),
                        (false, false) => is_in && !around(false),
                    }
                })
                .collect()
        });
        FaceSet::from_nodes(nodes)
    }

    /// Every live face of each edge-connected piece holding a face of `set`, hidden faces
    /// included in the walk.
    pub fn shells_of(&self, set: &FaceSet) -> FaceSet {
        let (t, cfg) = (self.topo(), self.cfg);
        let mut picked: Vec<Bits> = (0..self.node_count())
            .map(|k| Bits::zeros(self.node(k as NodeId).faces.len()))
            .collect();
        // Every face of the set seeds one flood; the order faces are reached in does not matter.
        let mut stack = Vec::new();
        for (k, seen) in picked.iter_mut().enumerate() {
            let Some(bits) = set.node(k as NodeId) else {
                continue;
            };
            let first = self.node(k as NodeId).first_face;
            for j in (0..bits.len()).filter(|&j| bits.get(j)) {
                seen.set(j, true);
                stack.push(first + j as u32);
            }
        }
        let mut nb = Vec::new();
        while let Some(f) = stack.pop() {
            edge_faces(&t, |g| t.face(g)[0] != NO, f, &mut nb);
            for &g in &nb {
                let (n, l) = (cfg.face_node(g) as usize, cfg.face_local(g) as usize);
                if !picked[n].get(l) {
                    picked[n].set(l, true);
                    stack.push(g);
                }
            }
        }
        FaceSet::from_nodes(picked)
    }

    /// Every live face whose face group a face of `set` holds (group 0, no group, counts).
    pub fn groups_of(&self, set: &FaceSet) -> FaceSet {
        let g = self.attr_id(&AttrDef::face_group().name);
        let reader = g.map(|g| self.attr_reader(g));
        let held: Vec<BTreeSet<u32>> = gs_par::map(self.node_count(), |k| {
            let first = self.node(k as NodeId).first_face;
            let Some(bits) = set.node(k as NodeId) else {
                return BTreeSet::new();
            };
            (0..bits.len())
                .filter(|&j| bits.get(j))
                .map(|j| reader.map_or(0, |r| r.label(first + j as u32)))
                .collect()
        });
        let ids: BTreeSet<u32> = held.into_iter().flatten().collect();
        if ids.is_empty() {
            return FaceSet::default();
        }
        self.select(&Selector::Groups(ids.into_iter().collect()))
    }
}
