//! Carrying edits made on a lower level up to the top (decision 0014, section 5).
//!
//! One level at a time. A fine point the edit can reach (its rule or its frame reads a changed
//! point) is put back at its old offset from the predicted surface, measured in an orthonormal
//! frame on that surface before the edit and applied in the frame after it. Every other point
//! keeps its exact bits. Point attributes travel as deltas, face attributes go to every
//! descendant top face. Work and scratch memory follow the edit, never the mesh.

use std::sync::Arc;

use crate::columns::{AttrDef, AttrKind, AttrValue, AttrVec, Domain, NodeTopo};
use crate::config::NodeId;
use crate::math::{NO, Vec3};
use crate::numbering::Numbering;
use crate::par::select_mut;
use crate::store::MeshStore;
use crate::view::PosRead;
use crate::visibility::Cover;

use super::region::{PerNode, marks, per_node, region, whole_level};
use super::rules::{Parent, Rule, Topo, frame, into_frame, out_of_frame};
use super::{Base, Level};

/// What carrying up wrote into the top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Carried {
    /// Top points whose position changed.
    pub points: usize,
    /// Top points or faces whose attribute values changed.
    pub values: usize,
}

/// Sorted, distinct.
fn unique(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v.dedup();
    v
}

/// Every fine face in the rings of `points`, sorted and distinct.
fn faces_around(fine: &Topo, points: &[u32]) -> Vec<u32> {
    let per = gs_par::map(points.len(), |i| fine.ring(points[i]).to_vec());
    unique(per.concat())
}

/// Every corner of `faces`, sorted and distinct.
fn corners(fine: &Topo, faces: &[u32]) -> Vec<u32> {
    let per = gs_par::map(faces.len(), |i| fine.face(faces[i]));
    unique(per.into_iter().flatten().filter(|&v| v != NO).collect())
}

/// The fine points whose rule reads one of `changed` (coarse points): every point of the
/// children of the coarse faces around them. Each coarse face around `c` has one child around
/// `c`, whose corner across from `c` is the face point; the face point's faces are all the
/// children.
fn reached(fine: &Topo, changed: &[u32], everything: bool) -> Vec<u32> {
    if everything {
        return (0..fine.num.vertex_count() as u32).collect();
    }
    let per = gs_par::map(changed.len(), |i| {
        let c = changed[i];
        let mut m = Vec::new();
        for &f in fine.ring(c) {
            let fc = fine.face(f);
            if let Some(at) = fc.iter().position(|&u| u == c) {
                m.push(fc[(at + 2) % 4]);
            }
        }
        m
    });
    let points = unique(per.concat());
    corners(fine, &faces_around(fine, &points))
}

/// A value of `of`, sorted by key, looked up by binary search.
fn find<T: Copy>(of: &[(u32, T)], c: u32) -> Option<T> {
    of.binary_search_by_key(&c, |x| x.0).ok().map(|i| of[i].1)
}

/// A point that changed on one level: its store id there, its canonical index and its new
/// position.
type Moved = (u32, u32, Vec3);

/// One level's fine nodes: how many, and each one's slot count.
struct Shape<'a> {
    nodes: usize,
    topo: &'a [Arc<NodeTopo>],
}

impl Shape<'_> {
    fn slots(&self) -> impl Fn(NodeId) -> usize + Sync + '_ {
        |n| self.topo[n as usize].ring.len()
    }
}

/// One step of carrying positions up: the fine points that changed, from the coarse ones
/// (`changed`), whose new positions `new` reads (the old ones `old` reads), by canonical index.
fn step_positions(
    rule: &Rule,
    fine: &Topo,
    coarse: u32,
    changed: &[Moved],
    new: &(impl Fn(u32) -> Vec3 + Sync),
    old: &(impl Fn(u32) -> Vec3 + Sync),
    everything: bool,
) -> Vec<Moved> {
    let shape = Shape {
        nodes: fine.nodes.len(),
        topo: fine.nodes,
    };
    let seeds: Vec<u32> = changed.iter().map(|x| fine.num.id(x.1)).collect();
    let r = if everything {
        whole_level(fine)
    } else {
        region(fine, &seeds, shape.nodes)
    };
    // Predicted positions before and after, over every point a frame of the area reads.
    let q = PerNode::compute(fine.cfg, shape.nodes, &shape.slots(), &r.seen, &|v| {
        let parent = Parent::of(fine, fine.num.canonical(v), coarse);
        (parent.place(rule, old), parent.place(rule, new))
    });
    let q_old = |c: u32| q.get(fine.num.id(c)).0;
    let q_new = |c: u32| q.get(fine.num.id(c)).1;
    let moved = |v: u32| {
        let (a, b) = q.get(v);
        a.bits() != b.bits()
    };
    per_node(fine.cfg, shape.nodes, &r.area, &|v| {
        let around =
            (fine.ring_at(v).iter()).any(|&f| fine.stored(f).iter().any(|&u| u != NO && moved(u)));
        if !moved(v) && !around {
            return None;
        }
        let c = fine.num.canonical(v);
        let (before, after) = (q_old(c), q_new(c));
        let detail = old(c) - before;
        let placed = match (frame(fine, c, &q_old), frame(fine, c, &q_new)) {
            (Some(r_old), Some(r_new)) => after + out_of_frame(&r_new, into_frame(&r_old, detail)),
            _ => after + detail,
        };
        (placed.bits() != old(c).bits()).then_some((v, c, placed))
    })
}

/// A point attribute's change as up to four channels: a scalar, or a color's.
type Delta = [f32; 4];

fn channels(kind: AttrKind, x: AttrValue) -> Delta {
    match (kind, x) {
        (AttrKind::Rgba8, AttrValue::U32(c)) => {
            [0, 8, 16, 24].map(|s| f32::from(((c >> s) & 255) as u8))
        }
        (_, AttrValue::F32(f)) => [f, 0.0, 0.0, 0.0],
        (_, AttrValue::U32(u)) => [u as f32, 0.0, 0.0, 0.0],
    }
}

fn minus(a: Delta, b: Delta) -> Delta {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

/// The mean of deltas, in the given order.
fn mean(ds: &[Delta]) -> Delta {
    let mut s = [0.0f32; 4];
    for d in ds {
        for (x, y) in s.iter_mut().zip(d) {
            *x += y;
        }
    }
    let w = 1.0 / ds.len() as f32;
    s.map(|x| x * w)
}

/// One step of carrying a point attribute's deltas up: even points copy their parent's, edge
/// points take the mean of their ends', face points of their corners'.
fn step_deltas(
    fine: &Topo,
    coarse: u32,
    deltas: &[(u32, Delta)],
    everything: bool,
) -> Vec<(u32, Delta)> {
    let keys: Vec<u32> = deltas.iter().map(|x| x.0).collect();
    let reach = reached(fine, &keys, everything);
    let d = |c: u32| find(deltas, c).unwrap_or([0.0; 4]);
    let out = gs_par::map(reach.len(), |i| {
        let v = reach[i];
        let delta = match Parent::of(fine, v, coarse) {
            Parent::Vertex(c) => d(c),
            Parent::Edge(a, b) => mean(&[d(a), d(b)]),
            Parent::Face(f) => {
                let n = f.iter().filter(|&&c| c != NO).count();
                let ds: Vec<Delta> = f[..n].iter().map(|&c| d(c)).collect();
                mean(&ds)
            }
        };
        delta.iter().any(|x| x.abs() > 0.0).then_some((v, delta))
    });
    out.into_iter().flatten().collect()
}

/// The value an old top value takes with a delta added, as the column keeps it: a color clamped
/// per channel, a ranged column (the mask, a UV hint) settled in its range ([`AttrDef::stored`]).
fn add(def: &AttrDef, old: AttrValue, d: Delta) -> AttrValue {
    match (def.kind, old) {
        (AttrKind::Rgba8, AttrValue::U32(c)) => {
            let mut out = 0u32;
            for (i, s) in [0u32, 8, 16, 24].into_iter().enumerate() {
                let x = f32::from(((c >> s) & 255) as u8) + d[i];
                out |= ((x.round().clamp(0.0, 255.0)) as u32) << s;
            }
            AttrValue::U32(out)
        }
        (_, AttrValue::F32(x)) => def.stored(AttrValue::F32(x + d[0])),
        (_, other) => other,
    }
}

/// The levels and stores one carry reads.
pub(crate) struct Stack<'a> {
    pub levels: &'a [Arc<Level>],
    pub base: &'a Base,
    /// The active level's store and its state when gathered.
    pub live: &'a MeshStore,
    pub gathered: &'a MeshStore,
    pub k: usize,
    /// Every point of each level instead of the marked region (see `Levels::carry_everything`).
    pub everything: bool,
}

impl Stack<'_> {
    fn topo<'t>(&'t self, j: usize, top: &'t MeshStore) -> Topo<'t> {
        let cfg = top.cfg;
        let nodes: &[Arc<NodeTopo>] = match &self.levels[j].skeleton {
            Some(s) => &s.topo,
            None => &top.cols.topo,
        };
        Topo {
            cfg,
            nodes,
            num: &self.levels[j].numbering,
        }
    }

    fn rule<'t>(&'t self, j: usize, top: &'t MeshStore) -> Rule<'t> {
        Rule {
            topo: self.topo(j, top),
            sharp: &self.levels[j].sharp,
            smooth: self.levels[j + 1].smooth,
            pin_corners: self.base.pin_corners,
        }
    }

    fn count(&self, j: usize) -> u32 {
        self.levels[j].numbering.vertex_count() as u32
    }
}

/// Live points whose bits differ from the gathered ones: only nodes whose position column was
/// written are compared.
fn changed_points(live: &MeshStore, gathered: &MeshStore, num: &Numbering) -> Vec<Moved> {
    let (a, b) = (&live.cols.pos, &gathered.cols.pos);
    let cfg = live.cfg;
    let per = gs_par::map(a.len(), |n| {
        if Arc::ptr_eq(&a[n], &b[n]) {
            return Vec::new();
        }
        (a[n].iter().zip(b[n].iter()).enumerate())
            .filter(|(_, (x, y))| x.bits() != y.bits())
            .map(|(l, (x, _))| {
                let (n, l) = (n as NodeId, l as u32);
                (cfg.vert(n, l), num.index(n, l), *x)
            })
            .collect()
    });
    per.concat()
}

/// Carries the live store's changes up into `top`, reading the top as it was from a share.
pub(crate) fn carry(stack: &Stack, top: &mut MeshStore) -> Carried {
    let old = top.share();
    let t = stack.levels.len() - 1;
    let top_num = Arc::clone(&stack.levels[t].numbering);
    let p = old.pos();
    let p_old = |c: u32| p.get(top_num.id(c));
    let k = stack.k;
    let mut changed = changed_points(stack.live, stack.gathered, &stack.levels[k].numbering);
    let live = stack.live.pos();
    for j in k..t {
        if changed.is_empty() {
            break;
        }
        let (rule, fine) = (stack.rule(j, &old), stack.topo(j + 1, &old));
        let coarse = stack.topo(j, &old);
        // Level j's new positions: the live store's at the edited level, the step below's
        // results above it.
        let marks_and_values = (j > k).then(|| {
            let shape = Shape {
                nodes: coarse.nodes.len(),
                topo: coarse.nodes,
            };
            let pairs: Vec<(u32, Vec3)> = changed.iter().map(|x| (x.0, x.2)).collect();
            let mut set = marks(coarse.cfg, shape.nodes);
            for x in &pairs {
                set.set(x.0 as usize, true);
            }
            (
                set,
                PerNode::from_pairs(coarse.cfg, shape.nodes, &shape.slots(), &pairs),
            )
        });
        let new = |c: u32| {
            let v = coarse.num.id(c);
            match &marks_and_values {
                None => live.get(v),
                Some((set, values)) if set.get(v as usize) => values.get(v),
                Some(_) => p_old(c),
            }
        };
        changed = step_positions(
            &rule,
            &fine,
            stack.count(j),
            &changed,
            &new,
            &p_old,
            stack.everything,
        );
    }
    let changed = protect(&old, changed, &p_old);
    let written = positions_into(top, &changed);
    top.after_moves(&written);
    Carried {
        points: changed.len(),
        values: carry_attributes(stack, top, &old, &top_num),
    }
}

/// Carried points handed to one parallel task.
const BATCH: usize = 4096;

/// The top's own protection (decision 0015): each top point's carried move scaled by `1 -
/// mask` of that point, as a brush on the top weighs it, and a point whose every face is hidden
/// left where it is. A lower level shows thinner protection (a point takes the mask of its top
/// namesake, a face is hidden only when all its top descendants are), so the lower level decides
/// only where a brush reaches, never what the top lets move (review 0004, M1). The top's columns
/// are read as they were before the carry. A part with no mask column and no hidden face costs
/// nothing, and a node with neither costs one summary read per point.
fn protect(
    top: &MeshStore,
    changed: Vec<Moved>,
    old: &(impl Fn(u32) -> Vec3 + Sync),
) -> Vec<Moved> {
    let mask = top.attr_id(&AttrDef::mask().name);
    if mask.is_none() && !top.any_hidden() {
        return changed;
    }
    let cfg = top.cfg;
    let free = |&(v, c, placed): &Moved| -> Option<Moved> {
        let n = cfg.node_of(v);
        let st = top.node_state(n);
        if st.points == Cover::None && st.mask == Cover::None {
            return Some((v, c, placed));
        }
        let (node, l) = (top.node(n), cfg.local_of(v) as usize);
        if node.hidden(l) {
            return None;
        }
        let m = match mask.and_then(|a| node.attr(a)) {
            Some(AttrVec::F32(m)) => m[l],
            _ => 0.0,
        };
        if m <= 0.0 {
            return Some((v, c, placed));
        }
        // As the kernels read it: a value that is not a finite number masks fully.
        let w = if m.is_finite() { 1.0 - m.min(1.0) } else { 0.0 };
        let from = old(c);
        let to = from + (placed - from) * w;
        (to.bits() != from.bits()).then_some((v, c, to))
    };
    let per = gs_par::map(changed.len().div_ceil(BATCH), |k| {
        let part = &changed[k * BATCH..((k + 1) * BATCH).min(changed.len())];
        part.iter().filter_map(free).collect::<Vec<Moved>>()
    });
    per.concat()
}

/// Writes carried positions (store ids of the top) into the top; returns the nodes written.
fn positions_into(top: &mut MeshStore, changed: &[Moved]) -> Vec<NodeId> {
    let cfg = top.cfg;
    let mut by_node: Vec<(NodeId, u32, Vec3)> = (changed.iter())
        .map(|&(v, _, p)| (cfg.node_of(v), cfg.local_of(v), p))
        .collect();
    by_node.sort_unstable_by_key(|x| (x.0, x.1));
    let nodes = unique(by_node.iter().map(|x| x.0).collect());
    let mut cols = select_mut(&mut top.cols.pos, &nodes);
    let mut at = 0;
    for (n, col) in nodes.iter().zip(cols.iter_mut()) {
        let col = Arc::make_mut(col);
        while at < by_node.len() && by_node[at].0 == *n {
            col[by_node[at].1 as usize] = by_node[at].2;
            at += 1;
        }
    }
    nodes
}

/// Carries every point and face attribute change up into `top`: the number of values written.
fn carry_attributes(
    stack: &Stack,
    top: &mut MeshStore,
    old: &MeshStore,
    top_num: &Numbering,
) -> usize {
    // Columns first made on the edited level (a first mask stroke, hidden faces): the top takes
    // their definitions after its own, as the live store numbers them.
    for def in stack.live.defs.iter().skip(top.defs.len()) {
        top.defs.push(def.clone());
        top.cols.attrs.push(vec![None; top.node_count()]);
    }
    let mut count = 0;
    for a in 0..top.defs.len() {
        let def = top.defs[a].clone();
        let writes = match def.domain {
            Domain::Vertex => point_values(stack, old, top_num, a, &def),
            Domain::Face => face_values(stack, a),
        };
        count += writes.len();
        attr_into(top, top_num, a, &def, &writes);
    }
    count
}

/// Node `n`'s column `a`: `None` when the node holds none, or when the store has no column `a`
/// yet (one made on the edited level after it was gathered).
fn held_at(store: &MeshStore, a: usize, n: usize) -> Option<&Arc<AttrVec>> {
    store.cols.attrs.get(a).and_then(|c| c[n].as_ref())
}

/// Whether the live store's column `a` of node `n` is the one gathered.
fn unedited(stack: &Stack, a: usize, n: usize) -> bool {
    match (held_at(stack.live, a, n), held_at(stack.gathered, a, n)) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (x, y) => x.is_none() && y.is_none(),
    }
}

/// A column's value at a store slot, zero when the node holds none.
fn value_at(store: &MeshStore, a: usize, node: usize, slot: usize, kind: AttrKind) -> AttrValue {
    held_at(store, a, node).map_or(AttrValue::zero(kind), |c| c.get(slot))
}

/// The live store's changed values of point attribute `a`, carried up: the new top value of
/// every point it changes, by canonical index.
fn point_values(
    stack: &Stack,
    old: &MeshStore,
    top_num: &Numbering,
    a: usize,
    def: &AttrDef,
) -> Vec<(u32, AttrValue)> {
    let num = &stack.levels[stack.k].numbering;
    let per = gs_par::map(stack.live.node_count(), |n| {
        if unedited(stack, a, n) {
            return Vec::new();
        }
        let slots = stack.live.cols.pos[n].len();
        (0..slots)
            .filter_map(|l| {
                let x = value_at(stack.live, a, n, l, def.kind);
                let y = value_at(stack.gathered, a, n, l, def.kind);
                (x != y).then(|| (num.index(n as NodeId, l as u32), x, y))
            })
            .collect::<Vec<_>>()
    });
    let mut exact: Vec<(u32, AttrValue, AttrValue)> = per.concat();
    exact.sort_unstable_by_key(|x| x.0);
    if exact.is_empty() {
        return Vec::new();
    }
    let t = stack.levels.len() - 1;
    let mut deltas: Vec<(u32, Delta)> = Vec::new();
    if !matches!(def.kind, AttrKind::Label | AttrKind::Flag) {
        deltas = (exact.iter())
            .map(|&(c, x, y)| (c, minus(channels(def.kind, x), channels(def.kind, y))))
            .collect();
        for j in stack.k..t {
            let fine = stack.topo(j + 1, old);
            deltas = step_deltas(&fine, stack.count(j), &deltas, stack.everything);
        }
    }
    let cfg = old.cfg;
    let old_at = |c: u32| {
        let v = top_num.id(c);
        value_at(
            old,
            a,
            cfg.node_of(v) as usize,
            cfg.local_of(v) as usize,
            def.kind,
        )
    };
    // In fixed chunks, each kept in order, so the thread count cannot change a bit.
    let chunks: Vec<&[(u32, Delta)]> = deltas.chunks(gs_par::CHUNK).collect();
    let carried = gs_par::map(chunks.len(), |k| {
        (chunks[k].iter())
            .filter(|(c, _)| exact.binary_search_by_key(c, |x| x.0).is_err())
            .map(|&(c, d)| (c, add(def, old_at(c), d)))
            .collect::<Vec<_>>()
    });
    let mut out: Vec<(u32, AttrValue)> = carried.concat();
    out.extend(exact.iter().map(|&(c, x, _)| (c, x)));
    // Two sorted runs: the stable sort finds them and merges them in one pass.
    out.sort_by_key(|x| x.0);
    out
}

/// The live store's changed values of face attribute `a`, written to every descendant top face:
/// `(top canonical face, value)`.
fn face_values(stack: &Stack, a: usize) -> Vec<(u32, AttrValue)> {
    let num = &stack.levels[stack.k].numbering;
    let kind = stack.live.defs[a].kind;
    if stack.base.is_corner_column(&stack.live.defs[a].name) {
        return Vec::new();
    }
    let canon = num.face_canon();
    let starts = num.face_starts();
    let mut out = Vec::new();
    for n in 0..stack.live.node_count() {
        if unedited(stack, a, n) {
            continue;
        }
        let faces = stack.live.cols.topo[n].faces.len();
        for slot in 0..faces {
            let x = value_at(stack.live, a, n, slot, kind);
            if x == value_at(stack.gathered, a, n, slot, kind) {
                continue;
            }
            let p = canon[starts[n] as usize + slot];
            let range = stack.base.descendants(stack.k, stack.levels.len() - 1, p);
            out.extend(range.map(|f| (f, x)));
        }
    }
    out.sort_unstable_by_key(|x| x.0);
    out
}

/// Writes attribute values (by canonical point or face of the top) into the top's column `a`,
/// allocating a node's column as zeros on first write; returns the nodes written.
fn attr_into(
    top: &mut MeshStore,
    num: &Numbering,
    a: usize,
    def: &AttrDef,
    values: &[(u32, AttrValue)],
) -> Vec<NodeId> {
    let cfg = top.cfg;
    let mut by_node: Vec<(NodeId, u32, AttrValue)> = (values.iter())
        .map(|&(c, x)| match def.domain {
            Domain::Vertex => {
                let v = num.id(c);
                (cfg.node_of(v), cfg.local_of(v), x)
            }
            Domain::Face => {
                let f = num.face(c).0;
                (cfg.face_node(f), cfg.face_local(f), x)
            }
        })
        .collect();
    by_node.sort_unstable_by_key(|x| (x.0, x.1));
    let nodes = unique(by_node.iter().map(|x| x.0).collect());
    let lens: Vec<usize> = (nodes.iter())
        .map(|&n| match def.domain {
            Domain::Vertex => top.cols.pos[n as usize].len(),
            Domain::Face => top.cols.topo[n as usize].faces.len(),
        })
        .collect();
    let mut cols = select_mut(&mut top.cols.attrs[a], &nodes);
    let mut at = 0;
    for ((n, col), len) in nodes.iter().zip(cols.iter_mut()).zip(lens) {
        let col = Arc::make_mut(col.get_or_insert_with(|| Arc::new(AttrVec::zeros(def.kind, len))));
        while at < by_node.len() && by_node[at].0 == *n {
            col.set(by_node[at].1 as usize, by_node[at].2);
            at += 1;
        }
    }
    let cols = crate::columns::ColumnSet::attr(crate::columns::AttrId(a as u8));
    top.mark(&nodes, cols);
    top.refresh_state(&nodes, cols);
    nodes
}
