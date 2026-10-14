//! Charts grown one at a time from the flattest face, the cheapest neighbour next, while it
//! costs at most 0.4 (decision 0061 section 1, UV2.2).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use gs_protocol::arity;

use super::context::{Ctx, NONE};
use crate::math::dot;

/// The most a face may cost to join a chart.
const LIMIT: f64 = 0.4;
/// What the boundary term costs per unit of boundary over a square's.
const ROUND: f64 = 0.15;
/// A square's boundary over the square root of its area.
const SQUARE: f64 = 4.0;

/// The charts as the cutter has them: a chart per face ([`NONE`] for a face left to its
/// mirror), how many, and the cuts made inside each, a bit per half-edge as in the seam masks.
pub(super) struct State {
    pub chart: Vec<u32>,
    pub count: u32,
    pub cut: Vec<u8>,
}

/// The chart growing now: its area-weighted normal and density, area and boundary length.
#[derive(Default)]
struct Grown {
    normal: [f64; 3],
    density: f64,
    area: f64,
    boundary: f64,
}

/// Grows every chart.
pub(super) fn grow(ctx: &Ctx) -> State {
    let faces = ctx.surface.faces;
    let mut state = State {
        chart: vec![NONE; faces.len()],
        count: 0,
        cut: vec![0; faces.len()],
    };
    let limit = if ctx.kept.is_some() {
        f64::INFINITY
    } else {
        LIMIT
    };
    for f in seeds(ctx) {
        if state.chart[f] != NONE {
            continue;
        }
        let c = state.count;
        state.count += 1;
        let mut grown = Grown::default();
        let mut heap = BinaryHeap::new();
        join(ctx, &mut state.chart, (c, f), &mut grown, &mut heap);
        while let Some(Reverse((_, g))) = heap.pop() {
            if state.chart[g] == NONE && cost(ctx, &state.chart, (c, g), &grown) <= limit {
                join(ctx, &mut state.chart, (c, g), &mut grown, &mut heap);
            }
        }
    }
    state
}

/// The faces growth starts from, flattest first, ties to the lowest: a face's flatness is its
/// mean `1 - n_f · n_g` over the neighbours it could join.
fn seeds(ctx: &Ctx) -> impl Iterator<Item = usize> {
    let faces = ctx.surface.faces;
    let active: Vec<usize> = (0..faces.len()).filter(|&f| ctx.active(f)).collect();
    let mut keys = gs_par::map(active.len(), |i| {
        let f = active[i];
        let (mut sum, mut n) = (0.0, 0);
        for k in 0..arity(faces[f]) {
            if let Some(t) = ctx.cross(4 * f + k) {
                sum += 1.0 - dot(ctx.face[f].normal, ctx.face[t / 4].normal);
                n += 1;
            }
        }
        let flat = if n == 0 { 0.0 } else { sum / f64::from(n) };
        u64::from((flat.max(0.0) as f32).to_bits()) << 32 | f as u64
    });
    gs_par::sort_keys(&mut keys);
    keys.into_iter().map(|k| (k & u64::from(u32::MAX)) as usize)
}

/// What face `f` costs to join chart `c`, grown so far as `grown`.
fn cost(ctx: &Ctx, chart: &[u32], (c, f): (u32, usize), grown: &Grown) -> f64 {
    let face = &ctx.face[f];
    let (change, attract) = meet(ctx, chart, (c, f));
    let boundary = grown.boundary + change;
    let length = dot(grown.normal, grown.normal).sqrt();
    let bend = if length > 0.0 {
        1.0 - dot(face.normal, grown.normal) / length
    } else {
        0.0
    };
    let area = grown.area + face.area;
    let round = if area > 0.0 {
        ROUND * (boundary / area.sqrt() - SQUARE).max(0.0)
    } else {
        0.0
    };
    let density = if grown.area > 0.0 {
        (face.density - grown.density / grown.area).abs()
    } else {
        0.0
    };
    (bend + round + attract + density).max(0.0)
}

/// How face `f` joining chart `c` changes its boundary length, and the largest attract among
/// the edges it would take off the boundary.
fn meet(ctx: &Ctx, chart: &[u32], (c, f): (u32, usize)) -> (f64, f64) {
    let (mut change, mut attract) = (0.0, 0.0_f64);
    for k in 0..arity(ctx.surface.faces[f]) {
        let h = 4 * f + k;
        if ctx.cross(h).is_some_and(|t| chart[t / 4] == c) {
            change -= ctx.len(h);
            attract = attract.max(ctx.attract(h));
        } else {
            change += ctx.len(h);
        }
    }
    (change, attract)
}

/// Adds face `f` to chart `c`, with its whole protected region when it has one, and offers
/// each new face's neighbours to the heap at their cost now.
fn join(
    ctx: &Ctx,
    chart: &mut [u32],
    (c, f): (u32, usize),
    grown: &mut Grown,
    heap: &mut BinaryHeap<Reverse<(u64, usize)>>,
) {
    let mut added = vec![f];
    if ctx.protected(f) {
        added = ctx.members(ctx.region[f]).map(|g| g as usize).collect();
    }
    for &g in &added {
        if chart[g] == NONE {
            take(ctx, chart, (c, g), grown);
        }
    }
    for &g in &added {
        for k in 0..arity(ctx.surface.faces[g]) {
            if let Some(t) = ctx.cross(4 * g + k)
                && chart[t / 4] == NONE
                && ctx.active(t / 4)
            {
                let price = cost(ctx, chart, (c, t / 4), grown);
                heap.push(Reverse((price.to_bits(), t / 4)));
            }
        }
    }
}

/// Puts face `f` in chart `c` and adds it to `grown`.
fn take(ctx: &Ctx, chart: &mut [u32], (c, f): (u32, usize), grown: &mut Grown) {
    let face = &ctx.face[f];
    grown.boundary += meet(ctx, chart, (c, f)).0;
    chart[f] = c;
    grown.normal = std::array::from_fn(|i| grown.normal[i] + face.normal[i] * face.area);
    grown.density += face.density * face.area;
    grown.area += face.area;
}
