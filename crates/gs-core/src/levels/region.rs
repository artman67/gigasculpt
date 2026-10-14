//! The points one carrying step reaches (decision 0014, section 5), marked over the fine
//! level's store ids, and values for them kept in arrays per node: scratch that follows the
//! edit, and that is read back without searching.

use crate::bits::Bits;
use crate::config::{NodeConfig, NodeId, VertId};
use crate::math::NO;

use super::rules::Topo;

/// Points handed to one parallel task when gathering neighbours.
const BATCH: usize = 1024;

/// An empty set of one level's store points: one bit per id slot (a store id is its node's
/// first slot plus its local slot, so it indexes the bits directly).
pub(crate) fn marks(cfg: NodeConfig, nodes: usize) -> Bits {
    Bits::zeros(nodes * cfg.capacity() as usize)
}

/// Marks every point `near` lists for each point of `from` (gathered in parallel); returns the
/// points newly marked, in the order found.
fn mark(marks: &mut Bits, from: &[u32], near: &(impl Fn(u32, &mut Vec<u32>) + Sync)) -> Vec<u32> {
    let found = gs_par::map(from.len().div_ceil(BATCH), |k| {
        let mut out = Vec::new();
        for &v in &from[k * BATCH..((k + 1) * BATCH).min(from.len())] {
            near(v, &mut out);
        }
        out
    });
    let mut new = Vec::new();
    for v in found.into_iter().flatten() {
        if !marks.get(v as usize) {
            marks.set(v as usize, true);
            new.push(v);
        }
    }
    new
}

/// One step's points on the fine level, as store ids.
pub(crate) struct Region {
    /// Points whose rule reads a changed coarse point, and every point sharing a face with one
    /// (whose frame can turn).
    pub area: Vec<u32>,
    /// `area` and one more ring: every point a frame of `area` reads.
    pub seen: Vec<u32>,
}

/// The region of the coarse points `changed` (their store ids on the fine level, where each
/// keeps its canonical index). A coarse point's rule-readers are every point of the children
/// of the coarse faces around it: each such face has one child around the point, whose corner
/// across from it is the face point, and the face point's faces are all the children.
pub(crate) fn region(fine: &Topo, changed: &[u32], nodes: usize) -> Region {
    let mut marks = marks(fine.cfg, nodes);
    let corners = |v: u32, out: &mut Vec<u32>| {
        for &f in fine.ring_at(v) {
            out.extend(fine.stored(f).iter().filter(|&&u| u != NO));
        }
    };
    let children = |v: u32, out: &mut Vec<u32>| {
        for &f in fine.ring_at(v) {
            let fc = fine.stored(f);
            if let Some(at) = fc.iter().position(|&u| u == v) {
                corners(fc[(at + 2) % 4], out);
            }
        }
    };
    let reach = mark(&mut marks, changed, &children);
    let more = mark(&mut marks, &reach, &corners);
    let ring = mark(&mut marks, &more, &corners);
    let mut area = reach;
    area.extend(more);
    let mut seen = area.clone();
    seen.extend(ring);
    Region { area, seen }
}

/// Every point of the fine level, as both the area and what it reads: the oracle the region is
/// checked against (review 0004, T1).
pub(crate) fn whole_level(fine: &Topo) -> Region {
    let all: Vec<u32> = (0..fine.num.vertex_count() as u32)
        .map(|c| fine.num.id(c))
        .collect();
    Region {
        seen: all.clone(),
        area: all,
    }
}

/// Values for some points of a level, in one array per node that holds any.
pub(crate) struct PerNode<T> {
    cfg: NodeConfig,
    nodes: Vec<Option<Vec<T>>>,
}

/// `points` (store ids) grouped by node: each node's points in the order given.
fn by_node(cfg: NodeConfig, nodes: usize, points: &[u32]) -> Vec<(NodeId, Vec<u32>)> {
    let mut count = vec![0usize; nodes];
    for &v in points {
        count[cfg.node_of(v) as usize] += 1;
    }
    let mut groups: Vec<Vec<u32>> = count.iter().map(|&n| Vec::with_capacity(n)).collect();
    for &v in points {
        groups[cfg.node_of(v) as usize].push(v);
    }
    (groups.into_iter().enumerate())
        .filter(|(_, g)| !g.is_empty())
        .map(|(n, g)| (n as NodeId, g))
        .collect()
}

impl<T: Copy + Default + Send + Sync> PerNode<T> {
    /// `value(v)` for every point of `points`, computed in parallel node by node; `slots(n)` is
    /// node `n`'s slot count.
    pub fn compute(
        cfg: NodeConfig,
        nodes: usize,
        slots: &(impl Fn(NodeId) -> usize + Sync),
        points: &[u32],
        value: &(impl Fn(u32) -> T + Sync),
    ) -> Self {
        let groups = by_node(cfg, nodes, points);
        let arrays = gs_par::map(groups.len(), |i| {
            let (n, pts) = &groups[i];
            let mut a = vec![T::default(); slots(*n)];
            for &v in pts {
                a[cfg.local_of(v) as usize] = value(v);
            }
            a
        });
        let mut out = vec![None; nodes];
        for ((n, _), a) in groups.iter().zip(arrays) {
            out[*n as usize] = Some(a);
        }
        Self { cfg, nodes: out }
    }

    /// The values of `pairs` (store id, value), each at its point.
    pub fn from_pairs(
        cfg: NodeConfig,
        nodes: usize,
        slots: &(impl Fn(NodeId) -> usize + Sync),
        pairs: &[(u32, T)],
    ) -> Self {
        let mut out: Vec<Option<Vec<T>>> = vec![None; nodes];
        for &(v, x) in pairs {
            let n = cfg.node_of(v);
            let a = out[n as usize].get_or_insert_with(|| vec![T::default(); slots(n)]);
            a[cfg.local_of(v) as usize] = x;
        }
        Self { cfg, nodes: out }
    }

    /// The value at store point `v` (the default where none was computed in its node).
    #[inline]
    pub fn get(&self, v: VertId) -> T {
        let node = self.nodes[self.cfg.node_of(v) as usize].as_ref();
        node.map_or(T::default(), |a| a[self.cfg.local_of(v) as usize])
    }
}

/// For each point of `points`, in parallel node by node, what `each` gives (`None` to leave a
/// point out): the results node after node, each node's in the order given.
pub(crate) fn per_node<T: Send>(
    cfg: NodeConfig,
    nodes: usize,
    points: &[u32],
    each: &(impl Fn(u32) -> Option<T> + Sync),
) -> Vec<T> {
    let groups = by_node(cfg, nodes, points);
    let out = gs_par::map(groups.len(), |i| {
        groups[i]
            .1
            .iter()
            .filter_map(|&v| each(v))
            .collect::<Vec<T>>()
    });
    out.into_iter().flatten().collect()
}
