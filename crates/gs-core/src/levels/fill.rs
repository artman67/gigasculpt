//! Attribute columns of a level that is built (divide) or gathered from the top (a switch),
//! by the rules of decision 0014, section 5.
//!
//! Built: a new point blends its sources (edge ends or face corners, in canonical order; the
//! mask settled on its grid, decision 0015 as amended 2026-10-13), a child face copies its
//! parent's value, and corner columns (UVs) follow decision 0006's linear rule, or
//! [`super::uvs`] when the step smooths them. Gathered: a point reads the top's value
//! at its canonical index; a face shows the majority of its top descendants' labels or flags
//! (ties to the lowest), is hidden only when every descendant is, takes each corner value from
//! that corner's descendant (on the nearest level keeping its own UVs, else the top), and
//! otherwise shows its first descendant's value.

use std::sync::Arc;

use crate::columns::{AttrDef, AttrKind, AttrValue, AttrVec, Domain};
use crate::config::{FaceId, NodeId, VertId};
use crate::math::arity;
use crate::numbering::Numbering;
use crate::store::MeshStore;
use crate::subdivide::{corner_child, ends};

use super::Base;
use super::cut::{Coarse, Layout};
use super::rules::key;
use super::uvs::{Corners, Smooth};

/// One attribute column, per node.
pub(crate) type Column = Vec<Option<Arc<AttrVec>>>;

/// True if some node of `store` holds column `a`.
pub(super) fn held(store: &MeshStore, a: usize) -> bool {
    store.cols.attrs[a].iter().any(Option::is_some)
}

/// Column `a`'s value at slot `slot` of node `node` (the empty value where the node holds none).
pub(super) fn value(store: &MeshStore, a: usize, node: NodeId, slot: u32) -> AttrValue {
    let col = store.cols.attrs[a][node as usize].as_deref();
    col.map_or(AttrValue::zero(store.defs[a].kind), |c| {
        c.get(slot as usize)
    })
}

/// Column `a`'s value at store point `v`.
fn point(store: &MeshStore, a: usize, v: VertId) -> AttrValue {
    value(store, a, store.cfg.node_of(v), store.cfg.local_of(v))
}

/// Column `a`'s value at store face `f`: the face's node and slot.
pub(super) fn face_slot(store: &MeshStore, f: FaceId) -> (NodeId, u32) {
    (store.cfg.face_node(f), store.cfg.face_local(f))
}

/// A column of `kind` with `len[n]` values in node `n`, each from `value(n, slot)`.
fn column(
    kind: AttrKind,
    len: &[usize],
    value: &(impl Fn(usize, usize) -> AttrValue + Sync),
) -> Column {
    gs_par::map(len.len(), |n| {
        let mut col = AttrVec::zeros(kind, len[n]);
        for slot in 0..len[n] {
            col.set(slot, value(n, slot));
        }
        Some(Arc::new(col))
    })
}

/// A corner column: its group's columns, the group's place among the groups, and its corner.
type Corner = (Vec<usize>, usize, usize);

/// Column `a` of `defs` as a corner column, if it holds corner values.
fn corner_of(base: &Base, defs: &[AttrDef], a: usize) -> Option<Corner> {
    let name = &defs[a].name;
    base.corners.iter().enumerate().find_map(|(g, group)| {
        let i = group.iter().position(|x| x == name)?;
        let ids: Option<Vec<usize>> = group
            .iter()
            .map(|x| defs.iter().position(|d| &d.name == x))
            .collect();
        Some((ids?, g, i))
    })
}

pub(super) fn scalar(x: AttrValue) -> f32 {
    match x {
        AttrValue::F32(f) => f,
        AttrValue::U32(u) => f32::from_bits(u),
    }
}

/// Every attribute column of a level built over `c` (see the module docs), the corner
/// columns from `smooth` when the step smooths the UVs.
pub(crate) fn built(
    c: &Coarse,
    layout: &Layout,
    base: &Base,
    smooth: Option<&Smooth>,
) -> Vec<Column> {
    let store = c.store;
    let old = c.num.vertex_count() as u32;
    let points: Vec<usize> = layout.members.iter().map(Vec::len).collect();
    let faces: Vec<usize> = layout.parents.iter().map(Vec::len).collect();
    (0..store.defs.len())
        .map(|a| {
            let def = &store.defs[a];
            if !held(store, a) {
                return vec![None; points.len()];
            }
            match def.domain {
                Domain::Vertex => column(def.kind, &points, &|n, l| {
                    let v = layout.members[n][l];
                    if v < old {
                        return point(store, a, c.num.id(v));
                    }
                    let s = c.src[(v - old) as usize] as usize;
                    let f = c.faces[s / 4];
                    let from: Vec<u32> = if c.step.center[s / 4] == v {
                        key(f)[..arity(f)].to_vec()
                    } else {
                        let (x, y) = ends(f, s % 4);
                        vec![x, y]
                    };
                    let vals: Vec<AttrValue> =
                        from.iter().map(|&u| point(store, a, c.num.id(u))).collect();
                    AttrValue::blend(def, &vals)
                }),
                Domain::Face => {
                    let corner = corner_of(base, &store.defs, a);
                    column(def.kind, &faces, &|n, slot| {
                        let (fi, k) = layout.parents[n][slot];
                        let parent = c.num.face(fi).0;
                        let Some((group, g, i)) = &corner else {
                            let (n, slot) = face_slot(store, parent);
                            return value(store, a, n, slot);
                        };
                        if let Some(s) = smooth {
                            return AttrValue::F32(s.child(c, fi as usize, k as usize)[*i][*g]);
                        }
                        let mut v = [[0.0f32; 1]; 4];
                        for (x, &col) in v.iter_mut().zip(group) {
                            let (n, slot) = face_slot(store, parent);
                            x[0] = scalar(value(store, col, n, slot));
                        }
                        let f = c.faces[fi as usize];
                        AttrValue::F32(corner_child(&v, arity(f), k as usize)[*i][0])
                    })
                }
            }
        })
        .collect()
}

/// The value most of `vals` hold, ties to the lowest.
pub(super) fn majority(mut vals: Vec<u32>) -> u32 {
    vals.sort_unstable();
    let (mut best, mut most, mut i) = (0, 0, 0);
    while i < vals.len() {
        let run = vals[i..].iter().take_while(|&&x| x == vals[i]).count();
        if run > most {
            (best, most) = (vals[i], run);
        }
        i += run;
    }
    best
}

/// The top and its numbering, and the level being gathered.
pub(crate) struct Gather<'a> {
    pub top: &'a MeshStore,
    pub top_num: &'a Numbering,
    pub base: &'a Base,
    pub num: &'a Numbering,
    /// The level gathered and the top's level.
    pub j: usize,
    pub t: usize,
    /// The nearest level at or above `j` (below the top) that keeps its own UVs, and them.
    pub own: Option<(usize, &'a [Corners])>,
}

impl Gather<'_> {
    /// The top's value of face column `a` at canonical top face `f`.
    fn top_face(&self, a: usize, f: u32) -> AttrValue {
        let (n, slot) = face_slot(self.top, self.top_num.face(f).0);
        value(self.top, a, n, slot)
    }

    /// Face column `a` at canonical face `p` of the level (see the module docs).
    fn face_value(&self, a: usize, def: &AttrDef, corner: Option<&Corner>, p: u32) -> AttrValue {
        if let Some((group, g, i)) = corner {
            if let Some((m, own)) = self.own {
                let (d, at) = self.base.corner_descendant(self.j, m, p, *i);
                return AttrValue::F32(own[d as usize][at][*g]);
            }
            let (d, at) = self.base.corner_descendant(self.j, self.t, p, *i);
            return self.top_face(group[at], d);
        }
        let range = self.base.descendants(self.j, self.t, p);
        let bits = |x: AttrValue| match x {
            AttrValue::U32(u) => u,
            AttrValue::F32(f) => f.to_bits(),
        };
        if *def == AttrDef::hide() {
            let all = range.clone().all(|f| bits(self.top_face(a, f)) != 0);
            return AttrValue::U32(u32::from(all));
        }
        if matches!(def.kind, AttrKind::Label | AttrKind::Flag) {
            return AttrValue::U32(majority(range.map(|f| bits(self.top_face(a, f))).collect()));
        }
        self.top_face(a, range.start)
    }

    /// Every attribute column of the level, laid out like `slots` (points per node) and
    /// `faces` (faces per node).
    pub fn columns(&self, slots: &[usize], faces: &[usize]) -> Vec<Column> {
        let defs = &self.top.defs;
        let canon = if defs.iter().any(|d| d.domain == Domain::Face) {
            self.num.face_canon()
        } else {
            Vec::new()
        };
        let starts = self.num.face_starts();
        (0..defs.len())
            .map(|a| {
                let def = &defs[a];
                if !held(self.top, a) {
                    return vec![None; slots.len()];
                }
                match def.domain {
                    Domain::Vertex => column(def.kind, slots, &|n, l| {
                        let c = self.num.index(n as NodeId, l as u32);
                        point(self.top, a, self.top_num.id(c))
                    }),
                    Domain::Face => {
                        let corner = corner_of(self.base, defs, a);
                        column(def.kind, faces, &|n, slot| {
                            let p = canon[starts[n] as usize + slot];
                            self.face_value(a, def, corner.as_ref(), p)
                        })
                    }
                }
            })
            .collect()
    }
}
