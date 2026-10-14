//! The one cavity measure (decision 0056): how recessed the surface is around a point, read by
//! the cavity automask (`kernels/influence.rs`) and the cavity mask maker (`mask/make.rs`).
//!
//! It ports Blender's `calc_blurred_cavity_mesh` and `calc_cavity_factor`
//! (`editors/sculpt_paint/mesh/sculpt_automasking.cc`), with flat surface free instead of half
//! masked.

use std::collections::VecDeque;

use crate::config::VertId;
use crate::math::Vec3;
use crate::view::{PosRead, TopoRead, edge_neighbours};
use crate::visibility::Visible;

/// The walk's scratch: points reached (ascending), the queue and neighbours.
pub(crate) struct Walk {
    seen: Vec<VertId>,
    queue: VecDeque<(VertId, u32)>,
    nb: Vec<(VertId, u8)>,
}

impl Walk {
    pub(crate) const fn new() -> Walk {
        Walk {
            seen: Vec::new(),
            queue: VecDeque::new(),
            nb: Vec::new(),
        }
    }
}

/// The measure at `v`: a breadth-first walk over visible edges out to `rings` steps (at least
/// 1). `A` is the mean position of every point reached and `D` their mean distance from `v`;
/// `B` and `N` are the mean position and the normalized mean normal (`nrm`) of the points
/// closer than `rings` steps (`v`'s own normal if that sum is zero). The measure is
/// `dot(A - B, N) / D`: positive in a hollow, negative on a peak, 0 on a plane and when `D` is.
pub(crate) fn measure(
    (pos, topo, visible): (&impl PosRead, &impl TopoRead, Visible<'_>),
    nrm: impl Fn(VertId) -> Vec3,
    v: VertId,
    rings: u32,
    w: &mut Walk,
) -> f32 {
    let start = pos.get(v);
    let (mut all, mut dist, mut count) = (Vec3::ZERO, 0.0f32, 0u32);
    let (mut near, mut normal, mut near_count) = (Vec3::ZERO, Vec3::ZERO, 0u32);
    w.seen.clear();
    w.seen.push(v);
    w.queue.push_back((v, 0));
    while let Some((u, depth)) = w.queue.pop_front() {
        let at = pos.get(u);
        all += at;
        dist += (at - start).len2().sqrt();
        count += 1;
        if depth >= rings {
            continue;
        }
        near += at;
        normal += nrm(u);
        near_count += 1;
        edge_neighbours(topo, u, |f| visible.face(f), &mut w.nb);
        for &(x, _) in &w.nb {
            if let Err(i) = w.seen.binary_search(&x) {
                w.seen.insert(i, x);
                w.queue.push_back((x, depth + 1));
            }
        }
    }
    // `v` itself is reached at depth 0 < rings, so both counts are at least 1.
    let (a, b) = (all * (1.0 / count as f32), near * (1.0 / near_count as f32));
    let dist = dist / count as f32;
    let mut normal = normal.normalized();
    if normal.len2() == 0.0 {
        normal = nrm(v);
    }
    if dist == 0.0 {
        0.0
    } else {
        (a - b).dot(normal) / dist
    }
}

/// How much of full strength the measure `f` reaches at `contrast` (0 to 100):
/// `clamp(0.5 * contrast * f, 0, 1)`, so 0 on a plane or a peak and 1 in a hollow deep enough.
/// Contrast 50 gives Blender's default factor of 25.
pub(crate) fn depth(f: f32, contrast: f32) -> f32 {
    (0.5 * contrast * f).clamp(0.0, 1.0)
}
