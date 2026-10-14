//! Spatial sort and node cut (ARCHITECTURE 3.1): import, subdivide and re-sort all come here.
//!
//! Vertices are sorted by Morton code and cut into nodes, so a node owns a contiguous id range
//! of nearby vertices. A face goes to the node owning its lowest vertex id, rotated so that
//! vertex comes first; faces are then sorted, so each node's faces are in ascending order too.
//! Each vertex gets a ring: every face that uses it, ascending, including faces stored in
//! neighbouring nodes.
//!
//! A node holds at most `fill` vertices and at most `4 × fill` faces, the same share of its
//! face slots as `fill` is of its vertex slots (decision 0010). Ordinary meshes have about two
//! faces per vertex, so cutting every `fill` vertices meets both limits and is all that runs.
//! Only when a node of that cut owns too many faces (duplicated faces, very high valence) are
//! the faces each vertex owns counted, and the nodes cut again in Morton order, each ending at
//! `fill` vertices or before the vertex whose faces would pass the face limit.

use std::sync::Arc;

use crate::columns::{AttrDef, AttrVec, Columns, Domain, NodeTopo, ring_head};
use crate::config::{MeshError, NodeConfig};
use crate::math::{Aabb, NO, Vec3, arity, morton};

/// A plain indexed mesh in any vertex order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Flat {
    pub pos: Vec<Vec3>,
    pub faces: Vec<[u32; 4]>,
    /// One entry per registered attribute: `None` when absent, else one value per vertex or
    /// per face (by the attribute's domain).
    pub attrs: Vec<Option<AttrVec>>,
}

/// Source vertex indices in Morton order.
fn morton_order(pos: &[Vec3]) -> Vec<u32> {
    let b = gs_par::reduce(
        pos.len(),
        || Aabb::EMPTY,
        |mut b, i| {
            b.grow(pos[i]);
            b
        },
        Aabb::union,
    );
    let mut keys = gs_par::map(pos.len(), |i| (morton(pos[i], b), i as u32));
    gs_par::sort(&mut keys);
    keys.into_iter().map(|k| k.1).collect()
}

/// Faces with new vertex ids, lowest id first, sorted, each with its source face index.
fn sorted_faces(src: &[[u32; 4]], remap: &[u32]) -> Vec<([u32; 4], u32)> {
    let mut faces = gs_par::map(src.len(), |i| {
        let f = src[i];
        let ar = arity(f);
        let mut g = f;
        for x in &mut g[..ar] {
            *x = remap[*x as usize];
        }
        let lowest = (0..ar).min_by_key(|&k| g[k]).unwrap_or(0);
        g[..ar].rotate_left(lowest);
        (g, i as u32)
    });
    gs_par::sort(&mut faces);
    faces
}

/// Packed ring headers and ring pool of node `k`, which has `len` vertex slots and stores
/// `own` (as store vertex ids, in slot order), from its own faces and `foreign` (faces stored
/// elsewhere that use its vertices, as `(face id, local slot)`).
fn node_rings(
    cfg: NodeConfig,
    k: u32,
    len: usize,
    own: &[[u32; 4]],
    foreign: &[(u32, u32)],
) -> Result<(Vec<u32>, Vec<u32>), MeshError> {
    // (local slot, face id) for every face using one of this node's vertices.
    let mut entries: Vec<(u32, u32)> = foreign.iter().map(|&(f, l)| (l, f)).collect();
    for (j, fc) in own.iter().enumerate() {
        for &v in fc[..arity(*fc)].iter().filter(|&&v| cfg.node_of(v) == k) {
            entries.push((cfg.local_of(v), cfg.face(k, j as u32)));
        }
    }
    pack_rings(len, entries)
}

/// The packed ring headers and ring pool of a node with `len` vertex slots, from every
/// `(local slot, face id)` that uses one of them (any order, each slot below `len`): each ring
/// sorted by face id, the pool in slot order, as [`crate::columns::NodeTopo::compact`] leaves
/// it.
pub(crate) fn pack_rings(
    len: usize,
    mut entries: Vec<(u32, u32)>,
) -> Result<(Vec<u32>, Vec<u32>), MeshError> {
    entries.sort_unstable();
    let mut ring = vec![0u32; len];
    let mut start = 0;
    for (l, head) in ring.iter_mut().enumerate() {
        let count = entries[start..]
            .iter()
            .take_while(|e| e.0 as usize == l)
            .count();
        if count > 255 {
            return Err(MeshError::Input(format!(
                "a vertex is used by {count} faces (max 255)"
            )));
        }
        *head = ring_head(start, count);
        start += count;
    }
    Ok((ring, entries.into_iter().map(|e| e.1).collect()))
}

/// Seam incidences grouped by owner node: `foreign[k]` lists `(face id, local slot)` for faces
/// stored outside node `k` that use one of its vertices, in face id order.
fn foreign_refs(cfg: NodeConfig, faces: &[Vec<[u32; 4]>]) -> Vec<Vec<(u32, u32)>> {
    let nn = faces.len();
    let per = gs_par::map(nn, |k| {
        let mut out = Vec::new();
        for (j, fc) in faces[k].iter().enumerate() {
            for &v in &fc[..arity(*fc)] {
                if cfg.node_of(v) as usize != k {
                    out.push((
                        cfg.node_of(v),
                        cfg.face(k as u32, j as u32),
                        cfg.local_of(v),
                    ));
                }
            }
        }
        out
    });
    let mut out = vec![Vec::new(); nn];
    for (n, f, l) in per.into_iter().flatten() {
        out[n as usize].push((f, l));
    }
    out
}

/// Every node's topology: its faces (`faces[k]`, as store vertex ids, in storage order), and
/// the ring of each of its `slots[k]` vertex slots, built from every face that uses it. The one
/// place nodes get their topology, for an import's cut and a detail level's (decision 0014).
/// Fails if a vertex is used by more faces than a ring holds.
pub(crate) fn topology(
    cfg: NodeConfig,
    slots: &[usize],
    faces: Vec<Vec<[u32; 4]>>,
) -> Result<Vec<Arc<NodeTopo>>, MeshError> {
    let foreign = foreign_refs(cfg, &faces);
    let rings = gs_par::map(slots.len(), |k| {
        node_rings(cfg, k as u32, slots[k], &faces[k], &foreign[k])
    });
    (rings.into_iter().zip(faces))
        .map(|(r, own)| {
            let (ring, pool) = r?;
            Ok(Arc::new(NodeTopo {
                faces: own,
                ring,
                pool,
                ..NodeTopo::default()
            }))
        })
        .collect()
}

/// Gather one attribute's values for a node, by vertex or face source index.
pub(crate) fn gather(values: &AttrVec, src: impl Iterator<Item = u32>) -> AttrVec {
    match values {
        AttrVec::F32(v) => AttrVec::F32(src.map(|i| v[i as usize]).collect()),
        AttrVec::U32(v) => AttrVec::U32(src.map(|i| v[i as usize]).collect()),
        AttrVec::Bits(b) => AttrVec::Bits(src.map(|i| b.get(i as usize)).collect()),
    }
}

/// Where [`build`] put the source mesh: each source vertex's id, and the faces as stored,
/// sorted, each with its source index.
pub(crate) struct Placed {
    pub remap: Vec<u32>,
    pub faces: Vec<([u32; 4], u32)>,
}

/// One cut of the Morton order into nodes: the source vertices placed, and the faces sorted.
struct Cut {
    /// Node `k` holds the vertices of Morton ranks `starts[k]..starts[k + 1]`.
    starts: Vec<usize>,
    remap: Vec<u32>,
    faces: Vec<([u32; 4], u32)>,
    /// Node `k` stores faces `fstart[k]..fstart[k + 1]`.
    fstart: Vec<usize>,
}

impl Cut {
    fn new(cfg: NodeConfig, order: &[u32], starts: Vec<usize>, src: &[[u32; 4]]) -> Cut {
        let mut remap = vec![0u32; order.len()];
        for (k, w) in starts.windows(2).enumerate() {
            for s in w[0]..w[1] {
                remap[order[s] as usize] = cfg.vert(k as u32, (s - w[0]) as u32);
            }
        }
        let faces = sorted_faces(src, &remap);
        let fstart = (0..starts.len())
            .map(|k| faces.partition_point(|x| (cfg.node_of(x.0[0]) as usize) < k))
            .collect();
        Cut {
            starts,
            remap,
            faces,
            fstart,
        }
    }

    fn nodes(&self) -> usize {
        self.starts.len() - 1
    }

    /// The first node storing more than `most` faces.
    fn crowded(&self, most: usize) -> Option<usize> {
        (0..self.nodes()).find(|&k| self.fstart[k + 1] - self.fstart[k] > most)
    }

    /// Cut again by vertices and faces: walking the Morton order, a node ends at `fill`
    /// vertices or before the vertex whose own faces would take it past `most` faces.
    fn by_faces(self, cfg: NodeConfig, order: &[u32], src: &[[u32; 4]], most: usize) -> Cut {
        // A face belongs to the vertex of lowest rank among its corners; ids follow ranks.
        let rank = |v: u32| self.starts[cfg.node_of(v) as usize] + cfg.local_of(v) as usize;
        let mut owned = vec![0usize; order.len()];
        for (f, _) in &self.faces {
            owned[rank(f[0])] += 1;
        }
        let fill = cfg.fill() as usize;
        let mut starts = vec![0];
        let (mut verts, mut faces) = (0, 0);
        for (s, &own) in owned.iter().enumerate() {
            if verts == fill || (verts > 0 && faces + own > most) {
                starts.push(s);
                (verts, faces) = (0, 0);
            }
            verts += 1;
            faces += own;
        }
        starts.push(order.len());
        Cut::new(cfg, order, starts, src)
    }
}

/// Cut `src` into nodes. Fails if a node would need more face slots or a vertex more ring
/// entries than the id layout allows, or the mesh needs more nodes than ids can hold.
pub(crate) fn build(
    src: &Flat,
    cfg: NodeConfig,
    defs: &[AttrDef],
) -> Result<(Columns, Placed), MeshError> {
    let order = morton_order(&src.pos);
    let (nv, fill) = (order.len(), cfg.fill() as usize);
    let too_many = |nn: usize| (nn > cfg.max_nodes()).then_some(MeshError::TooLarge(nn));
    if let Some(e) = too_many(nv.div_ceil(fill)) {
        return Err(e);
    }
    let even = (0..nv).step_by(fill).chain([nv]).collect();
    let mut cut = Cut::new(cfg, &order, even, &src.faces);
    if cut.crowded(4 * fill).is_some() {
        cut = cut.by_faces(cfg, &order, &src.faces, 4 * fill);
        if let Some(e) = too_many(cut.nodes()) {
            return Err(e);
        }
    }
    if let Some(k) = cut.crowded(cfg.face_capacity() as usize) {
        return Err(MeshError::Input(format!(
            "node {k} needs more face slots than it has"
        )));
    }
    let Cut {
        starts,
        remap,
        faces,
        fstart,
    } = cut;
    let slots: Vec<usize> = starts.windows(2).map(|w| w[1] - w[0]).collect();
    let stored = gs_par::map(slots.len(), |k| {
        (faces[fstart[k]..fstart[k + 1]].iter())
            .map(|x| x.0)
            .collect()
    });
    let topo = topology(cfg, &slots, stored)?;
    let nodes = gs_par::map(slots.len(), |k| {
        let verts = &order[starts[k]..starts[k + 1]];
        let own = &faces[fstart[k]..fstart[k + 1]];
        let attrs: Vec<Option<Arc<AttrVec>>> = defs
            .iter()
            .zip(&src.attrs)
            .map(|(d, a)| {
                a.as_ref().map(|a| {
                    let mut v = match d.domain {
                        Domain::Vertex => gather(a, verts.iter().copied()),
                        Domain::Face => gather(a, own.iter().map(|x| x.1)),
                    };
                    // A mirror map names partners by source index; the store, by id.
                    if crate::mirror::axis_of(d).is_some()
                        && let AttrVec::U32(v) = &mut v
                    {
                        for x in v {
                            *x = remap.get(*x as usize).copied().unwrap_or(NO);
                        }
                    }
                    Arc::new(v)
                })
            })
            .collect();
        let pos: Vec<Vec3> = verts.iter().map(|&i| src.pos[i as usize]).collect();
        (pos, attrs)
    });
    let mut cols = Columns {
        attrs: vec![Vec::new(); defs.len()],
        topo,
        ..Columns::default()
    };
    for (pos, attrs) in nodes {
        cols.nrm.push(Arc::new(vec![Vec3::ZERO; pos.len()]));
        cols.pos.push(Arc::new(pos));
        for (col, a) in cols.attrs.iter_mut().zip(attrs) {
            col.push(a);
        }
    }
    Ok((cols, Placed { remap, faces }))
}
