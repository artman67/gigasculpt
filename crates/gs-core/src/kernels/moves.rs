//! Move and its Hook variant (Move, Move Connected, Move Elastic, Pull Out), and the walk along
//! edges that the connected selection and the topology automask share.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use super::Frame;
use crate::brush::{HookSpec, MoveSpec, Select};
use crate::config::VertId;
use crate::math::Vec3;
use crate::store::MeshStore;
use crate::view::{PosRead, edge_neighbours};

/// The elastic move's fade: `(1 - x^2)^2` inside `x < 1`, so the move ends smoothly at its
/// bounded support (ARCHITECTURE 4.3: brushes that touch everything get a bounded support).
pub(super) fn cutoff(x: f32) -> f32 {
    let u = (1.0 - x * x).max(0.0);
    u * u
}

/// Displacement at offset `rel` from the grab of a regularized Kelvinlet pulled by `t`, with
/// scale `eps` and Poisson ratio `nu`, scaled so the grab itself moves by exactly `t` (de Goes
/// and James, "Regularized Kelvinlets", 2017; the stiffness cancels in that scaling).
pub(super) fn kelvinlet(rel: Vec3, t: Vec3, eps: f32, nu: f32) -> Vec3 {
    let b = 1.0 / (4.0 * (1.0 - nu));
    let e2 = eps * eps;
    let r2 = rel.len2() + e2;
    let r = r2.sqrt();
    let r3 = r2 * r;
    let along = (1.0 - b) / r + e2 / (2.0 * r3);
    let across = b / r3 * rel.dot(t);
    (t * along + rel * across) * (eps / (1.5 - b))
}

/// Move: the vertex's offset from its stroke-start position (the stroke reverted the mesh to
/// it before this update).
pub(super) fn drag(k: &MoveSpec, f: &Frame, p: Vec3, w: f32) -> Vec3 {
    let field = match k.select {
        Select::Elastic => kelvinlet(p - f.center, f.delta, f.radius, k.poisson),
        Select::Ball | Select::Connected => f.delta,
    };
    field * (w * f.strength)
}

/// Hook: the step from the previous dab, plus a pull toward the line of motion. Without a step
/// (the press) there is no line, so nothing moves.
pub(super) fn hook(k: HookSpec, f: &Frame, p: Vec3, w: f32) -> Vec3 {
    if f.delta.len2() == 0.0 {
        return Vec3::ZERO;
    }
    let dir = f.delta.normalized();
    let to = f.center - p;
    let across = to - dir * dir.dot(to);
    (f.delta + across * k.pinch) * (w * f.strength)
}

/// Every vertex within `limit` of `from` walking along edges of visible faces, ascending, with
/// its distance. Bounded by the limit, so it costs the brush footprint, not the mesh (the stroke
/// reports the count as a work counter).
pub(crate) fn flood(store: &MeshStore, from: VertId, limit: f32) -> Vec<(VertId, f32)> {
    let (pos, topo, visible) = (store.pos(), store.topo(), store.visible());
    let mut best: BTreeMap<VertId, f32> = BTreeMap::from([(from, 0.0)]);
    let mut done = BTreeSet::new();
    let mut queue = BinaryHeap::from([Reverse((0u32, from))]);
    let mut nb = Vec::new();
    // Distances are never negative, so their bit patterns sort like the numbers. Each vertex
    // is finished the first time it comes off the queue, at its shortest distance.
    while let Some(Reverse((bits, v))) = queue.pop() {
        if !done.insert(v) {
            continue;
        }
        let d = f32::from_bits(bits);
        edge_neighbours(&topo, v, |f| visible.face(f), &mut nb);
        for &(u, _) in &nb {
            let du = d + (pos.get(u) - pos.get(v)).len2().sqrt();
            let old = best.get(&u).copied().unwrap_or(f32::INFINITY);
            if du <= limit && du.min(old).to_bits() != old.to_bits() {
                best.insert(u, du);
                queue.push(Reverse((du.to_bits(), u)));
            }
        }
    }
    best.into_iter().collect()
}
