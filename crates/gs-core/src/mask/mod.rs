//! The mask's rules, in one home (research 13, 1.5 and 8.4; decisions 0015 and 0026): how every
//! writer stores a value, the neighbour filters and their targets (which the Mask Pen's blur and
//! sharpen read too), the point maps, the mask makers and their combine step (`make`), and the
//! whole-part edits behind the `mask.*` commands.
//!
//! The mask is one `f32` per point, 0 free and 1 fully masked. Every writer stores
//! [`settle`]d values: clamped to 0..1 and on a grid of 2^-24, the spacing of `f32` just below
//! 1. On that grid `1 - m` is exact, so inverting twice gives back the same bits.
//!
//! Whole-part edits act on visible points (Blender's rule; masking everything on a partly hidden
//! model leaves the hidden points alone) unless their scope says every point. Neighbour filters
//! read the mask across visible faces only, as passes over a read-only view, so the thread
//! count cannot change a bit.

mod make;

pub use make::{Combine, MaskMaker, NewMask};

use crate::columns::{AttrDef, AttrId, AttrVec, Range};
use crate::config::{NodeId, VertId};
use crate::kernel::SculptSurface;
use crate::store::MeshStore;
use crate::view::{NodeView, edge_neighbours};
use crate::write::NewValues;

/// A mask value as every writer stores it: clamped to 0..1 on the 2^-24 grid ([`Range::MASK`]).
/// Not a number stores as 1, fully masked, the way the influence stage reads it.
pub fn settle(x: f32) -> f32 {
    Range::MASK.settle(x)
}

/// A neighbour filter: each point moves toward a target made from its own value `m` and the
/// mean of its neighbours across visible faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskFilter {
    /// Toward the mean. A command pass goes half way (Blender's rule).
    Blur,
    /// Away from the mean: toward `2m - mean`, clamped (unsharp masking; research 13, 1.3).
    Sharpen,
    /// Toward `max(m, mean)`: the mask spreads with a soft edge, and nothing masked shrinks.
    Grow,
    /// Toward `min(m, mean)`: the edge pulls in softly, and nothing free gains mask.
    Shrink,
}

impl MaskFilter {
    /// Where the filter moves a point at mask `m` whose neighbours average `mean`.
    pub fn target(self, m: f32, mean: f32) -> f32 {
        match self {
            MaskFilter::Blur => mean,
            MaskFilter::Sharpen => (2.0 * m - mean).clamp(0.0, 1.0),
            MaskFilter::Grow => m.max(mean),
            MaskFilter::Shrink => m.min(mean),
        }
    }

    /// How far one command pass at full strength moves toward the target.
    pub(crate) fn step(self) -> f32 {
        match self {
            MaskFilter::Blur => 0.5,
            _ => 1.0,
        }
    }
}

/// A point map: each visible point moves toward a target made from its own value alone
/// (decision 0026, 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointMap {
    /// Toward `sqrt(m)`: values between 0 and 1 rise, 0 and 1 stay.
    Boost,
    /// Toward `m * m`: values between 0 and 1 fall, 0 and 1 stay.
    Dilute,
    /// Toward `(m - lo) / (hi - lo)`, with `lo` and `hi` the lowest and highest mask of the
    /// visible points: the mask's range stretched to 0..1. A flat mask stays.
    Normalize,
}

/// Which points a whole-part mask edit writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Visible points only; hidden ones keep their mask.
    #[default]
    Visible,
    /// Every point.
    All,
}

/// The mean mask of the neighbours of `v` across visible faces, or `None` if it has none.
/// `nb` is scratch.
pub(crate) fn mean_around(
    s: SculptSurface<'_>,
    mask: AttrId,
    v: VertId,
    nb: &mut Vec<(VertId, u8)>,
) -> Option<f32> {
    let visible = s.visible();
    edge_neighbours(&s.topo(), v, |f| visible.face(f), nb);
    let reader = s.attr_reader(mask);
    let sum: f32 = nb.iter().map(|e| reader.scalar(e.0)).sum();
    (!nb.is_empty()).then(|| sum / nb.len() as f32)
}

/// A node's values of the mask as a [`MeshStore::put_attr`] write: released when they are all
/// zero (also a column that already held only zeros), unchanged, or new values.
fn written(old: Option<&[f32]>, new: Vec<f32>) -> NewValues {
    if new.iter().all(|x| x.to_bits() == 0) {
        return if old.is_some() {
            NewValues::Zeros
        } else {
            NewValues::Same
        };
    }
    match old {
        Some(o) if o.iter().zip(&new).all(|(a, b)| a.to_bits() == b.to_bits()) => NewValues::Same,
        _ => NewValues::Values(AttrVec::F32(new)),
    }
}

/// A node's mask values, if it holds any.
fn held<'a>(node: &NodeView<'a>, mask: AttrId) -> Option<&'a [f32]> {
    match node.attr(mask) {
        Some(AttrVec::F32(v)) => Some(v),
        _ => None,
    }
}

impl MeshStore {
    /// Set the mask of every point in `scope` to `value` (clamped to 0..1). Setting 0 where
    /// nothing else in a node is masked releases its column. Returns the nodes written.
    pub fn mask_set(&mut self, value: f32, scope: Scope) -> Vec<NodeId> {
        let value = settle(value);
        self.map_mask(scope, |_, _| value)
    }

    /// `1 - m` at every point in `scope`. Returns the nodes written.
    pub fn mask_invert(&mut self, scope: Scope) -> Vec<NodeId> {
        self.map_mask(scope, |_, m| 1.0 - m)
    }

    /// Move the mask of every visible point `strength` of the way (0 to 1) to the point map's
    /// target. Returns the nodes written.
    pub fn mask_map(&mut self, map: PointMap, strength: f32) -> Vec<NodeId> {
        let (lo, hi) = match map {
            PointMap::Normalize => match self.visible_mask_range() {
                Some((lo, hi)) if hi > lo => (lo, hi),
                _ => return Vec::new(),
            },
            _ => (0.0, 1.0),
        };
        let target = |m: f32| match map {
            PointMap::Boost => m.sqrt(),
            PointMap::Dilute => m * m,
            PointMap::Normalize => (m - lo) / (hi - lo),
        };
        self.map_mask(Scope::Visible, |_, m| m + (target(m) - m) * strength)
    }

    /// The lowest and highest mask of the live visible points, if there are any.
    fn visible_mask_range(&self) -> Option<(f32, f32)> {
        let mask = self.attr_id(&AttrDef::mask().name);
        let per_node = gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            let old = mask.and_then(|m| held(&node, m));
            (0..node.pos.len())
                .filter(|&l| !node.pos[l].is_dead() && !node.hidden(l))
                .map(|l| old.map_or(0.0, |o| o[l]))
                .fold(None, |r: Option<(f32, f32)>, m| {
                    Some(r.map_or((m, m), |(lo, hi)| (lo.min(m), hi.max(m))))
                })
        });
        (per_node.into_iter().flatten()).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    /// Apply `f` to the mask of every live point in `scope` (given the point and its mask), as
    /// one write. A part with no mask column gets one with its first non-zero value
    /// ([`MeshStore::put_column`]).
    fn map_mask(&mut self, scope: Scope, f: impl Fn(VertId, f32) -> f32 + Sync) -> Vec<NodeId> {
        let mask = self.attr_fits(&AttrDef::mask()).ok().flatten();
        let nodes: Vec<NodeId> = (0..self.node_count() as NodeId).collect();
        let fresh = gs_par::map(nodes.len(), |i| {
            let node = self.node(nodes[i]);
            let old = mask.and_then(|m| held(&node, m));
            let new = (0..node.pos.len())
                .map(|l| {
                    let m = old.map_or(0.0, |o| o[l]);
                    let keep = node.pos[l].is_dead() || (scope == Scope::Visible && node.hidden(l));
                    if keep {
                        m
                    } else {
                        settle(f(node.base + l as u32, m))
                    }
                })
                .collect();
            written(old, new)
        });
        self.put_column(AttrDef::mask(), &nodes, fresh)
    }

    /// One pass of `filter` over the visible points at `strength` (0 to 1: how far toward the
    /// target, 1 being a full pass). Hidden points keep their mask. Returns the nodes written.
    pub fn mask_filter(&mut self, filter: MaskFilter, strength: f32) -> Vec<NodeId> {
        let Some(mask) = self.attr_id(&AttrDef::mask().name) else {
            return Vec::new();
        };
        let amount = filter.step() * strength;
        let nodes: Vec<NodeId> = (0..self.node_count() as NodeId).collect();
        let s = self.surface();
        let fresh = gs_par::map(nodes.len(), |i| {
            let node = s.node(nodes[i]);
            let old = held(&node, mask);
            let mut new: Vec<f32> = old.map_or_else(|| vec![0.0; node.pos.len()], <[f32]>::to_vec);
            // A node holding no mask stays clear inside: only its seam can see a masked
            // neighbour in another node.
            let slots: Box<dyn Iterator<Item = usize>> = match old {
                Some(_) => Box::new(0..node.pos.len()),
                None => Box::new(node.seam.iter().map(|&l| l as usize)),
            };
            let mut nb = Vec::new();
            for l in slots {
                if node.pos[l].is_dead() || node.hidden(l) {
                    continue;
                }
                let m = new[l];
                let Some(mean) = mean_around(s, mask, node.base + l as u32, &mut nb) else {
                    continue;
                };
                new[l] = settle(m + (filter.target(m, mean) - m) * amount);
            }
            written(old, new)
        });
        self.put_attr(mask, &nodes, fresh)
    }
}
