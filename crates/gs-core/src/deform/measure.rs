//! The box and the spherize radius a transform leaves to be measured: the box around every
//! point that weighs more than zero, in frame coordinates, and the mean distance of those
//! points from the origin over the kept axes. Each node is measured on its own and the nodes
//! are summed in node order, so the result is the same on any number of threads.

use super::{Deform, Ready};
use crate::config::NodeId;
use crate::math::{Aabb, Vec3};
use crate::store::MeshStore;
use crate::transform::{Op, Transform, Weighted};

impl MeshStore {
    /// `t` with the box and spherize radius it leaves out, where its operation or gradient
    /// needs them, measured on these points: what a log line holds so replay measures nothing.
    pub fn measured(&self, t: &Transform) -> Transform {
        if wants(t) == (false, false) {
            return t.clone();
        }
        Weighted::new(self, t).measured(self).t
    }
}

/// Whether `t` leaves out a box its operation or gradient needs, and a spherize radius.
fn wants(t: &Transform) -> (bool, bool) {
    let boxed = matches!(t.op, Op::Deform(ref d) if d.needs_box());
    let scope = t.scope;
    (
        scope.bounds.is_none() && (boxed || scope.gradient.is_some()),
        matches!(t.op, Op::Deform(Deform::Spherize { radius: None, .. })),
    )
}

/// One node's share: the box around its points, their distances summed, and how many.
#[derive(Clone, Copy)]
struct Share {
    lo: [f64; 3],
    hi: [f64; 3],
    sum: f64,
    count: u64,
}

impl Share {
    const NONE: Share = Share {
        lo: [f64::INFINITY; 3],
        hi: [f64::NEG_INFINITY; 3],
        sum: 0.0,
        count: 0,
    };

    fn then(self, o: Share) -> Share {
        Share {
            lo: [0, 1, 2].map(|k| self.lo[k].min(o.lo[k])),
            hi: [0, 1, 2].map(|k| self.hi[k].max(o.hi[k])),
            sum: self.sum + o.sum,
            count: self.count + o.count,
        }
    }
}

impl Weighted {
    /// The kernel with its box and spherize radius measured where the transform leaves them
    /// out and an operation or the gradient needs them. With no point weighing, the box is
    /// the origin alone and the radius 0.
    pub(crate) fn measured(mut self, store: &MeshStore) -> Weighted {
        let (wants_box, wants_radius) = wants(&self.t);
        if !wants_box && !wants_radius {
            return self;
        }
        let shares = gs_par::map(store.node_count(), |n| self.share(store, n as NodeId));
        let all = shares.into_iter().fold(Share::NONE, Share::then);
        let some = all.count > 0;
        if wants_box {
            let at = |c: [f64; 3]| {
                let f = |k: usize| if some { c[k] as f32 + 0.0 } else { 0.0 };
                Vec3::new(f(0), f(1), f(2))
            };
            self.t.scope.bounds = Some(Aabb {
                lo: at(all.lo),
                hi: at(all.hi),
            });
        }
        if wants_radius && let Op::Deform(Deform::Spherize { radius, .. }) = &mut self.t.op {
            *radius = Some(if some {
                (all.sum / all.count as f64) as f32
            } else {
                0.0
            });
        }
        self.ready = Ready::new(&self.t);
        self
    }

    /// Node `n`'s share, its points in slot order.
    fn share(&self, store: &MeshStore, n: NodeId) -> Share {
        let node = store.node(n);
        let mask = self.mask_of(&node);
        let mut s = Share::NONE;
        for l in 0..node.pos.len() {
            let Some((v, ..)) = self.weigh(&node, mask, l) else {
                continue;
            };
            let c = self.ready.coords(v);
            for (k, &x) in c.iter().enumerate() {
                s.lo[k] = s.lo[k].min(x);
                s.hi[k] = s.hi[k].max(x);
            }
            let kept = self.ready.held(c);
            s.sum += (kept[0] * kept[0] + kept[1] * kept[1] + kept[2] * kept[2]).sqrt();
            s.count += 1;
        }
        s
    }
}
