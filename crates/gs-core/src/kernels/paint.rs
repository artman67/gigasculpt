//! The paint stroke (decision 0039, section 6). A stroke mixes color in linear light into a float
//! buffer it holds for each node it touched: per point, the color at the stroke's start and the
//! color its dabs mixed so far. Each dab rounds the buffer into the color column, so the view
//! and undo work as for any column, and never reads that rounding back. When the stroke ends,
//! every point it moved is rounded once more, with a dither from the stroke's own random stream
//! ([`CommandRng`], decision 0018): soft strokes do not stall in 8 bits (research 24, wrong
//! result 2), and the result is the same on any thread count and replays exactly.
//!
//! The buffer keeps the mix itself, not a coverage and one target: each dab mixes toward its own
//! target, so a target that changes within the stroke (Alt, Gradient) mixes as Blender's
//! premultiplied color buffer does.
//!
//! Smooth paints by [`blend`] instead (section 8): each dab reads its neighbours' colors as they
//! are, so it holds no buffer and dithers every dab.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::{Ctx, Frame};
use crate::columns::{AttrDef, AttrValue, AttrVec, WHITE, linear_rgb, linear_to_srgb};
use crate::config::{NodeId, VertId};
use crate::math::Vec3;
use crate::rng::CommandRng;
use crate::store::MeshStore;
use crate::stroke::PaintColors;
use crate::view::{NodeView, edge_neighbours};
use crate::write::NewValues;

type Rgb = [f32; 3];

/// One node's points in a paint stroke, by slot in the node.
struct NodePaint {
    /// Each point's color at the stroke's start, linear.
    start: Vec<Rgb>,
    /// The color the stroke's dabs mixed so far, linear: `start` until a dab touches it.
    mixed: Vec<Rgb>,
}

impl NodePaint {
    fn new(bytes: &[u32]) -> NodePaint {
        let start: Vec<Rgb> = bytes.iter().map(|&c| linear_rgb(c)).collect();
        NodePaint {
            mixed: start.clone(),
            start,
        }
    }
}

/// How a dab paints.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paint {
    pub gain: f32,
    /// The color painted toward, linear: under Gradient, at the brush's centre.
    main: Rgb,
    /// Under Gradient, the color at the brush's rim, linear.
    rim: Option<Rgb>,
}

impl Paint {
    /// A dab of a stroke painting `colors`, with Alt held or not: Alt swaps Main and Secondary
    /// (decision 0039, section 7) while Alternate is on (section 11).
    pub fn of(gain: f32, colors: PaintColors, alt: bool) -> Paint {
        let (main, other) = if alt && colors.alternate {
            (colors.secondary, colors.main)
        } else {
            (colors.main, colors.secondary)
        };
        Paint {
            gain,
            main: linear_rgb(main),
            rim: colors.gradient.then(|| linear_rgb(other)),
        }
    }

    /// The color painted toward at a point whose falloff weight is `fall`: Main, or under
    /// Gradient the rim's color mixed toward Main by the weight, in linear light.
    fn target(&self, fall: f32) -> Rgb {
        match self.rim {
            None => self.main,
            Some(rim) => {
                let t = fall.clamp(0.0, 1.0);
                std::array::from_fn(|k| rim[k] + (self.main[k] - rim[k]) * t)
            }
        }
    }
}

/// A paint stroke's buffer: the nodes it touched.
pub(crate) struct PaintBuffer {
    nodes: BTreeMap<NodeId, NodePaint>,
    /// Color Strength: how far toward the dabs' mix the stroke goes, 0 to 1.
    strength: f32,
}

/// One node's work in a dab: its buffer going in and out, and what it writes.
struct Job {
    node: NodeId,
    buf: Option<NodePaint>,
    out: NewValues,
}

impl PaintBuffer {
    /// An empty buffer for a stroke of Color Strength `strength`.
    pub fn new(strength: f32) -> PaintBuffer {
        PaintBuffer {
            nodes: BTreeMap::new(),
            strength,
        }
    }

    /// Forget everything: the colors went back to the stroke's start (a drag's update).
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Forget `nodes`, whose points changed (Live Detail refined them): their next dab starts
    /// again from their colors as they are.
    pub fn forget(&mut self, nodes: &[NodeId]) {
        for n in nodes {
            self.nodes.remove(n);
        }
    }

    /// One dab over all its passes. Returns the nodes written.
    pub fn dab(
        &mut self,
        store: &mut MeshStore,
        ctx: &Ctx<'_>,
        frames: &[Frame],
        paint: &Paint,
    ) -> Vec<NodeId> {
        let spheres: Vec<(Vec3, f32)> = frames.iter().map(|f| (f.center, f.radius)).collect();
        let q = store.query_all(&spheres);
        let (nodes, skipped) = store.reachable(&q.brushed, ctx.node_reach());
        store.work.skipped += skipped.len() as u64;
        let def = AttrDef::color();
        let col = store.attr_fits(&def).ok().flatten();
        let mut jobs: Vec<Job> = (nodes.iter())
            .map(|&node| Job {
                node,
                buf: self.nodes.remove(&node),
                out: NewValues::Same,
            })
            .collect();
        let (s, strength) = (store.surface(), self.strength);
        gs_par::apply_each(&mut jobs, |_, job| {
            let node = s.node(job.node);
            let bytes = colors(&node, col);
            let mut buf = (job.buf.take())
                .filter(|b| b.start.len() == bytes.len())
                .unwrap_or_else(|| NodePaint::new(&bytes));
            let mut new = bytes.to_vec();
            let (mut ws, plan) = (vec![0.0; frames.len()], ctx.plan(&node, frames));
            for (l, c) in new.iter_mut().enumerate() {
                let Some(m) = ctx.influence(s, &node, &plan, l, frames, &mut ws) else {
                    continue;
                };
                let fall: f32 = ws.iter().sum();
                let w = fall * m;
                let frac = 1.0 - libm::expf(-paint.gain * w * frames[0].amount);
                let mixed = &mut buf.mixed[l];
                for (x, t) in mixed.iter_mut().zip(paint.target(fall)) {
                    *x += (t - *x) * frac;
                }
                *c = pack(seen(buf.start[l], *mixed, strength), 0.5);
            }
            if new != *bytes {
                job.out = NewValues::Values(AttrVec::U32(new));
            }
            job.buf = Some(buf);
        });
        let fresh = jobs
            .into_iter()
            .map(|job| {
                if let Some(buf) = job.buf {
                    self.nodes.insert(job.node, buf);
                }
                job.out
            })
            .collect();
        store.put_column(def, &nodes, fresh)
    }

    /// The stroke's end: every point the stroke moved is rounded once more, with a dither from
    /// `rng` at its id, and the buffer is dropped. Returns the nodes written.
    pub fn finish(&mut self, store: &mut MeshStore, rng: CommandRng) -> Vec<NodeId> {
        let held = std::mem::take(&mut self.nodes);
        if held.is_empty() {
            return Vec::new();
        }
        let def = AttrDef::color();
        let col = store.attr_fits(&def).ok().flatten();
        let (nodes, bufs): (Vec<NodeId>, Vec<NodePaint>) = held.into_iter().unzip();
        let (s, strength) = (store.surface(), self.strength);
        let fresh = gs_par::map(nodes.len(), |i| {
            let (node, buf) = (s.node(nodes[i]), &bufs[i]);
            let bytes = colors(&node, col);
            let mut new = bytes.to_vec();
            let points = new.iter_mut().zip(buf.start.iter().zip(&buf.mixed));
            for (l, (c, (&start, &mixed))) in points.enumerate() {
                if mixed.map(f32::to_bits) != start.map(f32::to_bits) {
                    let id = u64::from(node.base) + l as u64;
                    *c = pack(seen(start, mixed, strength), rng.unit_at(id));
                }
            }
            if new == *bytes {
                NewValues::Same
            } else {
                NewValues::Values(AttrVec::U32(new))
            }
        });
        store.put_column(def, &nodes, fresh)
    }
}

/// One dab of Smooth painting color (decision 0039, section 8): each point it touches moves
/// toward its edge neighbours' mean color across visible faces (the mask blur's neighbours), in
/// linear light, by `strength` times Smooth's step, from the colors as the dab found them. Each
/// point it moves is rounded down after adding a number drawn from `rng` at the dab's count and
/// the point's id. Returns the nodes written: none on a part without colors, which is all white.
pub(crate) fn blend(
    store: &mut MeshStore,
    ctx: &Ctx<'_>,
    frames: &[Frame],
    gain: f32,
    strength: f32,
    rng: CommandRng,
    dab: u32,
) -> Vec<NodeId> {
    let def = AttrDef::color();
    let Some(col) = store.attr_fits(&def).ok().flatten() else {
        return Vec::new();
    };
    let spheres: Vec<(Vec3, f32)> = frames.iter().map(|f| (f.center, f.radius)).collect();
    let q = store.query_all(&spheres);
    let (nodes, skipped) = store.reachable(&q.brushed, ctx.node_reach());
    store.work.skipped += skipped.len() as u64;
    let s = store.surface();
    let (reader, visible, topo) = (s.attr_reader(col), s.visible(), s.topo());
    let word = |u: VertId| match reader.get(u) {
        Some(AttrValue::U32(w)) => w,
        _ => WHITE,
    };
    // Past every point id, so no dab's draw is the stroke end's.
    let key = (u64::from(dab) + 1) << 32;
    let fresh = gs_par::map(nodes.len(), |i| {
        let node = s.node(nodes[i]);
        let bytes = colors(&node, Some(col));
        let mut new = bytes.to_vec();
        let (mut ws, mut nb) = (vec![0.0; frames.len()], Vec::new());
        let plan = ctx.plan(&node, frames);
        for (l, c) in new.iter_mut().enumerate() {
            let Some(m) = ctx.influence(s, &node, &plan, l, frames, &mut ws) else {
                continue;
            };
            let v = node.base + l as VertId;
            edge_neighbours(&topo, v, |f| visible.face(f), &mut nb);
            if nb.iter().all(|&(u, _)| word(u) == bytes[l]) {
                continue;
            }
            let mut mean = [0.0f32; 3];
            for &(u, _) in &nb {
                for (x, y) in mean.iter_mut().zip(linear_rgb(word(u))) {
                    *x += y;
                }
            }
            let mean = mean.map(|x| x / nb.len() as f32);
            let w = ws.iter().sum::<f32>() * m;
            let frac = strength * (1.0 - libm::expf(-gain * w * frames[0].amount));
            let u = rng.unit_at(key | u64::from(v));
            *c = pack(seen(linear_rgb(bytes[l]), mean, frac), u);
        }
        if new == *bytes {
            NewValues::Same
        } else {
            NewValues::Values(AttrVec::U32(new))
        }
    });
    store.put_column(def, &nodes, fresh)
}

impl MeshStore {
    /// Fill Color (decision 0039, section 11): each live point goes from its color toward
    /// `color` (an opaque word, red lowest) by `strength` times how free its mask leaves it, in
    /// linear light as a paint stroke mixes, and is rounded once after adding a number drawn
    /// from `rng` at its id. A point whose weight is 1 takes `color` exactly. Fully masked
    /// points, points whose faces are all hidden and points already `color` keep their bytes.
    /// Returns the nodes written: none, and no column added, when nothing changes.
    pub fn fill_color(&mut self, color: u32, strength: f32, rng: CommandRng) -> Vec<NodeId> {
        let def = AttrDef::color();
        let col = self.attr_fits(&def).ok().flatten();
        let mask = self.attr_id(&AttrDef::mask().name);
        let nodes: Vec<NodeId> = (0..self.node_count() as NodeId).collect();
        let (s, target) = (self.surface(), linear_rgb(color));
        let fresh = gs_par::map(nodes.len(), |i| {
            let node = s.node(nodes[i]);
            let bytes = colors(&node, col);
            let held = match mask.and_then(|m| node.attr(m)) {
                Some(AttrVec::F32(m)) => Some(m),
                _ => None,
            };
            let mut new = bytes.to_vec();
            for (l, c) in new.iter_mut().enumerate() {
                let w = strength * held.map_or(1.0, |m| free(m[l]));
                if *c == color || w <= 0.0 || node.hidden(l) || node.pos[l].is_dead() {
                    continue;
                }
                *c = if w >= 1.0 {
                    color
                } else {
                    let u = rng.unit_at(u64::from(node.base) + l as u64);
                    pack(seen(linear_rgb(*c), target, w), u)
                };
            }
            if new == *bytes {
                NewValues::Same
            } else {
                NewValues::Values(AttrVec::U32(new))
            }
        });
        self.put_column(def, &nodes, fresh)
    }
}

/// How much a point of mask value `m` is free to change: the rule brushes use (`Ctx::common`),
/// where a value that is not a number masks fully.
fn free(m: f32) -> f32 {
    if m.is_finite() {
        1.0 - m.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// A node's colors: its column, or white where it has none (the column's empty value).
fn colors<'n>(node: &NodeView<'n>, col: Option<crate::columns::AttrId>) -> Cow<'n, [u32]> {
    match col.and_then(|c| node.attr(c)) {
        Some(AttrVec::U32(v)) => Cow::Borrowed(v),
        _ => Cow::Owned(vec![WHITE; node.pos.len()]),
    }
}

/// The color the stroke shows at a point: from `start` toward the dabs' mix by Color Strength,
/// sRGB-encoded in levels (0 to 255).
fn seen(start: Rgb, mixed: Rgb, strength: f32) -> Rgb {
    std::array::from_fn(|k| {
        let c = start[k] + (mixed[k] - start[k]) * strength;
        255.0 * linear_to_srgb(c.clamp(0.0, 1.0))
    })
}

/// Levels as an opaque color, each rounded down after adding `u`: 0.5 rounds to nearest, a
/// value from 0 to 1 drawn per point dithers.
fn pack(levels: Rgb, u: f32) -> u32 {
    let byte = |k: usize| ((levels[k] + u).floor() as u32).min(255) << (8 * k);
    byte(0) | byte(1) | byte(2) | 0xff00_0000
}
