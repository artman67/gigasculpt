//! A part's own numbering of its vertices and faces against its store's ids (decision 0014).
//!
//! A [`MeshStore`] sorts its vertices along a Morton curve and cuts them into nodes, so its own
//! order is not the order a file gave or a subdivision rule makes. A [`Numbering`] maps that
//! canonical order to the store's ids and back, for vertices and for faces with their corner
//! order. Parts made from plain meshes keep the caller's numbering with one; every detail level
//! has one, made by the level cut, whose canonical order is decision 0006's rule.
//!
//! A numbering holds while the store's topology is unchanged and the store has no dead slots,
//! which is true right after a build and for every level (levels never change topology).
//! "Dense" indices count live slots node by node: a vertex's place in [`MeshStore::to_plain`].

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::config::{FaceId, NodeConfig, NodeId, VertId};
use crate::math::{NO, arity};
use crate::mirror::is_map;
use crate::plain::Placement;
use crate::store::MeshStore;

/// A canonical numbering of a store's vertices and faces. See the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numbering {
    /// The node size the store ids are made with.
    cfg: NodeConfig,
    /// Canonical vertex to store vertex id.
    ids: Vec<VertId>,
    /// Dense store vertex index to canonical vertex.
    canon: Vec<u32>,
    /// Per node, the dense index of its first vertex slot; then the vertex count.
    vstart: Vec<u32>,
    /// Canonical face to store face id.
    faces: Vec<FaceId>,
    /// Per canonical face: which of its corners the stored face starts with, so stored corner
    /// `j` is canonical corner `(first + j) % corners`. Empty when every face starts at its
    /// first corner.
    first: Vec<u8>,
    /// Per node, the dense index of its first face slot; then the face count.
    fstart: Vec<u32>,
}

/// Dense starts of every node's vertex and face slots, each with the total at the end.
fn starts(store: &MeshStore) -> (Vec<u32>, Vec<u32>) {
    let (mut v, mut f) = (vec![0u32], vec![0u32]);
    for n in 0..store.node_count() as NodeId {
        let node = store.node(n);
        v.push(v[v.len() - 1] + node.pos.len() as u32);
        f.push(f[f.len() - 1] + node.faces.len() as u32);
    }
    (v, f)
}

/// Store ids of every dense slot, from the starts: vertex ids with `id(node, local)`.
fn dense_ids(starts: &[u32], id: impl Fn(NodeId, u32) -> u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(starts[starts.len() - 1] as usize);
    for (n, w) in starts.windows(2).enumerate() {
        out.extend((0..w[1] - w[0]).map(|l| id(n as NodeId, l)));
    }
    out
}

impl Numbering {
    /// The store's own order: canonical index = dense index, faces as stored.
    pub fn store_order(store: &MeshStore) -> Numbering {
        let cfg = store.config();
        let (vstart, fstart) = starts(store);
        let ids = dense_ids(&vstart, |n, l| cfg.vert(n, l));
        let faces = dense_ids(&fstart, |n, l| cfg.face(n, l));
        Numbering {
            cfg,
            canon: (0..ids.len() as u32).collect(),
            ids,
            vstart,
            faces,
            first: Vec::new(),
            fstart,
        }
    }

    /// The numbering `MeshStore::import` reported for the plain mesh `store` was built from.
    pub fn from_placement(store: &MeshStore, placed: Placement) -> Numbering {
        let (vstart, fstart) = starts(store);
        Self::from_starts(store.config(), vstart, fstart, placed)
    }

    /// The numbering `placed` gives nodes with the vertex and face slots of `table` (per node,
    /// `[vertex slots, face slots]`): a level's, read from a file before its store exists.
    pub(crate) fn from_table(cfg: NodeConfig, table: &[[u32; 2]], placed: Placement) -> Numbering {
        let prefix = |i: usize| {
            let mut s = Vec::with_capacity(table.len() + 1);
            s.push(0u32);
            for row in table {
                s.push(s[s.len() - 1] + row[i]);
            }
            s
        };
        Self::from_starts(cfg, prefix(0), prefix(1), placed)
    }

    /// The numbering `placed` gives nodes whose vertex and face slots start at `vstart` and
    /// `fstart` (dense indices, each with the total at the end).
    fn from_starts(
        cfg: NodeConfig,
        vstart: Vec<u32>,
        fstart: Vec<u32>,
        placed: Placement,
    ) -> Numbering {
        let vid = dense_ids(&vstart, |n, l| cfg.vert(n, l));
        let fid = dense_ids(&fstart, |n, l| cfg.face(n, l));
        let mut canon = vec![0u32; vid.len()];
        for (c, &d) in placed.vertices.iter().enumerate() {
            canon[d as usize] = c as u32;
        }
        let first = if placed.first_corner.iter().all(|&f| f == 0) {
            Vec::new()
        } else {
            placed.first_corner
        };
        Numbering {
            cfg,
            ids: placed.vertices.iter().map(|&d| vid[d as usize]).collect(),
            canon,
            vstart,
            faces: placed.faces.iter().map(|&d| fid[d as usize]).collect(),
            first,
            fstart,
        }
    }

    /// A numbering made by a level cut: canonical vertex ids, each node's points by slot
    /// (`members[node]`, canonical), canonical face ids and each node's face count; every face
    /// stored from its first corner.
    pub(crate) fn from_layout(
        cfg: NodeConfig,
        ids: Vec<VertId>,
        members: &[Vec<u32>],
        faces: Vec<FaceId>,
        face_counts: &[usize],
    ) -> Numbering {
        let prefix = |count: &mut dyn Iterator<Item = usize>| {
            let mut s = vec![0u32];
            for c in count {
                s.push(s[s.len() - 1] + c as u32);
            }
            s
        };
        Numbering {
            cfg,
            ids,
            canon: members.concat(),
            vstart: prefix(&mut members.iter().map(Vec::len)),
            faces,
            first: Vec::new(),
            fstart: prefix(&mut face_counts.iter().copied()),
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.ids.len()
    }
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// The store id of canonical vertex `c`.
    #[inline]
    pub fn id(&self, c: u32) -> VertId {
        self.ids[c as usize]
    }

    /// Every canonical vertex's store id.
    pub fn ids(&self) -> &[VertId] {
        &self.ids
    }

    /// The canonical index of slot `local` of node `node`.
    #[inline]
    pub fn index(&self, node: NodeId, local: u32) -> u32 {
        self.canon[(self.vstart[node as usize] + local) as usize]
    }

    /// The canonical index of store vertex `v`.
    #[inline]
    pub fn canonical(&self, v: VertId) -> u32 {
        self.index(self.cfg.node_of(v), self.cfg.local_of(v))
    }

    /// The store id of canonical face `c`, and which of its canonical corners the stored face
    /// starts with.
    #[inline]
    pub fn face(&self, c: u32) -> (FaceId, u8) {
        let first = self.first.get(c as usize).copied().unwrap_or(0);
        (self.faces[c as usize], first)
    }

    /// The canonical index of every face, by dense store index (node `n`'s face slot `j` is
    /// dense index `face_starts()[n] + j`). Built on demand: the other direction is kept.
    pub fn face_canon(&self) -> Vec<u32> {
        let dense = Self::dense(&self.fstart, self.cfg.fshift(), &self.faces);
        let mut out = vec![0u32; self.faces.len()];
        for (c, &d) in dense.iter().enumerate() {
            out[d as usize] = c as u32;
        }
        out
    }

    /// The dense index of each node's first face slot, then the face count.
    pub fn face_starts(&self) -> &[u32] {
        &self.fstart
    }

    /// Canonical face `c` as canonical vertex indices, in its canonical corner order.
    pub fn canonical_face(&self, store: &MeshStore, c: u32) -> [u32; 4] {
        let cfg = self.cfg;
        let (f, first) = self.face(c);
        let stored = store.node(cfg.face_node(f)).faces[cfg.face_local(f) as usize];
        let n = arity(stored);
        let mut out = [NO; 4];
        for (j, &v) in stored[..n].iter().enumerate() {
            out[(first as usize + j) % n] = self.canonical(v);
        }
        out
    }

    /// Every face as canonical vertex indices, in canonical order.
    pub fn canonical_faces(&self, store: &MeshStore) -> Vec<[u32; 4]> {
        gs_par::map(self.faces.len(), |c| self.canonical_face(store, c as u32))
    }

    /// The dense indices of store ids (`node << shift | local`) whose nodes start at `starts`:
    /// points with the node size's shift, faces with its face shift.
    fn dense(starts: &[u32], shift: u32, ids: &[u32]) -> Vec<u32> {
        let local = (1u32 << shift) - 1;
        gs_par::map(ids.len(), |i| {
            starts[(ids[i] >> shift) as usize] + (ids[i] & local)
        })
    }

    /// The numbering as dense arrays, the inverse of [`Numbering::from_placement`]: each
    /// canonical vertex's and face's dense store index, and which corner each stored face
    /// starts with (one per face, zeros included).
    pub fn placement(&self) -> Placement {
        let first_corner = if self.first.is_empty() {
            vec![0; self.faces.len()]
        } else {
            self.first.clone()
        };
        Placement {
            vertices: Self::dense(&self.vstart, self.cfg.shift(), &self.ids),
            faces: Self::dense(&self.fstart, self.cfg.fshift(), &self.faces),
            first_corner,
        }
    }

    /// Bytes its maps hold.
    pub fn bytes(&self) -> usize {
        4 * (self.ids.len() + self.canon.len() + self.vstart.len() + self.faces.len())
            + 4 * self.fstart.len()
            + self.first.len()
    }

    /// A plain mesh exported in the store's order ([`MeshStore::to_plain`] of `store`),
    /// renumbered into this numbering, faces and corner order included.
    pub fn apply(&self, sorted: PlainMesh) -> PlainMesh {
        let verts = Self::dense(&self.vstart, self.cfg.shift(), &self.ids);
        let faces = Self::dense(&self.fstart, self.cfg.fshift(), &self.faces);
        let renumbered = gs_par::map(faces.len(), |c| {
            let mut face = sorted.faces[faces[c] as usize];
            let n = arity(face);
            for v in &mut face[..n] {
                *v = self.canon[*v as usize];
            }
            let first = self.first.get(c).copied().unwrap_or(0);
            face[..n].rotate_right(first as usize);
            face
        });
        let attributes: BTreeMap<String, Attribute> = (sorted.attributes.into_iter())
            .map(|(name, a)| {
                let order = match a.domain {
                    AttrDomain::Vertex => &verts,
                    AttrDomain::Face => &faces,
                };
                let values = match &a.values {
                    AttrValues::F32(v) => AttrValues::F32(pick(v, order)),
                    // A mirror map's values are point indices: renumbered as the points are.
                    AttrValues::U32(v) if a.domain == AttrDomain::Vertex && is_map(&name) => {
                        let canon = |x: &u32| self.canon.get(*x as usize).copied().unwrap_or(NO);
                        AttrValues::U32(pick(v, order).iter().map(canon).collect())
                    }
                    AttrValues::U32(v) => AttrValues::U32(pick(v, order)),
                };
                let domain = a.domain;
                (name, Attribute { domain, values })
            })
            .collect();
        PlainMesh {
            positions: pick(&sorted.positions, &verts),
            faces: renumbered,
            attributes,
        }
    }
}

/// `values[pick[i]]` for every `i`.
fn pick<T: Copy + Send + Sync>(values: &[T], pick: &[u32]) -> Vec<T> {
    gs_par::map(pick.len(), |i| values[pick[i] as usize])
}
