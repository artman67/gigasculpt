//! Shortest paths over a small graph, the disc cut's trees and the halving's two fronts share.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::context::NONE;

/// A shortest-path tree from one or more sources.
pub(super) struct Tree {
    /// Each node's distance, infinite where no source reaches.
    pub dist: Vec<f64>,
    /// The node each was reached from, [`NONE`] at a source or out of reach.
    pub parent: Vec<u32>,
    /// The edge each was reached along, as `edges` named it, [`NONE`] where `parent` is.
    pub via: Vec<u32>,
    /// The source each was reached from, [`NONE`] out of reach.
    pub root: Vec<u32>,
}

/// Dijkstra from `sources` over `n` nodes, `edges(v, push)` calling `push(w, cost, edge)` for
/// each edge out of `v` (costs not below 0). Nodes settle by (distance, index), so ties go to
/// the lowest index and the tree is the same on every run.
pub(super) fn tree(
    n: usize,
    sources: impl IntoIterator<Item = usize>,
    edges: impl Fn(usize, &mut dyn FnMut(usize, f64, u32)),
) -> Tree {
    let mut t = Tree {
        dist: vec![f64::INFINITY; n],
        parent: vec![NONE; n],
        via: vec![NONE; n],
        root: vec![NONE; n],
    };
    let mut heap = BinaryHeap::new();
    for s in sources {
        t.dist[s] = 0.0;
        t.root[s] = s as u32;
        heap.push(Reverse((0_u64, s)));
    }
    let mut settled = vec![false; n];
    while let Some(Reverse((_, v))) = heap.pop() {
        if settled[v] {
            continue;
        }
        settled[v] = true;
        if t.parent[v] != NONE {
            t.root[v] = t.root[t.parent[v] as usize];
        }
        let d = t.dist[v];
        edges(v, &mut |w, cost, edge| {
            let reach = d + cost;
            if !settled[w] && reach < t.dist[w] {
                t.dist[w] = reach;
                t.parent[w] = v as u32;
                t.via[w] = edge;
                heap.push(Reverse((reach.to_bits(), w)));
            }
        });
    }
    t
}

/// The node farthest from the sources of `t` that they reach, ties to the lowest.
pub(super) fn farthest(t: &Tree) -> usize {
    let reached = (0..t.dist.len()).filter(|&v| t.dist[v].is_finite());
    reached
        .reduce(|best, v| if t.dist[v] > t.dist[best] { v } else { best })
        .unwrap_or(0)
}
