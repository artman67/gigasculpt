//! Each chart cut to a disc on its wedge mesh: its extra boundary loops joined to the outer one
//! along a shortest-path tree, then its handles cut by tree and cotree (UV2.3).

use std::cmp::Reverse;

use super::context::{Ctx, NONE};
use super::edges::OPEN;
use super::local::Local;
use super::path::{Tree, tree};
use crate::weld::{find, join};

/// What cutting a chart to a disc came to.
pub(super) enum Disc {
    /// A disc: its mesh as cut, and the cuts, each edge as one of the part's half-edges.
    Done(Local, Vec<usize>),
    /// No loop to start from (a closed chart), or still no disc after the cut: to be halved.
    Closed,
    /// A cut must cross this half-edge of the part, inside a protected region.
    Protected(usize),
}

/// Cuts the chart of `members` (lowest first; `at` the charts and places of every face) to a
/// disc.
pub(super) fn disc(ctx: &Ctx, at: (&[u32], &[u32]), members: &[u32]) -> Disc {
    let faces = ctx.surface.faces;
    let hard = |h: usize| ctx.hard[h / 4] >> (h % 4) & 1 == 1;
    let mut local = Local::new(faces, &ctx.edges, at, members.to_vec(), hard);
    let mut cuts = Vec::new();
    for step in [loops, handles] {
        if local.is_disc() {
            break;
        }
        let Some(more) = step(ctx, &local) else {
            return Disc::Closed;
        };
        if let Some(&h) = more.iter().find(|&&h| ctx.guarded(h)) {
            return Disc::Protected(h);
        }
        if more.is_empty() {
            continue;
        }
        cuts.extend(more);
        cuts.sort_unstable();
        let cut = |h: usize| hard(h) || cuts.binary_search(&edge(ctx, h)).is_ok();
        local = Local::new(faces, &ctx.edges, at, members.to_vec(), cut);
    }
    if local.is_disc() {
        Disc::Done(local, cuts)
    } else {
        Disc::Closed
    }
}

/// The half-edge that names half-edge `h`'s edge: the lower of it and its twin.
fn edge(ctx: &Ctx, h: usize) -> usize {
    let t = ctx.edges.twin[h] as usize;
    h.min(t)
}

/// A shortest-path tree over the chart's wedges from `sources`, each edge inside the chart
/// costing its [`Ctx::weight`] and each boundary edge `free` says is free costing nothing.
fn grow(ctx: &Ctx, local: &Local, sources: &[usize], free: &[bool]) -> Tree {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); local.chart.points.len()];
    for h in local.halves() {
        let (a, b) = local.ends(h);
        out[a].push(h);
        if local.twin[h] == OPEN {
            out[b].push(h);
        }
    }
    tree(
        local.chart.points.len(),
        sources.iter().copied(),
        |v, push| {
            for &h in &out[v] {
                let (a, b) = local.ends(h);
                let w = if a == v { b } else { a };
                let cost = if local.twin[h] != OPEN {
                    ctx.weight(local.global(h))
                } else if free[h] {
                    0.0
                } else {
                    continue;
                };
                push(w, cost, h as u32);
            }
        },
    )
}

/// The edges inside the chart on the tree path from wedge `v` back to its source, as the
/// part's half-edges.
fn walk(local: &Local, t: &Tree, mut v: usize, out: &mut Vec<usize>) {
    while t.parent[v] != NONE {
        let h = t.via[v] as usize;
        if local.twin[h] != OPEN {
            out.push(local.global(h));
        }
        v = t.parent[v] as usize;
    }
}

/// The cuts joining every extra boundary loop to the outer one, the longest (ties to the
/// lowest edge), each from its wedge nearest the outer loop; `None` for a chart with no loop.
fn loops(ctx: &Ctx, local: &Local) -> Option<Vec<usize>> {
    let rings = local.loops().filter(|r| !r.is_empty())?;
    let length = |r: &Vec<usize>| -> f64 { r.iter().map(|&h| ctx.len(local.global(h))).sum() };
    let outer = (0..rings.len())
        .reduce(|best, i| {
            if length(&rings[i]) > length(&rings[best]) {
                i
            } else {
                best
            }
        })
        .unwrap_or(0);
    let mut free = vec![false; local.twin.len()];
    for (_, ring) in rings.iter().enumerate().filter(|&(i, _)| i != outer) {
        for &h in ring {
            free[h] = true;
        }
    }
    let sources: Vec<usize> = rings[outer].iter().map(|&h| local.ends(h).0).collect();
    let t = grow(ctx, local, &sources, &free);
    let mut cuts = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        if i == outer {
            continue;
        }
        let near = (ring.iter().map(|&h| local.ends(h).0))
            .min_by(|&a, &b| t.dist[a].total_cmp(&t.dist[b]).then(a.cmp(&b)))?;
        if !t.dist[near].is_finite() {
            return None;
        }
        walk(local, &t, near, &mut cuts);
    }
    Some(dedup(ctx, cuts))
}

/// The cuts that open every handle of a chart with one loop: each edge in neither the tree from
/// the loop nor the cotree, with the tree paths from both its ends.
fn handles(ctx: &Ctx, local: &Local) -> Option<Vec<usize>> {
    let rings = local.loops().filter(|r| r.len() == 1)?;
    let sources: Vec<usize> = rings[0].iter().map(|&h| local.ends(h).0).collect();
    let t = grow(ctx, local, &sources, &vec![false; local.twin.len()]);
    let mut in_tree = vec![false; local.twin.len()];
    for v in (0..t.via.len()).filter(|&v| t.via[v] != NONE) {
        let h = t.via[v] as usize;
        in_tree[h] = true;
        in_tree[local.twin[h] as usize] = true;
    }
    let mut dual: Vec<(Reverse<u64>, usize)> = (local.halves())
        .filter(|&h| local.twin[h] != OPEN && (h as u32) < local.twin[h] && !in_tree[h])
        .map(|h| {
            let (a, b) = local.ends(h);
            let span = t.dist[a] + t.dist[b] + ctx.weight(local.global(h));
            (Reverse(span.to_bits()), h)
        })
        .collect();
    dual.sort_unstable();
    let mut root: Vec<usize> = (0..local.chart.faces.len()).collect();
    let mut cuts = Vec::new();
    for (_, h) in dual {
        let (f, g) = (h / 4, local.twin[h] as usize / 4);
        if find(&mut root, f) == find(&mut root, g) {
            let (a, b) = local.ends(h);
            cuts.push(local.global(h));
            walk(local, &t, a, &mut cuts);
            walk(local, &t, b, &mut cuts);
        } else {
            join(&mut root, f, g);
        }
    }
    Some(dedup(ctx, cuts))
}

/// Each cut edge once, named by [`edge`], in order.
fn dedup(ctx: &Ctx, cuts: Vec<usize>) -> Vec<usize> {
    let mut cuts: Vec<usize> = cuts.into_iter().map(|h| edge(ctx, h)).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}
