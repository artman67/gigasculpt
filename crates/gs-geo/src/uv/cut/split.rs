//! Every chart cut to a disc and checked, and one that fails halved and tried again, in rounds
//! (UV2.3 and UV2.4).

use super::context::{Ctx, NONE};
use super::disc::{Disc, disc};
use super::edges::OPEN;
use super::grow::State;
use super::local::{Local, places};
use super::path::{farthest, tree};
use crate::math::{dot, sub};
use crate::uv::metrics::{Sums, triangles};
use crate::uv::{Chart, Refusal, Surface};
use crate::weld::{find, join};

/// Rounds of halving before a chart is kept as it is.
const ROUNDS: usize = 64;
/// The worst stretch a chart may have.
const WORST: f64 = 2.0;
/// The area spread a chart may have.
const SPREAD: f64 = 0.2;
/// How much longer than wide a chart's box may be.
const LONG: f64 = 8.0;
/// How much more a painted edge costs the halving's two fronts to cross, per unit of attract.
const PULL: f64 = 4.0;

/// What becomes of a chart this round.
enum Outcome {
    /// Kept, with these cuts made inside it.
    Keep(Vec<usize>),
    /// Split into these parts, each its faces, lowest first.
    Halve(Vec<Vec<u32>>),
    Refuse(Refusal),
}

/// Cuts every chart in `state` to a disc, halving those `flatten` lays out badly, and records
/// each kept chart's cuts.
pub(super) fn settle<F>(ctx: &Ctx, state: &mut State, flatten: &F) -> Result<(), Refusal>
where
    F: Fn(&Chart, Surface) -> Vec<[f32; 2]> + Sync,
{
    let mut pending: Vec<u32> = (0..state.count).collect();
    let mut round = 0;
    while !pending.is_empty() {
        let places = places(&state.chart, state.count as usize);
        let at = (state.chart.as_slice(), places.slot.as_slice());
        let outcomes = gs_par::map(pending.len(), |i| {
            judge(ctx, at, &places.faces[pending[i] as usize], round, flatten)
        });
        let mut next = Vec::new();
        for (&c, outcome) in pending.iter().zip(outcomes) {
            match outcome {
                Outcome::Keep(cuts) => {
                    for h in cuts {
                        let t = ctx.edges.twin[h] as usize;
                        state.cut[h / 4] |= 1 << (h % 4);
                        state.cut[t / 4] |= 1 << (t % 4);
                    }
                }
                Outcome::Halve(parts) => {
                    for (i, part) in parts.into_iter().enumerate() {
                        let id = if i == 0 { c } else { state.count };
                        state.count += u32::from(i > 0);
                        for &f in &part {
                            state.chart[f as usize] = id;
                        }
                        next.push(id);
                    }
                }
                Outcome::Refuse(refusal) => return Err(refusal),
            }
        }
        pending = next;
        round += 1;
    }
    Ok(())
}

/// What becomes of the chart of `members` in round `round`.
fn judge<F>(ctx: &Ctx, at: (&[u32], &[u32]), members: &[u32], round: usize, flatten: &F) -> Outcome
where
    F: Fn(&Chart, Surface) -> Vec<[f32; 2]> + Sync,
{
    let last = round >= ROUNDS;
    let whole = last || members.len() < 2 || one_region(ctx, members);
    match disc(ctx, at, members) {
        Disc::Protected(h) => Outcome::Refuse(ctx.refusal(h)),
        Disc::Done(local, cuts) => {
            if whole
                || ctx.kept.is_some()
                || passes(&local, &flatten(&local.chart, ctx.surface), ctx.surface)
            {
                return Outcome::Keep(cuts);
            }
            halve(ctx, &local).map_or(Outcome::Keep(cuts), Outcome::Halve)
        }
        Disc::Closed => {
            let parts = if whole {
                None
            } else {
                halve(ctx, &unjoined(ctx, at, members))
            };
            if let Some(parts) = parts {
                return Outcome::Halve(parts);
            }
            match members.iter().find(|&&f| ctx.protected(f as usize)) {
                Some(&f) => Outcome::Refuse(ctx.refusal(4 * f as usize)),
                None if members.len() < 2 => Outcome::Keep(Vec::new()),
                None => Outcome::Halve(members.iter().map(|&f| vec![f]).collect()),
            }
        }
    }
}

/// The chart of `members` before any cut, as halving reads it.
fn unjoined(ctx: &Ctx, at: (&[u32], &[u32]), members: &[u32]) -> Local {
    let hard = |h: usize| ctx.hard[h / 4] >> (h % 4) & 1 == 1;
    Local::new(ctx.surface.faces, &ctx.edges, at, members.to_vec(), hard)
}

/// Whether every face of the chart is in one protected region.
fn one_region(ctx: &Ctx, members: &[u32]) -> bool {
    let first = ctx.region[members[0] as usize];
    first != NONE && members.iter().all(|&f| ctx.region[f as usize] == first)
}

/// Whether the chart laid out as `uvs`, a UV per wedge, passes the check: every triangle wound
/// face up, worst stretch at most 2, area spread at most 0.2, its box at most 8 times longer
/// than wide.
fn passes(local: &Local, uvs: &[[f32; 2]], surface: Surface) -> bool {
    let chart = &local.chart;
    let mut tris = Vec::new();
    for (i, &f) in chart.faces.iter().enumerate() {
        let face = surface.faces[f as usize];
        let at = |k: usize| surface.points[face[k] as usize];
        tris.extend(triangles(face, at, |k| uvs[chart.corners[i][k] as usize]));
    }
    if tris.iter().any(|t| t.area3 > 0.0 && t.area2 <= 0.0) {
        return false;
    }
    let sums = tris.iter().fold(Sums::default(), Sums::add);
    if sums.area3 <= 0.0 {
        return true;
    }
    let s = sums.scale();
    let spread: f64 = (tris.iter())
        .filter(|t| t.area3 > 0.0)
        .map(|t| t.spread(s * s))
        .sum();
    let extent = |i: usize| {
        let lo = uvs
            .iter()
            .map(|p| f64::from(p[i]))
            .fold(f64::INFINITY, f64::min);
        let hi = uvs
            .iter()
            .map(|p| f64::from(p[i]))
            .fold(f64::NEG_INFINITY, f64::max);
        hi - lo
    };
    let (u, v) = (extent(0), extent(1));
    sums.worst * s <= WORST && spread / sums.area3 <= SPREAD && u.max(v) <= LONG * u.min(v)
}

/// The chart halved: two seeds, the face farthest from its lowest and the one farthest from
/// that along face centres, grow at once, crossing a painted edge costing more; each protected
/// region goes whole to the side of its lowest face, and each side splits into its connected
/// pieces. `None` when a side would be empty.
fn halve(ctx: &Ctx, local: &Local) -> Option<Vec<Vec<u32>>> {
    let faces = &local.chart.faces;
    if faces.len() < 2 {
        return None;
    }
    let centre = |i: usize| ctx.face[faces[i] as usize].centre;
    let step = |pull: f64| {
        move |i: usize, push: &mut dyn FnMut(usize, f64, u32)| {
            for h in (4 * i..4 * i + 4).filter(|&h| local.twin.get(h).is_some_and(|&t| t != OPEN)) {
                let j = local.twin[h] as usize / 4;
                let d = sub(centre(i), centre(j));
                let cost = dot(d, d).sqrt() * (1.0 + pull * ctx.attract(local.global(h)));
                push(j, cost, h as u32);
            }
        }
    };
    let a = farthest(&tree(faces.len(), [0], step(0.0)));
    let b = farthest(&tree(faces.len(), [a], step(0.0)));
    if a == b {
        return None;
    }
    let mut owner = tree(faces.len(), [a, b], step(PULL)).root;
    for i in 0..faces.len() {
        let region = ctx.region[faces[i] as usize];
        if region != NONE {
            let lowest = faces.binary_search(&region).unwrap_or(i);
            owner[i] = owner[lowest];
        }
    }
    let side = |i: usize| owner[i] == b as u32;
    if (0..faces.len()).all(side) || !(0..faces.len()).any(side) {
        return None;
    }
    let mut root: Vec<usize> = (0..faces.len()).collect();
    for h in (0..4 * faces.len()).filter(|&h| local.twin[h] != OPEN) {
        let j = local.twin[h] as usize / 4;
        if side(h / 4) == side(j) {
            join(&mut root, h / 4, j);
        }
    }
    let mut parts: Vec<Vec<u32>> = Vec::new();
    let mut part = vec![NONE; faces.len()];
    for (i, &face) in faces.iter().enumerate() {
        let r = find(&mut root, i);
        if part[r] == NONE {
            part[r] = parts.len() as u32;
            parts.push(Vec::new());
        }
        parts[part[r] as usize].push(face);
    }
    Some(parts)
}
