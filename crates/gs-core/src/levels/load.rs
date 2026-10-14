//! Levels as a file stores them (decision 0014, section 10): built from the base faces and the
//! top level's points, with the level the file was saved at loaded as edits not carried up yet;
//! and moving every level's points at once (an import's scale and fit).

use std::sync::Arc;

use gs_protocol::{AttrDomain, PlainMesh};

use super::{Levels, Settings, bad};
use crate::build::gather;
use crate::columns::{AttrDef, AttrVec, ColumnSet, Domain};
use crate::config::{MeshError, NodeConfig, NodeId};
use crate::math::Vec3;
use crate::numbering::Numbering;
use crate::plain::{attr_vec, checked};
use crate::store::MeshStore;
use crate::subdivide::level_vertex_counts;

/// Every face of the store under `num` equals `faces` (canonical), checked in parallel.
fn faces_match(store: &MeshStore, num: &Numbering, faces: &[[u32; 4]]) -> bool {
    num.face_count() == faces.len()
        && gs_par::reduce(
            faces.len(),
            || true,
            |ok, c| ok && num.canonical_face(store, c as u32) == faces[c],
            |a, b| a && b,
        )
}

/// Maps every position of `store`, then brings its normals, boxes and tree up to date.
fn map_store(store: &mut MeshStore, f: &(impl Fn(Vec3) -> Vec3 + Sync)) {
    gs_par::apply_each(&mut store.cols.pos, |_, col| {
        for p in Arc::make_mut(col).iter_mut() {
            *p = f(*p);
        }
    });
    let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    store.after_moves(&all);
}

impl Levels {
    /// A stack of `count` levels over `base` (canonical level-0 faces over `base_count` points),
    /// every level's points taken from `top`, the top level as stored: its positions by
    /// canonical index, its faces (which must follow decision 0006's rule) and its attribute
    /// columns. The top is active.
    pub fn from_top(
        base: &[[u32; 4]],
        base_count: u32,
        count: usize,
        top: &PlainMesh,
        settings: &Settings,
        cfg: NodeConfig,
    ) -> Result<Levels, MeshError> {
        let defs = checked(top)?;
        let counts = level_vertex_counts(base, base_count, count.saturating_sub(1) as u32)
            .ok_or_else(|| bad("a base face uses a point that does not exist"))?;
        if count == 0 || counts.last().copied() != Some(top.positions.len() as u64) {
            return Err(bad(
                "the stored top level does not have the points its level count gives",
            ));
        }
        let level0 = PlainMesh {
            positions: top.positions[..base_count as usize].to_vec(),
            faces: base.to_vec(),
            attributes: std::collections::BTreeMap::new(),
        };
        let (store, placed) = MeshStore::import(level0, cfg)?;
        let num = Numbering::from_placement(&store, placed);
        let mut l = Levels::new(store, Arc::new(num), settings)?;
        for _ in 1..count {
            l.add_level(true, false, Some(&top.positions))?;
        }
        if !faces_match(&l.top, l.numbering(), &top.faces) {
            return Err(bad(
                "the stored top level's faces do not follow the subdivision rule",
            ));
        }
        l.load_columns(top, &defs)?;
        Ok(l)
    }

    /// Registers `top`'s attribute columns (definitions `defs`, in name order) on the top store
    /// and fills them by canonical index.
    fn load_columns(&mut self, top: &PlainMesh, defs: &[AttrDef]) -> Result<(), MeshError> {
        let num = Arc::clone(self.numbering());
        let canon = num.face_canon();
        let starts = num.face_starts().to_vec();
        for (def, a) in defs.iter().zip(top.attributes.values()) {
            let id = self.top.add_attr(def.clone())?;
            let values = attr_vec(a.values.clone(), def);
            let store = &self.top;
            let cols = gs_par::map(store.node_count(), |n| {
                let col = match def.domain {
                    Domain::Vertex => {
                        let slots = store.cols.pos[n].len() as u32;
                        gather(&values, (0..slots).map(|l| num.index(n as NodeId, l)))
                    }
                    Domain::Face => {
                        let slots = store.cols.topo[n].faces.len();
                        let first = starts[n] as usize;
                        gather(&values, (first..first + slots).map(|d| canon[d]))
                    }
                };
                Some(Arc::new(col))
            });
            self.top.cols.attrs[id.index()] = cols;
        }
        let all: Vec<NodeId> = (0..self.top.node_count() as NodeId).collect();
        let every = self.top.attr_set();
        self.top.mark(&all, every);
        self.top.refresh_state(&all, every);
        Ok(())
    }

    /// Loads `mesh`, a stored copy of the active level (positions and point attribute columns
    /// by canonical index, faces in canonical order), into the live store as edits not carried
    /// up yet: what differs from the level gathered from the top is pending. Columns the top
    /// does not have are left out. Fails if the active level is the top or the faces differ.
    pub fn load_active(&mut self, mesh: &PlainMesh) -> Result<(), MeshError> {
        let num = Arc::clone(self.numbering());
        if num.vertex_count() != mesh.positions.len()
            || !faces_match(self.store(), &num, &mesh.faces)
        {
            return Err(bad("the stored level does not match the active level"));
        }
        let Some(live) = &mut self.live else {
            return Err(bad("the active level is the top; nothing is pending there"));
        };
        let store = &mut live.store;
        let mut written = Vec::new();
        for n in 0..store.node_count() {
            let at = |l: usize| num.index(n as NodeId, l as u32) as usize;
            let want = |l: usize| Vec3::from_array(mesh.positions[at(l)]);
            let col = &mut store.cols.pos[n];
            if (0..col.len()).any(|l| col[l].bits() != want(l).bits()) {
                for (l, p) in Arc::make_mut(col).iter_mut().enumerate() {
                    *p = want(l);
                }
                written.push(n as NodeId);
            }
        }
        store.after_moves(&written);
        for (name, a) in &mesh.attributes {
            let Some(id) = store.attr_id(name) else {
                continue;
            };
            let def = &store.defs[id.index()];
            if def.domain != Domain::Vertex || a.domain != AttrDomain::Vertex {
                continue;
            }
            let values = attr_vec(a.values.clone(), def);
            let mut nodes = Vec::new();
            for n in 0..store.node_count() {
                let slots = store.cols.pos[n].len() as u32;
                let col = gather(&values, (0..slots).map(|l| num.index(n as NodeId, l)));
                let held = &mut store.cols.attrs[id.index()][n];
                let zeros = AttrVec::zeros(def.kind, slots as usize);
                if held.as_deref().unwrap_or(&zeros) != &col {
                    *held = Some(Arc::new(col));
                    nodes.push(n as NodeId);
                }
            }
            store.mark(&nodes, ColumnSet::attr(id));
            store.refresh_state(&nodes, ColumnSet::attr(id));
        }
        Ok(())
    }

    /// Moves every point of every level by `f` (an affine map, such as an import's scale and
    /// fit), keeping pending edits pending: the top, the live store and its gathered state all
    /// move, and the live store keeps sharing what it had not edited.
    pub fn map_positions(&mut self, f: impl Fn(Vec3) -> Vec3 + Sync) {
        map_store(&mut self.top, &f);
        let Some(live) = &mut self.live else {
            return;
        };
        let before = live.gathered.cols.pos.clone();
        map_store(&mut live.gathered, &f);
        let (store, gathered) = (&mut live.store, &live.gathered);
        for (n, col) in store.cols.pos.iter_mut().enumerate() {
            if Arc::ptr_eq(col, &before[n]) {
                *col = Arc::clone(&gathered.cols.pos[n]);
            } else {
                for p in Arc::make_mut(col).iter_mut() {
                    *p = f(*p);
                }
            }
        }
        let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
        store.after_moves(&all);
    }
}
