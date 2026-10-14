//! Building a level on top of another without a global sort (decision 0014, section 4).
//!
//! The new level's node cut follows the level below: each coarse node gathers its own points,
//! the edge points whose first use (in canonical face order) is a face it stores, and the face
//! points of the faces it stores (a face's first-used edge points and its center are numbered
//! together, see `Step::made_by`); each group is sorted by Morton code within its own box and
//! cut into nodes of at most `fill` points. A coarse face's children go to the node of its face
//! point, which keeps every node within its face slots. Every pass runs in parallel per coarse
//! or fine node, except two scatters that invert a numbering. The new level's numbering falls out
//! of the cut.

use std::sync::Arc;

use crate::build::topology;
use crate::columns::Columns;
use crate::config::{MeshError, NodeConfig, NodeId};
use crate::math::{Aabb, NO, Vec3, arity, morton};
use crate::numbering::Numbering;
use crate::store::MeshStore;
use crate::subdivide::{Step, children, ends};

/// The level below, in canonical terms, with its step table.
pub(crate) struct Coarse<'a> {
    pub store: &'a MeshStore,
    pub num: &'a Numbering,
    /// Its faces as canonical indices, in canonical order.
    pub faces: &'a [[u32; 4]],
    pub step: &'a Step,
    /// [`Step::sources`] of `faces`.
    pub src: &'a [u32],
    /// Each store face's canonical index, by dense store index (`Numbering::face_canon`).
    pub fcanon: &'a [u32],
}

impl Coarse<'_> {
    /// Where the children of each coarse face start among the fine faces: a prefix sum of the
    /// corner counts (one more entry at the end).
    pub fn child_starts(&self) -> Vec<u32> {
        let mut s = Vec::with_capacity(self.faces.len() + 1);
        let mut at = 0u32;
        for &f in self.faces {
            s.push(at);
            at += arity(f) as u32;
        }
        s.push(at);
        s
    }

    /// The canonical faces stored in coarse node `n`, in storage order.
    fn stored_faces(&self, n: usize) -> &[u32] {
        let starts = self.num.face_starts();
        &self.fcanon[starts[n] as usize..starts[n + 1] as usize]
    }
}

/// Each corner slot's first use (see `Step::numbered`), from the coarse store's rings: the
/// lowest slot, among the faces around the edge's lower end, that walks the same edge. In
/// parallel over slots.
pub(crate) fn first_uses(
    store: &MeshStore,
    num: &Numbering,
    faces: &[[u32; 4]],
    fcanon: &[u32],
) -> Vec<u32> {
    let cfg = store.config();
    let starts = num.face_starts();
    let topo = store.topo();
    gs_par::map(faces.len() * 4, |s| {
        let f = faces[s / 4];
        if s % 4 >= arity(f) {
            return NO;
        }
        let edge = ends(f, s % 4);
        let a = num.id(edge.0);
        let mut first = s as u32;
        for &g in crate::view::TopoRead::ring(&topo, a) {
            let c = fcanon[(starts[cfg.face_node(g) as usize] + cfg.face_local(g)) as usize];
            let other = faces[c as usize];
            if let Some(i) = (0..arity(other)).find(|&i| ends(other, i) == edge) {
                first = first.min(c * 4 + i as u32);
            }
        }
        first
    })
}

/// What the cut placed where, for filling attribute columns afterwards.
pub(crate) struct Layout {
    /// Per node, the canonical fine point of each slot.
    pub members: Vec<Vec<u32>>,
    /// Per node, each face slot's coarse face (canonical) and which child of it it is.
    pub parents: Vec<Vec<(u32, u8)>>,
}

/// One coarse node's group: its points sorted by Morton code in the group's box (ties by
/// canonical index) and cut into parts of at most `fill`, each part's points and positions.
fn group(
    c: &Coarse,
    n: usize,
    pos: &(dyn Fn(u32) -> Vec3 + Sync),
    fill: usize,
) -> Vec<(Vec<u32>, Vec<Vec3>)> {
    let slots = c.store.node(n as NodeId).pos.len() as u32;
    let mut members: Vec<u32> = (0..slots).map(|l| c.num.index(n as NodeId, l)).collect();
    for &f in c.stored_faces(n) {
        members.extend(c.step.made_by(f as usize));
    }
    let pts: Vec<(u32, Vec3)> = members.iter().map(|&v| (v, pos(v))).collect();
    let b = pts.iter().fold(Aabb::EMPTY, |mut b, x| {
        b.grow(x.1);
        b
    });
    let mut keyed: Vec<(u64, u32, Vec3)> = pts.iter().map(|&(v, p)| (morton(p, b), v, p)).collect();
    keyed.sort_unstable_by_key(|x| (x.0, x.1));
    let parts = keyed.len().div_ceil(fill);
    (0..parts)
        .map(|j| {
            let part = &keyed[j * keyed.len() / parts..(j + 1) * keyed.len() / parts];
            (
                part.iter().map(|x| x.1).collect(),
                part.iter().map(|x| x.2).collect(),
            )
        })
        .collect()
}

/// The fine level over `c`, with each point placed at `pos(canonical index)`: node columns
/// (positions, zero normals, topology; no attribute columns), its numbering, and the layout.
pub(crate) fn cut(
    c: &Coarse,
    pos: &(dyn Fn(u32) -> Vec3 + Sync),
) -> Result<(Columns, Numbering, Layout), MeshError> {
    let cfg = c.store.config();
    let fill = cfg.fill() as usize;
    let groups = gs_par::map(c.store.node_count(), |n| group(c, n, pos, fill));
    // Node `first[g] + j` holds part `j` of group `g`.
    let mut first = Vec::with_capacity(groups.len() + 1);
    let mut nodes = 0usize;
    for g in &groups {
        first.push(nodes);
        nodes += g.len();
    }
    first.push(nodes);
    if nodes > cfg.max_nodes() {
        return Err(MeshError::TooLarge(nodes));
    }
    let mut cols = Columns::default();
    let mut members = Vec::with_capacity(nodes);
    for (points, positions) in groups.into_iter().flatten() {
        cols.nrm.push(Arc::new(vec![Vec3::ZERO; points.len()]));
        cols.pos.push(Arc::new(positions));
        members.push(points);
    }
    let mut ids = vec![NO; c.step.count as usize];
    for (k, m) in members.iter().enumerate() {
        for (l, &v) in m.iter().enumerate() {
            ids[v as usize] = cfg.vert(k as NodeId, l as u32);
        }
    }
    let (faces, face_ids, parents) = place_faces(cfg, c, &ids, &first);
    let slots: Vec<usize> = members.iter().map(Vec::len).collect();
    let counts: Vec<usize> = faces.iter().map(Vec::len).collect();
    cols.topo = topology(cfg, &slots, faces)?;
    let numbering = Numbering::from_layout(cfg, ids, &members, face_ids, &counts);
    Ok((cols, numbering, Layout { members, parents }))
}

/// Faces node after node: each coarse face's children in the node of its face point (one of
/// the parts its coarse node's group was cut into), in the coarse node's storage order. Returns
/// each fine node's faces (store ids), each fine face's store id by canonical index, and each
/// fine node's face parents.
#[expect(
    clippy::type_complexity,
    reason = "the three outputs of one placement, used once by the caller"
)]
fn place_faces(
    cfg: NodeConfig,
    c: &Coarse,
    ids: &[u32],
    first: &[usize],
) -> (Vec<Vec<[u32; 4]>>, Vec<u32>, Vec<Vec<(u32, u8)>>) {
    // Per coarse node, its faces bucketed by the part their center went to.
    let per_part = gs_par::map(first.len() - 1, |n| {
        let mut parts: Vec<Vec<u32>> = vec![Vec::new(); first[n + 1] - first[n]];
        for &f in c.stored_faces(n) {
            let node = cfg.node_of(ids[c.step.center[f as usize] as usize]) as usize;
            parts[node - first[n]].push(f);
        }
        parts
    });
    let lists: Vec<Vec<u32>> = per_part.into_iter().flatten().collect();
    let built = gs_par::map(lists.len(), |k| {
        let mut faces = Vec::new();
        let mut parents = Vec::new();
        for &fi in &lists[k] {
            let f = c.faces[fi as usize];
            let (kids, n) = children(f, c.step.edges(fi as usize), c.step.center[fi as usize]);
            for (j, kid) in kids[..n].iter().enumerate() {
                faces.push(kid.map(|v| ids[v as usize]));
                parents.push((fi, j as u8));
            }
        }
        (faces, parents)
    });
    // The store id of each coarse face's first child, then every fine face's, in parallel:
    // children are stored one after another.
    let mut first_child = vec![NO; c.faces.len()];
    for (k, (_, parents)) in built.iter().enumerate() {
        for (j, &(fi, child)) in parents.iter().enumerate() {
            if child == 0 {
                first_child[fi as usize] = cfg.face(k as NodeId, j as u32);
            }
        }
    }
    let cstart = c.child_starts();
    let quads = cstart.last().copied() == Some(4 * c.faces.len() as u32);
    let face_ids = gs_par::map(cstart[cstart.len() - 1] as usize, |f| {
        let f = f as u32;
        let parent = if quads {
            f / 4
        } else {
            cstart.partition_point(|&s| s <= f) as u32 - 1
        };
        first_child[parent as usize] + (f - cstart[parent as usize])
    });
    let (faces, parents) = built.into_iter().unzip();
    (faces, face_ids, parents)
}
