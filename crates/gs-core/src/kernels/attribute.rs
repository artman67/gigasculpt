//! Attribute: write mask, a UV hint, face group or hide (Mask Pen, UV Hint, UV Density). Two
//! phases like every kernel: new column values per node from a read-only view, then one write
//! guard over the nodes that changed, which records undo. A point's value is settled in its
//! column's range; Alt paints toward 0, which for a UV hint is from either side. Select Rect's
//! gesture writes hide through `stroke::gesture`. Color is painted by a preset's `[paint]` table
//! (`kernels::paint`), and refused here at load.

use super::{Ctx, Frame};
use crate::brush::{AttrOp, AttrSpec, Column};
use crate::columns::{AttrDef, AttrId, AttrKind, AttrValue, AttrVec};
use crate::config::NodeId;
use crate::kernel::{Reach, SculptSurface};
use crate::mask::{MaskFilter, mean_around};
use crate::math::{NO, Vec3, face_center};
use crate::par::merge;
use crate::store::MeshStore;
use crate::view::{NodeView, PosRead};
use crate::write::NewValues;

/// One dab of an attribute brush over all its passes, into the column `def` (registered with
/// its first value). Returns the nodes written.
pub(crate) fn paint(
    store: &mut MeshStore,
    ctx: &Ctx<'_>,
    frames: &[Frame],
    k: &AttrSpec,
    def: &AttrDef,
) -> Vec<NodeId> {
    let spheres: Vec<(Vec3, f32)> = frames.iter().map(|f| (f.center, f.radius)).collect();
    let q = store.query_all(&spheres);
    let faces = matches!(k.column, Column::Group | Column::Hide);
    let (nodes, reach) = match k.column {
        // Face values can sit in shell nodes: a face is stored with its lowest vertex.
        Column::Group => (merge(&q.brushed, &q.shell), Reach::Faces),
        Column::Hide => (merge(&q.brushed, &q.shell), Reach::Everything),
        Column::Mask | Column::UvHint | Column::UvDensity | Column::Color => {
            (q.brushed, ctx.node_reach())
        }
    };
    let (nodes, skipped) = store.reachable(&nodes, reach);
    store.work.skipped += skipped.len() as u64;
    let col = store.attr_fits(def).ok().flatten();
    let s = store.surface();
    let fresh = gs_par::map(nodes.len(), |i| {
        let node = s.node(nodes[i]);
        if faces {
            face_values(ctx, s, &node, frames, k, col, def.kind)
        } else {
            vertex_values(ctx, s, &node, frames, k, col, def)
        }
    });
    store.put_column(def.clone(), &nodes, fresh)
}

/// `new` as a write: unchanged, or new values.
fn changed(old: &AttrVec, new: AttrVec) -> NewValues {
    if new == *old {
        NewValues::Same
    } else {
        NewValues::Values(new)
    }
}

fn vertex_values(
    ctx: &Ctx<'_>,
    s: SculptSurface<'_>,
    node: &NodeView<'_>,
    frames: &[Frame],
    k: &AttrSpec,
    col: Option<AttrId>,
    def: &AttrDef,
) -> NewValues {
    let old = col
        .and_then(|c| node.attr(c))
        .cloned()
        .unwrap_or_else(|| AttrVec::zeros(def.kind, node.pos.len()));
    let mut new = old.clone();
    let value = if ctx.invert { 0.0 } else { k.value };
    let range = def.range();
    let (mut ws, mut nb) = (vec![0.0; frames.len()], Vec::new());
    let plan = ctx.plan(node, frames);
    for l in 0..node.pos.len() {
        let Some(m) = ctx.influence(s, node, &plan, l, frames, &mut ws) else {
            continue;
        };
        let w: f32 = ws.iter().sum::<f32>() * m;
        let frac = 1.0 - libm::expf(-k.gain * w * frames[0].amount);
        match &mut new {
            AttrVec::F32(v) => {
                let around = node.base + l as u32;
                let mut mean =
                    || (col.and_then(|c| mean_around(s, c, around, &mut nb))).unwrap_or(v[l]);
                let target = match k.op {
                    AttrOp::Set => value as f32,
                    AttrOp::Blur => MaskFilter::Blur.target(v[l], mean()),
                    AttrOp::Sharpen => MaskFilter::Sharpen.target(v[l], mean()),
                };
                let x = v[l] + (target - v[l]) * frac;
                v[l] = range.map_or(x, |r| r.settle(x));
            }
            AttrVec::U32(_) | AttrVec::Bits(_) => {}
        }
    }
    changed(&old, new)
}

fn face_values(
    ctx: &Ctx<'_>,
    s: SculptSurface<'_>,
    node: &NodeView<'_>,
    frames: &[Frame],
    k: &AttrSpec,
    col: Option<AttrId>,
    kind: AttrKind,
) -> NewValues {
    let old = col
        .and_then(|c| node.attr(c))
        .cloned()
        .unwrap_or_else(|| AttrVec::zeros(kind, node.faces.len()));
    let mut new = old.clone();
    let value = if ctx.invert { 0 } else { k.value as u32 };
    let (pos, visible) = (s.pos(), s.visible());
    let feather = ctx.feather(frames);
    for (j, fc) in node.faces.iter().enumerate() {
        let shown = visible.face(node.first_face + j as u32);
        if fc[0] == NO || (k.column != Column::Hide && !shown) {
            continue;
        }
        let c = face_center(*fc, |u| pos.get(u));
        if feather * frames.iter().map(|f| ctx.shape(f, c)).sum::<f32>() >= 0.5 {
            new.set(j, AttrValue::U32(value));
        }
    }
    changed(&old, new)
}
