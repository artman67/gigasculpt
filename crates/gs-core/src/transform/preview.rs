//! What the deformer preview on the graphics card takes from the CPU's own code (decision 0041,
//! amended 2026-10-12 for `DF9a`): each point's weight, weighed by the same functions the
//! transform runs, and the formula a point moves by, which the vertex stage's twin in gs-render
//! `shaders/deform.wgsl` matches.

use super::{RIGID, Ready, Transform, Weighted, Weights};
use crate::config::NodeId;
use crate::math::Vec3;
use crate::store::MeshStore;

impl MeshStore {
    /// Each vertex slot's weight under `t`, node by node and every slot of a node: what
    /// [`MeshStore::transform`] weighs the point (visible times (1 - mask) times the falloff,
    /// faded by the gradient), 0 for a hidden, dead or held one. A box or radius `t` leaves out
    /// is measured first, as the transform measures it. `None` when a Mirror or Parallel plane
    /// splits the part, which the transform runs once per region.
    pub fn weights(&self, t: &Transform) -> Option<Vec<f32>> {
        if t.splits(self) {
            return None;
        }
        let k = Weighted::new(self, t).measured(self);
        let nodes = gs_par::map(self.node_count(), |n| {
            let node = self.node(n as NodeId);
            let mask = k.mask_of(&node);
            (0..node.pos.len())
                .map(|l| (k.weigh(&node, mask, l)).map_or(0.0, |(v, _, w)| k.ready.ramp(v, w)))
                .collect::<Vec<f32>>()
        });
        Some(nodes.concat())
    }
}

impl Transform {
    /// Whether a Mirror or Parallel plane splits `store`'s points into regions for this
    /// transform, which runs once in each: `deformed` is the transform of one region.
    pub fn splits(&self, store: &MeshStore) -> bool {
        // Rigid: no plane depends on the falloff, and a soft one along the surface floods it.
        let weights = Weights {
            falloff: RIGID,
            ..self.weights
        };
        let rigid = Transform {
            weights,
            ..self.clone()
        };
        Weighted::new(store, &rigid).planes() != 0
    }

    /// Where the transform's deformer puts point `p` at weight `w`, as [`MeshStore::transform`]
    /// moves a point of that weight that no plane splits from the frame's origin; `p` itself
    /// for an operation that is not a deformer. The twin of `deformed` in gs-render
    /// `shaders/deform.wgsl`, one operation of its chain.
    pub fn deformed(&self, p: Vec3, w: f32) -> Vec3 {
        self.prepared().deformed(p, w)
    }

    /// The transform made ready once for many points (decision 0041, amended 2026-10-13 for
    /// `DF9b`): the cage's grid and the curve's arcs built once, not once a point.
    pub fn prepared(&self) -> Prepared {
        Prepared {
            ready: Ready::new(self),
            origin: self.frame.origin,
        }
    }
}

/// A transform made ready for many points (see [`Transform::prepared`]).
#[derive(Clone, Debug)]
pub struct Prepared {
    ready: Ready,
    origin: Vec3,
}

impl Prepared {
    /// [`Transform::deformed`], without making the transform ready again.
    pub fn deformed(&self, p: Vec3, w: f32) -> Vec3 {
        let d = self.ready.change(p - self.origin, w);
        if d == Vec3::ZERO { p } else { p + d }
    }

    /// What the vertex stage reads for the cage or the curve (gs-render
    /// `shaders/deform_tables.wgsl`): gs-core's own grid or arcs in `f32`, 16 bytes an entry.
    /// Empty for every other operation.
    pub fn table(&self) -> Vec<[f32; 4]> {
        self.ready.table()
    }
}
