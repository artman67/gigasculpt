//! Projecting one surface onto another (decision 0037): the one search that unfreezing levels
//! (decision 0030, section 4) and Project Detail share, the moves it makes, and the columns it
//! carries across.
//!
//! The search is the hybrid rule: from a point, cast along its normal both ways and take the
//! nearer hit; where neither direction hits within the reach, take the closest point within it.
//! A hit farther than twice the closest point's distance plus twice the moving surface's mean
//! edge is a ray that grazed past the near surface to a far one (decision 0037, section 1,
//! amended for D2b), and the closest point is taken instead. It reuses the spatial index: a
//! [`NodeTree`] over boxes of runs of the surface's triangles, queried in spheres that double
//! from about a triangle's size until one holds an answer at least as near as its radius (then
//! nothing outside it can be nearer: a ray hit at distance `t` lies inside every sphere of
//! radius `t` or more) or the radius passes the reach. Ties go to the lowest triangle, so each
//! point's answer depends only on the point, and the points run in parallel.

use crate::columns::{AttrDef, AttrId, AttrKind, AttrValue, AttrVec, ColumnSet, Domain};
use crate::config::{FaceId, MeshError, NodeId, VertId};
use crate::math::{
    Aabb, NO, Vec3, arity, closest_on_triangle, face_center, face_cross, face_triangles,
};
use crate::store::MeshStore;
use crate::transform::Similarity;
use crate::tree::NodeTree;
use crate::view::{PosRead, edge_neighbours};
use crate::write::NewValues;

use super::Levels;

/// Triangles per box of the search tree.
const RUN: usize = 16;

/// How far outside a triangle, in barycentric units, a ray still hits it, so a ray through a
/// shared edge cannot slip between the two triangles by rounding.
const EDGE: f32 = 1e-6;

/// Which side of the moving surface the other surface may be found on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    /// Either.
    #[default]
    Both,
    /// Along the moving point's normal.
    Outside,
    /// Against it.
    Inside,
}

impl Side {
    /// Whether an offset `s` along the normal is on an allowed side.
    fn allows(self, s: f32) -> bool {
        match self {
            Side::Both => true,
            Side::Outside => s >= 0.0,
            Side::Inside => s <= 0.0,
        }
    }
}

/// Where a point lands on a [`Surface`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    /// The point, in the surface's space.
    pub point: Vec3,
    /// The surface store's face it lies on.
    pub face: FaceId,
    /// The three points of the triangle of that face it lies on (one of the two a quad is drawn
    /// as), as the surface store's vertex ids.
    pub corners: [VertId; 3],
    /// Their weights: each 0 or more, summing to 1.
    pub weights: [f32; 3],
    /// Found by a ray along the normal (false: by the closest-point fallback).
    pub ray: bool,
}

/// A store's surface as triangles, for the search.
pub struct Surface {
    tris: Vec<[Vec3; 3]>,
    /// Each triangle's face and corners, as the store numbers them.
    ids: Vec<(FaceId, [VertId; 3])>,
    tree: NodeTree,
    /// The radius the first query tries: about one triangle's size.
    first: f32,
}

impl Surface {
    /// Every live face of `store` as triangles (a quad as the two it is drawn as), in face order;
    /// with `visible`, hidden faces are left out.
    pub fn of(store: &MeshStore, visible: bool) -> Surface {
        let (pos, cfg, shown) = (store.pos(), store.config(), store.visible());
        let mut tris = Vec::with_capacity(store.face_count() * 2);
        let mut ids = Vec::with_capacity(store.face_count() * 2);
        for n in 0..store.node_count() as NodeId {
            for (j, &f) in store.node(n).faces.iter().enumerate() {
                let id = cfg.face(n, j as u32);
                if f[0] == NO || (visible && !shown.face(id)) {
                    continue;
                }
                let p = f.map(|v| if v == NO { Vec3::ZERO } else { pos.get(v) });
                for t in face_triangles(f, |k| p[k].to_array()) {
                    tris.push(t.map(|k| p[k]));
                    ids.push((id, t.map(|k| f[k])));
                }
            }
        }
        let boxes: Vec<Aabb> = (tris.chunks(RUN))
            .map(|run| {
                let mut b = Aabb::EMPTY;
                run.iter().flatten().for_each(|&p| b.grow(p));
                b
            })
            .collect();
        let all = boxes.iter().fold(Aabb::EMPTY, |a, b| a.union(*b));
        let diagonal = (all.hi - all.lo).len2().sqrt();
        let first = diagonal / (tris.len().max(1) as f32).sqrt();
        Surface {
            tree: NodeTree::build(&boxes),
            tris,
            ids,
            first,
        }
    }

    /// Whether the surface has no triangle.
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Where a point `p` with normal `n` lands, no farther than `reach` and on `side`: the
    /// nearer ray hit along `n` both ways, if it lies no farther from `p` than twice the
    /// closest point's distance plus two `edge`s, else the closest point (decision 0037,
    /// section 1). `edge` is the moving surface's mean edge length ([`mean_edge`]), in the
    /// surface's units: a hit within two of them is within the span of the point's neighbours.
    pub fn find(&self, p: Vec3, n: Vec3, reach: f32, side: Side, edge: f32) -> Option<Landing> {
        if self.is_empty() || reach.is_nan() || reach <= 0.0 {
            return None;
        }
        let n = n.normalized();
        let slack = 2.0 * edge.max(0.0);
        let ray = (n.len2() > 0.0)
            .then(|| self.cast(p, n, reach, side))
            .flatten();
        let close = match ray {
            Some((hit, t)) if t <= slack => return Some(hit),
            Some((hit, t)) => {
                let close = self.closest(p, reach);
                if close.is_none_or(|(_, d)| t <= 2.0 * d + slack) {
                    return Some(hit);
                }
                close
            }
            None => self.closest(p, reach),
        };
        close
            .map(|(c, _)| c)
            .filter(|c| side.allows((c.point - p).dot(n)))
    }

    /// The first radius of a doubling search.
    fn start(&self, reach: f32) -> f32 {
        self.first.min(reach).max(reach / 1024.0)
    }

    /// The triangles of the boxes a sphere touches, with their numbers.
    fn near(&self, p: Vec3, r: f32) -> impl Iterator<Item = (usize, &[Vec3; 3])> {
        let (runs, _) = self.tree.query(p, r);
        runs.into_iter().flat_map(move |run| {
            let start = run as usize * RUN;
            let end = (start + RUN).min(self.tris.len());
            (start..end).zip(&self.tris[start..end])
        })
    }

    fn landing(&self, t: usize, (point, weights): (Vec3, [f32; 3]), ray: bool) -> Landing {
        let (face, corners) = self.ids[t];
        Landing {
            point,
            face,
            corners,
            weights,
            ray,
        }
    }

    /// The nearest hit of the line through `p` along unit `n`, within `reach` both ways, and
    /// its distance.
    fn cast(&self, p: Vec3, n: Vec3, reach: f32, side: Side) -> Option<(Landing, f32)> {
        let mut r = self.start(reach);
        loop {
            let mut best: Option<(f32, usize, Vec3, [f32; 3])> = None;
            for (t, tri) in self.near(p, r) {
                let Some((s, w)) = crossing(p, n, tri) else {
                    continue;
                };
                let d = s.abs();
                if d > reach || !side.allows(s) {
                    continue;
                }
                if best.is_none_or(|b| (d.total_cmp(&b.0).then(t.cmp(&b.1))).is_lt()) {
                    best = Some((d, t, at(tri, w), w));
                }
            }
            match best {
                Some((d, t, q, w)) if d <= r => return Some((self.landing(t, (q, w), true), d)),
                _ if r >= reach => return None,
                _ => r = (r * 2.0).min(reach),
            }
        }
    }

    /// The closest point to `p` no farther than `reach`, and its distance. A triangle outside a
    /// sphere's boxes is farther than its radius, so once the nearest triangle found inside is
    /// within the radius it is the nearest of all.
    fn closest(&self, p: Vec3, reach: f32) -> Option<(Landing, f32)> {
        let mut r = self.start(reach);
        loop {
            let mut best: Option<(f32, usize, Vec3, [f32; 3])> = None;
            for (t, tri) in self.near(p, r) {
                let (q, w) = closest_on_triangle(p, tri);
                let d2 = (q - p).len2();
                if best.is_none_or(|b| (d2.total_cmp(&b.0).then(t.cmp(&b.1))).is_lt()) {
                    best = Some((d2, t, q, w));
                }
            }
            match best {
                Some((d2, t, q, w)) if d2 <= r * r => {
                    let ok = d2 <= reach * reach;
                    return ok.then(|| (self.landing(t, (q, w), false), d2.sqrt()));
                }
                _ if r >= reach => return None,
                _ => r = (r * 2.0).min(reach),
            }
        }
    }
}

/// The point of triangle `t` with weights `w`.
fn at([a, b, c]: &[Vec3; 3], w: [f32; 3]) -> Vec3 {
    *a + (*b - *a) * w[1] + (*c - *a) * w[2]
}

/// Where the line through `p` along `n` crosses triangle `t` (Moller and Trumbore): the offset
/// along `n` and the weights, clamped onto the triangle. `None` when it misses or runs parallel.
fn crossing(p: Vec3, n: Vec3, t: &[Vec3; 3]) -> Option<(f32, [f32; 3])> {
    let [a, b, c] = *t;
    let (e1, e2) = (b - a, c - a);
    let pv = n.cross(e2);
    let det = e1.dot(pv);
    if det.abs() <= f32::MIN_POSITIVE || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    let tv = p - a;
    let u = tv.dot(pv) * inv;
    let qv = tv.cross(e1);
    let v = n.dot(qv) * inv;
    if !(u >= -EDGE && v >= -EDGE && u + v <= 1.0 + EDGE) {
        return None;
    }
    let s = e2.dot(qv) * inv;
    let (u, v) = (u.max(0.0), v.max(0.0));
    let sum = u + v;
    let (u, v) = if sum > 1.0 {
        (u / sum, v / sum)
    } else {
        (u, v)
    };
    Some((s, [1.0 - u - v, u, v]))
}

/// How the points of a store move onto a surface.
#[derive(Clone, Copy)]
pub struct Projection<'a> {
    pub onto: &'a Surface,
    /// How far a point may move, in the surface's units.
    pub reach: f32,
    pub side: Side,
    /// From the moving store's space into the surface's.
    pub to: Similarity,
    /// Each point moves `1 - mask` of the way, and points every face around which is hidden
    /// stay (Project Detail). Off: every live point may move (unfreeze).
    pub guarded: bool,
    /// 0 to 100: how much to smooth the moves, as voxel remesh's `blur`.
    pub blur: f32,
    /// Keep where each point landed, for [`carry`].
    pub keep: bool,
}

impl<'a> Projection<'a> {
    /// Every live point, both sides, in one space, no blur: unfreeze's projection.
    pub fn plain(onto: &'a Surface, reach: f32) -> Projection<'a> {
        Projection {
            onto,
            reach,
            side: Side::Both,
            to: Similarity::IDENTITY,
            guarded: false,
            blur: 0.0,
            keep: false,
        }
    }
}

/// What a projection did.
#[derive(Default)]
pub struct Projected {
    /// Points that moved.
    pub moved: usize,
    /// Points that looked and found nothing within reach.
    pub out_of_reach: usize,
    /// The largest move, in the moving store's units.
    pub largest: f32,
    /// The nodes whose points moved, ascending.
    pub nodes: Vec<NodeId>,
    /// Where each point landed, by node and slot, when kept.
    pub landed: Vec<Vec<Option<Landing>>>,
}

/// How much of its move a point makes: `None` when it may not move at all (a dead slot, or,
/// guarded, a point every face around which is hidden or one fully masked), else its mask (0
/// when unguarded).
fn held(
    store: &MeshStore,
    how: &Projection,
    mask: Option<AttrId>,
    n: usize,
    l: usize,
) -> Option<f32> {
    let node = store.node(n as NodeId);
    if node.pos[l].is_dead() {
        return None;
    }
    if !how.guarded {
        return Some(0.0);
    }
    let m = mask.map_or(0.0, |m| store.attr_reader(m).scalar(node.base + l as u32));
    (!node.hidden(l) && m < 1.0).then_some(m.max(0.0))
}

/// Where each point of `store` that may move lands on `how.onto`, by node and slot, without
/// moving it: [`Surface::find`] from the point along its normal, with `store`'s
/// [`mean_edge`] as the moving surface's edge.
pub fn landings(store: &MeshStore, how: &Projection) -> Vec<Vec<Option<Landing>>> {
    let mask = store.attr_id(&AttrDef::mask().name);
    let edge = how.to.length(mean_edge(store));
    gs_par::map(store.node_count(), |n| {
        let node = store.node(n as NodeId);
        (node.pos.iter().zip(node.nrm).enumerate())
            .map(|(l, (&p, &nrm))| {
                held(store, how, mask, n, l)?;
                let (q, d) = (how.to.point(p), how.to.direction(nrm));
                (how.onto).find(q, d, how.reach, how.side, edge)
            })
            .collect()
    })
}

/// The mean length of the edges of `store`'s live faces, each face's own edges counted (an
/// edge two faces share counts twice): the moving surface's edge that the guard in
/// [`Surface::find`] reads. One length for the whole surface, not each point's own: where a
/// projection pulled points into a crease the edges across it stretch, and a guard read there
/// let spikes through (decision 0037, section 1, amended for D2b). Summed by node and then in
/// node order, so it is the same at any thread count.
pub fn mean_edge(store: &MeshStore) -> f32 {
    let pos = store.pos();
    let sums: Vec<(f64, usize)> = gs_par::map(store.node_count(), |n| {
        let live = store.node(n as NodeId).faces.iter().filter(|f| f[0] != NO);
        live.fold((0.0, 0), |(sum, count), &f| {
            let k = arity(f);
            let edges = (0..k).map(|i| (pos.get(f[(i + 1) % k]) - pos.get(f[i])).len2());
            let length: f64 = edges.map(|e| f64::from(e.sqrt())).sum();
            (sum + length, count + k)
        })
    });
    let (sum, count) = (sums.into_iter()).fold((0.0, 0), |(s, c), (a, b)| (s + a, c + b));
    (sum / count.max(1) as f64) as f32
}

/// Moves the points of `store` onto `how.onto` (decision 0037): each lands where
/// [`landings`] says, or stays.
pub fn project(store: &mut MeshStore, how: &Projection) -> Projected {
    let back = how.to.inverse();
    let mask = store.attr_id(&AttrDef::mask().name);
    let landed = landings(store, how);
    let mut done = Projected::default();
    for (n, hits) in landed.iter().enumerate() {
        let looked = (0..hits.len()).filter(|&l| held(store, how, mask, n, l).is_some());
        done.out_of_reach += looked.filter(|&l| hits[l].is_none()).count();
    }
    let mut moves: Vec<Vec<Option<Vec3>>> = gs_par::map(store.node_count(), |n| {
        let node = store.node(n as NodeId);
        (node.pos.iter().zip(&landed[n]))
            .map(|(&p, hit)| hit.map(|h| back.point(h.point) - p))
            .collect()
    });
    if how.blur > 0.0 {
        moves = blurred(store, moves, how.blur);
    }
    // The landing itself when nothing scales the move, so an unguarded, unblurred projection
    // lands exactly; else the point plus its move, weighted by `1 - mask`.
    let placed: Vec<Vec<Option<Vec3>>> = gs_par::map(store.node_count(), |n| {
        let node = store.node(n as NodeId);
        (node.pos.iter().enumerate())
            .map(|(l, &p)| {
                let m = held(store, how, mask, n, l)?;
                let q = match (landed[n][l], moves[n][l]) {
                    (Some(h), _) if how.blur <= 0.0 && m <= 0.0 => back.point(h.point),
                    (_, Some(d)) => p + d * (1.0 - m),
                    _ => return None,
                };
                (q.bits() != p.bits()).then_some(q)
            })
            .collect()
    });
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId)
        .filter(|&n| placed[n as usize].iter().any(Option::is_some))
        .collect();
    for (n, to) in placed.iter().enumerate() {
        let from = store.node(n as NodeId).pos;
        for (p, q) in from.iter().zip(to) {
            if let Some(q) = q {
                done.moved += 1;
                done.largest = done.largest.max((*q - *p).len2().sqrt());
            }
        }
    }
    store.write(&nodes, ColumnSet::POSITION).each(|node| {
        let to = &placed[node.id() as usize];
        for (p, q) in node.positions().iter_mut().zip(to) {
            if let Some(q) = q {
                *p = *q;
            }
        }
    });
    if how.keep {
        done.landed = landed;
    }
    done.nodes = nodes;
    done
}

/// Moves smoothed over the store's edges, as voxel remesh smooths its projection: each pass
/// takes every move half way to the mean of its neighbours' (a point that found nothing counts
/// as a move of zero); `blur / 4` passes, rounded up.
fn blurred(store: &MeshStore, moves: Vec<Vec<Option<Vec3>>>, blur: f32) -> Vec<Vec<Option<Vec3>>> {
    let passes = (blur.clamp(0.0, 100.0) / 4.0).ceil() as usize;
    let (cfg, topo) = (store.config(), store.topo());
    let mut moves = moves;
    for _ in 0..passes {
        let get = |v: VertId| {
            let (n, l) = (cfg.node_of(v) as usize, cfg.local_of(v) as usize);
            moves[n][l].unwrap_or(Vec3::ZERO)
        };
        moves = gs_par::map(store.node_count(), |n| {
            let node = store.node(n as NodeId);
            let mut around = Vec::new();
            (node.pos.iter().enumerate())
                .map(|(l, p)| {
                    if p.is_dead() {
                        return None;
                    }
                    let v = node.base + l as u32;
                    edge_neighbours(&topo, v, |_| true, &mut around);
                    let mut mean = Vec3::ZERO;
                    for &(u, _) in &around {
                        mean += get(u) * (1.0 / around.len() as f32);
                    }
                    Some(get(v) * 0.5 + mean * 0.5)
                })
                .collect()
        });
    }
    moves
}

/// Copies column `def` of `source` onto `store`, where a projection of `store` onto `source`'s
/// surface landed (`landed`, as [`project`] kept it), by the column's kind (decision 0037,
/// section 3): a point column from the landing's three corners and weights, a face column from
/// the source face the face's centre lands on, searched now along the face's normal. Points and
/// faces with no landing keep their value; a carried mask is settled as every mask writer
/// stores it. Refuses a column `store` holds under another definition. Returns the nodes
/// written.
pub fn carry(
    store: &mut MeshStore,
    how: &Projection,
    source: &MeshStore,
    def: &AttrDef,
    landed: &[Vec<Option<Landing>>],
) -> Result<Vec<NodeId>, MeshError> {
    let Some(from) = source.attr_id(&def.name) else {
        return Ok(Vec::new());
    };
    store.attr_fits(def)?;
    let (read, zero) = (source.attr_reader(from), AttrValue::zero(def.kind));
    let own = store.attr_id(&def.name).map(|a| store.attr_reader(a));
    // A value carried across, or the store's own where nothing landed.
    let value = |found: Option<AttrValue>, id: u32| {
        found.unwrap_or_else(|| own.and_then(|o| o.get(id)).unwrap_or(zero))
    };
    let fresh: Vec<NewValues> = match def.domain {
        Domain::Vertex => gs_par::map(store.node_count(), |n| {
            let node = store.node(n as NodeId);
            let vals = (0..node.pos.len()).map(|l| {
                let hit = landed.get(n).and_then(|h| h.get(l).copied().flatten());
                let found = hit.map(|h| {
                    let vals = h.corners.map(|c| read.get(c).unwrap_or(zero));
                    AttrValue::blend_weighted(def, &vals, &h.weights)
                });
                value(found, node.base + l as u32)
            });
            NewValues::Values(column(def.kind, vals))
        }),
        Domain::Face => {
            let (pos, shown, to) = (store.pos(), store.visible(), how.to);
            let edge = to.length(mean_edge(store));
            gs_par::map(store.node_count(), |n| {
                let node = store.node(n as NodeId);
                let vals = node.faces.iter().enumerate().map(|(j, &f)| {
                    let id = node.first_face + j as u32;
                    if f[0] == NO || (how.guarded && !shown.face(id)) {
                        return value(None, id);
                    }
                    let (c, d) = (
                        face_center(f, |v| pos.get(v)),
                        face_cross(f, |v| pos.get(v)),
                    );
                    let (q, d) = (to.point(c), to.direction(d));
                    let hit = (how.onto).find(q, d, how.reach, how.side, edge);
                    value(hit.map(|h| read.get(h.face).unwrap_or(zero)), id)
                });
                NewValues::Values(column(def.kind, vals))
            })
        }
    };
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    Ok(store.put_column(def.clone(), &nodes, fresh))
}

/// A node's column of `kind` from its values.
fn column(kind: AttrKind, vals: impl Iterator<Item = AttrValue>) -> AttrVec {
    let vals: Vec<AttrValue> = vals.collect();
    let mut out = AttrVec::zeros(kind, vals.len());
    for (i, v) in vals.into_iter().enumerate() {
        out.set(i, v);
    }
    out
}

/// How many divisions bring a base with `quads` quads and `triangles` triangles closest to
/// `target` faces, from 0 to `most`; a tie takes fewer. A division makes a triangle three
/// quads and a quad four. None at all when the base is already the closest: a base as dense as
/// the old top or denser is the top itself (review 0007, m3: it was divided once all the same,
/// and came back 4 to 108 times the old top's faces).
fn divisions_toward(quads: usize, triangles: usize, target: usize, most: usize) -> usize {
    let mut faces = quads + triangles;
    let mut best = (faces.abs_diff(target), 0);
    for k in 1..=most {
        faces = match k {
            1 => 4 * quads + 3 * triangles,
            _ => faces.saturating_mul(4),
        };
        best = best.min((faces.abs_diff(target), k));
        if faces > target.saturating_mul(2) {
            break;
        }
    }
    best.1
}

impl Levels {
    /// `base`, a stack of one level, divided smooth [`divisions_toward`] `target` faces, at
    /// most `most` times, with every point of the new top projected by `how` after each
    /// division, so a coarse base's smoothing does not pull the levels inside (decision 0030,
    /// section 4). With no division, the base itself is the top and is projected once. Returns
    /// the stack, at its top, and what the last projection did.
    pub fn divided_onto(
        mut base: Levels,
        how: &Projection,
        target: usize,
        most: usize,
    ) -> Result<(Levels, Projected), MeshError> {
        let (mut quads, mut triangles) = (0, 0);
        for n in 0..base.top().node_count() as NodeId {
            for &f in base.top().node(n).faces.iter().filter(|f| f[0] != NO) {
                if arity(f) == 4 {
                    quads += 1;
                } else {
                    triangles += 1;
                }
            }
        }
        let k = divisions_toward(quads, triangles, target, most);
        let mut done = Projected::default();
        for _ in 0..k {
            base.divide(true)?;
            done = project(base.store_mut(), how);
        }
        if k == 0 {
            done = project(base.store_mut(), how);
        }
        Ok((base, done))
    }

    /// The stack unfreeze builds (decision 0030, sections 3 and 4): `base` divided toward the
    /// face count of `record`'s top (with its pending edits carried up), at most `record`'s
    /// level count plus two times, every point of the new top projected onto that top within
    /// `reach` (scene units) after each division by the hybrid rule. Every point column of
    /// that top (the color, the mask, the UV hints) then comes from where each point landed,
    /// by [`carry`] (decision 0030 as amended 2026-10-13), so paint finer than the base comes
    /// back with the detail; a column the old top does not hold stays as the base had it, and
    /// mirror maps, point ids meaning nothing on the new top, are left out. Returns the stack,
    /// at its top, and how many points the last projection moved.
    pub fn unfrozen(
        base: Levels,
        record: &Levels,
        reach: f32,
    ) -> Result<(Levels, usize), MeshError> {
        let mut source = record.share();
        source.set(source.top_level())?;
        let surface = Surface::of(source.top(), false);
        let how = Projection {
            keep: true,
            ..Projection::plain(&surface, reach)
        };
        let target = source.top().face_count();
        let (mut levels, done) = Levels::divided_onto(base, &how, target, record.count() + 2)?;
        // Each by its kind (decision 0037, section 3): scalars by weighted mean, colors per
        // channel rounded, labels and flags by the largest weight, settled by `AttrDef`'s range.
        let point = |d: &&AttrDef| d.domain == Domain::Vertex && !crate::mirror::is_map(&d.name);
        for def in source.top().attrs().iter().filter(point) {
            carry(levels.store_mut(), &how, source.top(), def, &done.landed)?;
        }
        Ok((levels, done.moved))
    }
}
