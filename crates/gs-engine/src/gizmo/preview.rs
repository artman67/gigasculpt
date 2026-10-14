//! The open deformer's preview on the graphics card (decision 0041, amended 2026-10-12 for
//! `DF9a`, and 2026-10-13 for `DF9b`, which adds noise, the cage and the curve to DF2's closed
//! forms). While such a deformer is open, its updates leave the part's points
//! as they were at the open, with no undo step open, and hand the window the operations and
//! each point's weight instead ([`DeformPreview`], read by `CallReport::preview`): the window's
//! vertex stage moves the points. Accept is today's, the CPU result once. [`on_card`] is the
//! one switch; everything it refuses takes today's path, the points moved on the CPU at every
//! update.
//!
//! A call that reads the part's points, other than the window's own traffic, catches up first
//! ([`Engine::catch_up`]): the CPU result goes into the store as today's update leaves it, and
//! the next update puts the points back and previews again.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gs_core::Vec3;
use gs_core::deform::Deform;
use gs_core::transform::{Op, RIGID, Transform};

use super::Gizmos;
use super::deformer::Deformer;
use crate::command::CommandSpec;
use crate::document::Part;
use crate::engine::Engine;

/// The operations the vertex stage runs: a chain of up to four (gs-render `deform.rs`).
const CHAIN: usize = 4;

/// The most cells noise may move a point and still preview on the card: past it the vertex
/// stage's `f32` place drifts toward the tolerance (section 1 of the `DF9b` amendment).
const NOISE_CELLS: f32 = 10.0;

/// The window's own calls, which knowingly see the points of the open (section 2 of the
/// amendment); with `ui.*`, no call of these catches up.
const WINDOW: [&str; 3] = ["gizmo.deformer", "pen.input", "session.state"];

/// A stamp for each weighing, never repeated in a process.
static WEIGHED: AtomicU64 = AtomicU64::new(1);

/// What the window draws while a deformer is open on the card: the part, as its points were at
/// the open, moved by `ops` at each point's weight.
#[derive(Clone, Debug)]
pub struct DeformPreview {
    /// The part's stable id.
    pub part: u64,
    /// The deformer's transforms in the part's own space, as Accept would run them; empty
    /// after Reset.
    pub ops: Vec<Transform>,
    /// Each vertex slot's weight, node by node and every slot of a node (gs-core
    /// `MeshStore::weights`), shared, not copied.
    pub weights: Arc<Vec<f32>>,
    /// Changes when the weights do.
    pub weighed: u64,
}

/// An open deformer on the card: what the window draws and what its weights were weighed under.
#[derive(Clone)]
pub(crate) struct Card {
    /// The first transform with its operation left out: the frame, weights, box and gradient.
    key: Transform,
    shown: DeformPreview,
    /// The store holds the CPU result (a catch-up): nothing shown until the next update.
    caught: bool,
    /// The next apply is a catch-up's, on the CPU.
    cpu: bool,
}

impl Gizmos {
    /// What the window draws for the open deformer, while it is on the card.
    pub(crate) fn preview(&self) -> Option<&DeformPreview> {
        let card = self.deformer.as_ref()?.card.as_ref()?;
        (!card.caught).then_some(&card.shown)
    }
}

/// True when `ops`, the deformer's settled transforms on `part`, preview on the card: one to
/// four deformers, each with its box and spherize radius, noise moving at most
/// [`NOISE_CELLS`] cells, on a part without detail levels, no Mirror or Parallel plane
/// splitting any; and for more than one, each rigid, without a gradient and with the same mask
/// switch, so one weight serves the chain (section 1; amended 2026-10-13 for `DF9b`).
pub(crate) fn on_card(ops: &[Transform], part: &Part) -> bool {
    let closed = |t: &Transform| match &t.op {
        Op::Deform(Deform::Spherize { radius, .. }) => radius.is_some(),
        Op::Deform(Deform::Noise { amount, scale, .. }) => amount.abs() <= NOISE_CELLS * scale,
        Op::Deform(d) => !d.needs_box() || t.scope.bounds.is_some(),
        _ => false,
    };
    let one = |t: &Transform| {
        let first = &ops[0].weights;
        let rigid = t.weights.falloff <= RIGID && first.falloff <= RIGID;
        rigid && t.scope.gradient.is_none() && t.weights.mask == first.mask
    };
    (1..=CHAIN).contains(&ops.len())
        && part.levels().is_none()
        && ops.iter().all(|t| closed(t) && !t.splits(part.store()))
        && (ops.len() == 1 || ops.iter().all(one))
}

impl Engine {
    /// Puts `d` on the card when [`on_card`] says so: its part's points back as they were at
    /// the open, no undo step open, its weights weighed again when what they depend on changed,
    /// and what the window draws set. False, and the card dropped, for today's path; a
    /// catch-up's apply is also today's.
    pub(super) fn preview(&mut self, d: &mut Deformer) -> bool {
        let catching = (d.card.as_mut()).is_some_and(|c| std::mem::take(&mut c.cpu));
        let Some(found) = self.state.doc.get_mut(&d.part) else {
            return false;
        };
        let from = d.start.transform;
        let ops: Vec<Transform> = (d.transforms.iter_mut())
            .filter_map(|a| {
                a.settle(found.store(), from, || found.world_diagonal())
                    .ok()
            })
            .collect();
        if catching || !on_card(&ops, found) {
            match &mut d.card {
                Some(card) if catching => card.caught = true,
                _ => d.card = None,
            }
            return false;
        }
        if d.points {
            found.store_mut().cancel_step();
            d.points = false;
        }
        let mut props = d.start;
        found.swap_props(&mut props);
        let key = Transform {
            op: Op::Move(Vec3::ZERO),
            ..ops[0].clone()
        };
        let kept = (d.card.take().filter(|c| c.key == key)).map(|c| c.shown);
        let weighed = kept.map(|s| (s.weights, s.weighed)).or_else(|| {
            let weights = found.store().weights(&ops[0])?;
            Some((Arc::new(weights), WEIGHED.fetch_add(1, Ordering::Relaxed)))
        });
        let Some((weights, weighed)) = weighed else {
            return false;
        };
        let part = found.id();
        let shown = DeformPreview {
            part,
            ops,
            weights,
            weighed,
        };
        d.card = Some(Card {
            key,
            shown,
            caught: false,
            cpu: false,
        });
        true
    }

    /// Before a call of `specs` that reads the part's points, other than the window's own
    /// traffic: the open deformer's CPU result into the store, as today's update leaves it,
    /// while the card shows one that moves the points.
    pub(crate) fn catch_up(&mut self, specs: &[&CommandSpec]) {
        let own = |s: &&CommandSpec| WINDOW.contains(&s.name) || s.name.starts_with("ui.");
        if specs.iter().all(own) {
            return;
        }
        let Some(mut d) = self.state.gizmos.deformer.take() else {
            return;
        };
        if let Some(card) = (d.card.as_mut()).filter(|c| d.changed && !c.caught) {
            card.cpu = true;
            self.reapply(&mut d);
        }
        self.state.gizmos.deformer = Some(d);
    }
}

impl Deformer {
    /// Reset on the card: no operations, the weights kept.
    pub(super) fn rest(&mut self) {
        if let Some(card) = &mut self.card {
            card.shown.ops.clear();
        }
    }
}
