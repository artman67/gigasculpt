//! A store's exact form, for scene files (decision 0016): every column as the store holds it,
//! node by node, dead slots and free lists included, so a store rebuilt from it is the same
//! store bit for bit and the next command gives the same result.
//!
//! Reading goes through the ordinary views ([`MeshStore::node`] for positions, faces and
//! attributes) plus [`MeshStore::free_slots`]. Writing is [`MeshStore::from_exact`], which
//! checks everything a store relies on and then rebuilds what is derived (vertex rings, normals,
//! bounds, halos, seam and shared lists, the tree). Rings come out as the engine keeps them:
//! sorted by face id, the pool in slot order, as the topology editor compacts every node it
//! touches.
//!
//! Attribute kinds and domains have names here, so a file stores columns by name, domain and
//! kind. A new kind needs its name in [`AttrKind::name`] and [`AttrKind::from_name`].

use std::sync::Arc;

use crate::bits::Bits;
use crate::build::pack_rings;
use crate::columns::{AttrDef, AttrKind, AttrVec, Columns, Domain, MAX_ATTRS, NodeTopo};
use crate::config::{MeshError, NodeConfig, NodeId};
use crate::derived::Derived;
use crate::history::History;
use crate::math::{NO, Vec3, arity};
use crate::store::{MeshStore, Work};

/// One node's stored columns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExactNode {
    /// Every vertex slot's position; a dead slot is NaN.
    pub pos: Vec<Vec3>,
    /// Every face slot; a dead slot is `[NO; 4]`.
    pub faces: Vec<[u32; 4]>,
    /// Free vertex slots, in stack order (the last is reused first).
    pub vfree: Vec<u32>,
    /// Free face slots, in stack order.
    pub ffree: Vec<u32>,
    /// One entry per attribute definition: `None` where the node never allocated the column.
    pub attrs: Vec<Option<AttrVec>>,
}

impl AttrKind {
    /// The kind's name in files.
    pub fn name(self) -> &'static str {
        match self {
            AttrKind::Scalar => "scalar",
            AttrKind::Label => "label",
            AttrKind::Rgba8 => "rgba8",
            AttrKind::Flag => "flag",
        }
    }
    /// The kind a file names.
    pub fn from_name(name: &str) -> Option<AttrKind> {
        [
            AttrKind::Scalar,
            AttrKind::Label,
            AttrKind::Rgba8,
            AttrKind::Flag,
        ]
        .into_iter()
        .find(|k| k.name() == name)
    }
}

impl Domain {
    /// The domain's name in files.
    pub fn name(self) -> &'static str {
        match self {
            Domain::Vertex => "vertex",
            Domain::Face => "face",
        }
    }
    /// The domain a file names.
    pub fn from_name(name: &str) -> Option<Domain> {
        match name {
            "vertex" => Some(Domain::Vertex),
            "face" => Some(Domain::Face),
            _ => None,
        }
    }
}

impl AttrVec {
    /// The values as 32-bit words, one per slot, as files store them (see [`AttrVec::bits`]).
    pub fn words(&self) -> Vec<u32> {
        (0..self.len()).map(|i| self.bits(i)).collect()
    }

    /// Values of a column of `kind` from the words [`AttrVec::words`] gave; `None` if a flag
    /// is neither 0 nor 1, which no column holds.
    pub fn of_words(kind: AttrKind, words: Vec<u32>) -> Option<AttrVec> {
        Some(match kind {
            AttrKind::Scalar => AttrVec::F32(words.into_iter().map(f32::from_bits).collect()),
            AttrKind::Label | AttrKind::Rgba8 => AttrVec::U32(words),
            AttrKind::Flag => {
                let mut bits = Bits::zeros(words.len());
                for (i, &w) in words.iter().enumerate() {
                    match w {
                        0 => {}
                        1 => bits.set(i, true),
                        _ => return None,
                    }
                }
                AttrVec::Bits(bits)
            }
        })
    }
}

fn bad(node: usize, what: impl std::fmt::Display) -> MeshError {
    MeshError::Input(format!("node {node}: {what}"))
}

/// True if the values fit a column of `kind`: the right type, and finite where they are floats.
pub(crate) fn fits(kind: AttrKind, values: &AttrVec) -> bool {
    match (kind, values) {
        (AttrKind::Scalar, AttrVec::F32(v)) => v.iter().all(|x| x.is_finite()),
        (AttrKind::Label | AttrKind::Rgba8, AttrVec::U32(_))
        | (AttrKind::Flag, AttrVec::Bits(_)) => true,
        _ => false,
    }
}

/// Names unique and at most [`MAX_ATTRS`].
fn check_defs(defs: &[AttrDef]) -> Result<(), MeshError> {
    if defs.len() > MAX_ATTRS {
        return Err(MeshError::Input(format!(
            "{} attribute columns, more than {MAX_ATTRS}",
            defs.len()
        )));
    }
    for (i, d) in defs.iter().enumerate() {
        if defs[..i].iter().any(|e| e.name == d.name) {
            return Err(MeshError::Input(format!("attribute {} twice", d.name)));
        }
    }
    Ok(())
}

/// Every free slot below `len`, listed once, naming a dead slot.
fn check_free(
    k: usize,
    what: &str,
    free: &[u32],
    dead: impl Fn(usize) -> bool,
) -> Result<(), MeshError> {
    let mut seen = free.to_vec();
    seen.sort_unstable();
    if seen.windows(2).any(|w| w[0] == w[1]) {
        return Err(bad(k, format!("a free {what} slot is listed twice")));
    }
    match free.iter().find(|&&l| !dead(l as usize)) {
        Some(l) => Err(bad(k, format!("free {what} slot {l} is not a dead slot"))),
        None => Ok(()),
    }
}

/// One node's own checks: sizes, positions, free lists and attribute columns.
fn check_node(cfg: NodeConfig, defs: &[AttrDef], k: usize, n: &ExactNode) -> Result<(), MeshError> {
    if n.pos.len() > cfg.capacity() as usize || n.faces.len() > cfg.face_capacity() as usize {
        return Err(bad(k, "more slots than a node has"));
    }
    let live = |p: &Vec3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    if let Some(l) = n.pos.iter().position(|p| !p.is_dead() && !live(p)) {
        return Err(bad(
            k,
            format!("vertex slot {l} is neither a point nor dead"),
        ));
    }
    if let Some(l) = n.faces.iter().position(|f| f[0] == NO && *f != [NO; 4]) {
        return Err(bad(k, format!("face slot {l} is half dead")));
    }
    let vdead = |l: usize| n.pos.get(l).is_some_and(|p| p.is_dead());
    check_free(k, "vertex", &n.vfree, vdead)?;
    let fdead = |l: usize| n.faces.get(l).is_some_and(|f| f[0] == NO);
    check_free(k, "face", &n.ffree, fdead)?;
    if n.attrs.len() != defs.len() {
        return Err(bad(
            k,
            format!("{} attribute columns, not {}", n.attrs.len(), defs.len()),
        ));
    }
    for (d, a) in defs.iter().zip(&n.attrs) {
        let Some(a) = a else { continue };
        let want = match d.domain {
            Domain::Vertex => n.pos.len(),
            Domain::Face => n.faces.len(),
        };
        if a.len() != want || !fits(d.kind, a) {
            return Err(bad(
                k,
                format!("attribute {} has the wrong length or values", d.name),
            ));
        }
    }
    Ok(())
}

/// Every live face names three or four distinct live vertices that exist.
fn check_faces(cfg: NodeConfig, nodes: &[ExactNode]) -> Result<(), MeshError> {
    let exists = |v: u32| {
        let (n, l) = (cfg.node_of(v) as usize, cfg.local_of(v) as usize);
        nodes
            .get(n)
            .and_then(|x| x.pos.get(l))
            .is_some_and(|p| !p.is_dead())
    };
    let problems = gs_par::map(nodes.len(), |k| {
        nodes[k].faces.iter().enumerate().find_map(|(l, f)| {
            if f[0] == NO {
                return None;
            }
            let c = &f[..arity(*f)];
            let repeats = (1..c.len()).any(|i| c[..i].contains(&c[i]));
            let missing = c.iter().any(|&v| v == NO || !exists(v));
            (repeats || missing)
                .then(|| bad(k, format!("face slot {l} names a vertex that is not there")))
        })
    });
    problems.into_iter().flatten().next().map_or(Ok(()), Err)
}

/// A node's packed ring headers and ring pool.
type Rings = (Vec<u32>, Vec<u32>);

/// Each node's ring headers and pool, from every live face using its vertices.
fn rings(cfg: NodeConfig, nodes: &[ExactNode]) -> Result<Vec<Rings>, MeshError> {
    // Corners that name another node's vertex, grouped by that node: `(local slot, face id)`.
    let foreign = gs_par::map(nodes.len(), |j| {
        let mut out = Vec::new();
        for (l, f) in nodes[j].faces.iter().enumerate().filter(|x| x.1[0] != NO) {
            for &v in f[..arity(*f)]
                .iter()
                .filter(|&&v| cfg.node_of(v) as usize != j)
            {
                out.push((
                    cfg.node_of(v),
                    cfg.local_of(v),
                    cfg.face(j as NodeId, l as u32),
                ));
            }
        }
        out
    });
    let mut by_node = vec![Vec::new(); nodes.len()];
    for (n, l, f) in foreign.into_iter().flatten() {
        by_node[n as usize].push((l, f));
    }
    let built = gs_par::map(nodes.len(), |k| {
        let mut entries = by_node[k].clone();
        for (l, f) in nodes[k].faces.iter().enumerate().filter(|x| x.1[0] != NO) {
            for &v in f[..arity(*f)]
                .iter()
                .filter(|&&v| cfg.node_of(v) as usize == k)
            {
                entries.push((cfg.local_of(v), cfg.face(k as NodeId, l as u32)));
            }
        }
        pack_rings(nodes[k].pos.len(), entries).map_err(|e| bad(k, e.to_string()))
    });
    built.into_iter().collect()
}

impl MeshStore {
    /// Node `n`'s free vertex and face slots, in stack order (the last is reused first).
    pub fn free_slots(&self, n: NodeId) -> (&[u32], &[u32]) {
        let t = &self.cols.topo[n as usize];
        (&t.vfree, &t.ffree)
    }

    /// The store whose exact form is `nodes` (see the module docs): checked, then every derived
    /// table rebuilt. It has no history, and every node is marked dirty. Refuses anything a
    /// store could not hold or that would break it later, naming the node.
    pub fn from_exact(
        cfg: NodeConfig,
        defs: Vec<AttrDef>,
        nodes: Vec<ExactNode>,
    ) -> Result<MeshStore, MeshError> {
        check_defs(&defs)?;
        if nodes.len() > cfg.max_nodes() {
            return Err(MeshError::TooLarge(nodes.len()));
        }
        let checked = gs_par::map(nodes.len(), |k| check_node(cfg, &defs, k, &nodes[k]));
        checked.into_iter().collect::<Result<Vec<()>, _>>()?;
        check_faces(cfg, &nodes)?;
        let rings = rings(cfg, &nodes)?;
        let mut cols = Columns {
            attrs: vec![Vec::with_capacity(nodes.len()); defs.len()],
            ..Columns::default()
        };
        for (node, (ring, pool)) in nodes.into_iter().zip(rings) {
            let ExactNode {
                pos,
                faces,
                vfree,
                ffree,
                attrs,
            } = node;
            cols.nrm.push(Arc::new(vec![Vec3::ZERO; pos.len()]));
            cols.pos.push(Arc::new(pos));
            let topo = NodeTopo {
                faces,
                ring,
                pool,
                vfree,
                ffree,
            };
            cols.topo.push(Arc::new(topo));
            for (col, a) in cols.attrs.iter_mut().zip(attrs) {
                col.push(a.map(Arc::new));
            }
        }
        let mut store = MeshStore {
            cfg,
            defs,
            cols,
            der: Derived::default(),
            dirty: Vec::new(),
            hist: History::default(),
            work: Work::default(),
        };
        store.refresh_all();
        Ok(store)
    }
}
