//! The density picker's measure (decision 0021, L4; `live_detail.pick`): how long the edges
//! around a point are, and the target that makes new edges about as long.

use crate::config::VertId;
use crate::math::arity;
use crate::store::MeshStore;
use crate::view::{PosRead, TopoRead};

/// How long a pass leaves edges on average, as a fraction of its target: the mean of the merge
/// length 0.4 and the target, which Blender's detail sampler divides by (research 17, section
/// 6; `sculpt_detail.cc` 645-656, GPL).
pub const TYPICAL_EDGE: f32 = 0.7;

/// The mean length of the edges of live point `v`, and the target that makes edges about that
/// long (the mean over [`TYPICAL_EDGE`]), in the store's units. Each edge counts once, its
/// length summed in the order of its far end. `None` for a point on no face.
pub fn typical(store: &MeshStore, v: VertId) -> Option<(f32, f32)> {
    let (cfg, topo, pos) = (store.config(), store.topo(), store.pos());
    let node = store.node(cfg.node_of(v));
    let mut ends: Vec<VertId> = (node.ring(cfg.local_of(v) as usize).iter())
        .flat_map(|&f| {
            let fc = topo.face(f);
            let k = arity(fc);
            let i = (0..k).find(|&i| fc[i] == v).unwrap_or_default();
            [fc[(i + 1) % k], fc[(i + k - 1) % k]]
        })
        .collect();
    ends.sort_unstable();
    ends.dedup();
    if ends.is_empty() {
        return None;
    }
    let p = pos.get(v);
    let sum: f32 = ends.iter().map(|&u| (pos.get(u) - p).len2().sqrt()).sum();
    let mean = sum / ends.len() as f32;
    Some((mean, mean / TYPICAL_EDGE))
}
