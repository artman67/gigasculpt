//! What an in-place edit changed in the nodes it wrote, against the open step's start (see
//! `effect` for what counts as touched): the vertices it moved; every vertex it touched (moved,
//! with a vertex attribute value changed, or a corner of a face whose face attribute value
//! changed); and whether an attribute column appeared or vanished, which touches every vertex,
//! because the part's exported columns changed shape. In a node whose topology changed (a
//! refining stroke) every live vertex counts as moved. It also notes an attribute value written
//! that is not a finite number, which `sound` refuses, and names the columns that changed.
//!
//! A part with a transform (decision 0029) is moved in its own space, but touched where its
//! scene point moved: a move smaller than the rounding of the scene point is a change of the
//! part that `part.get` does not show.

use gs_core::transform::Similarity;
use gs_core::{AttrValue, AttrVec, Domain, MeshStore, NO, NodeId, PosRead, VertId};

use crate::effect::Names;

pub(crate) struct Changed {
    /// Moved vertices, ascending.
    pub moved: Vec<VertId>,
    /// Touched vertices, ascending: those whose scene point moved, or an attribute value
    /// changed.
    pub touched: Vec<VertId>,
    /// An attribute column that no node held at the start is held now, or the other way.
    pub reshaped: bool,
    /// The attribute columns, by name, with a value changed at a live vertex or face, or that
    /// appeared or vanished.
    pub columns: Names,
    /// The name of an attribute column given a value that is not a finite number.
    pub not_finite: Option<String>,
}

impl Changed {
    pub fn any(&self) -> bool {
        self.reshaped || !self.touched.is_empty() || !self.moved.is_empty()
    }
}

/// Slot `i` of a node's column as raw bits; a column the node never held reads as its kind's
/// empty value (white for a color).
fn word(values: Option<&AttrVec>, i: usize, empty: u32) -> u32 {
    values.map_or(empty, |v| v.bits(i))
}

/// What the edit changed in `nodes` (ascending) of a part placed by `t`.
pub(crate) fn of(store: &MeshStore, nodes: &[NodeId], t: &Similarity) -> Changed {
    let start = store.start_pos();
    let seen = |p, q| t.point(p).bits() != t.point(q).bits();
    let mut shown = Vec::new();
    let columns: Vec<_> = (store.attrs().iter())
        .filter_map(|d| Some((store.attr_id(&d.name)?, d)))
        .collect();
    // Whether each column changed at a live slot.
    let mut hit = vec![false; columns.len()];
    let (mut moved, mut touched, mut not_finite) = (Vec::new(), Vec::new(), None);
    for &n in nodes {
        let node = store.node(n);
        // In a node whose topology changed, a slot names another point than at the start, so
        // every live point counts as moved (decision 0021, section 8).
        let reshuffled = store.start_topology_changed(n);
        for (v, p) in (node.base..).zip(node.pos) {
            let live = !p.is_dead();
            if reshuffled && live {
                moved.push(v);
                shown.push(v);
            } else if !reshuffled && p.bits() != start.get(v).bits() {
                moved.push(v);
                if seen(*p, start.get(v)) {
                    shown.push(v);
                }
            }
        }
        for (&(id, def), hit) in columns.iter().zip(&mut hit) {
            let (now, then) = (node.attr(id), store.start_attr(id, n));
            if now.or(then).is_none() {
                continue;
            }
            let slots = match def.domain {
                Domain::Vertex => node.pos.len(),
                Domain::Face => node.faces.len(),
            };
            let empty = AttrValue::zero(def.kind).bits();
            let differs = |i: usize| reshuffled || word(now, i, empty) != word(then, i, empty);
            for i in (0..slots).filter(|&i| differs(i)) {
                if let Some(AttrVec::F32(v)) = now
                    && !v[i].is_finite()
                {
                    not_finite = Some(def.name.clone());
                }
                match def.domain {
                    Domain::Vertex => {
                        touched.push(node.base + i as u32);
                        *hit |= !node.pos[i].is_dead();
                    }
                    Domain::Face => {
                        touched.extend(node.faces[i].into_iter().filter(|&u| u != NO));
                        *hit |= node.faces[i][0] != NO;
                    }
                }
            }
        }
    }
    touched.extend_from_slice(&shown);
    // A dead slot's value can change, but no vertex is there to touch.
    let pos = store.pos();
    touched.retain(|&v| !pos.get(v).is_dead());
    touched.sort_unstable();
    touched.dedup();
    let all = 0..store.node_count() as NodeId;
    let mut reshaped = false;
    for (&(id, _), hit) in columns.iter().zip(&mut hit) {
        let held = |n: NodeId| store.node(n).attr(id).is_some();
        let held_then = |n: NodeId| store.start_attr(id, n).is_some();
        if all.clone().any(held) != all.clone().any(held_then) {
            (reshaped, *hit) = (true, true);
        }
    }
    let named = columns.iter().zip(hit).filter(|(_, hit)| *hit);
    Changed {
        moved,
        touched,
        reshaped,
        columns: named.map(|((_, def), _)| def.name.clone()).collect(),
        not_finite,
    }
}
