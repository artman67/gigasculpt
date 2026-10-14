//! Import from and export to `gs_protocol::PlainMesh`, the interchange type, the flat form
//! that re-sort and subdivide work on, and re-sort itself, beside the order code it reports
//! through ([`Placement`], [`MeshStore::store_corners`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::build::{Flat, Placed};
use crate::columns::{AttrDef, AttrId, AttrKind, AttrVec, ColumnSet, Domain, MAX_ATTRS};
use crate::config::{MeshError, NodeConfig, NodeId};
use crate::math::{NO, Vec3, arity};
use crate::store::MeshStore;

fn bad(s: String) -> MeshError {
    MeshError::Input(s)
}

/// Check every face: three or four distinct vertices, all in range.
fn check_faces(faces: &[[u32; 4]], nv: usize) -> Result<(), MeshError> {
    for (i, f) in faces.iter().enumerate() {
        let c = &f[..arity(*f)];
        if c.iter().any(|&v| v as usize >= nv) {
            return Err(bad(format!("face {i} uses a vertex that does not exist")));
        }
        if (1..c.len()).any(|k| c[..k].contains(&c[k])) {
            return Err(bad(format!("face {i} repeats a vertex")));
        }
    }
    Ok(())
}

impl AttrDef {
    /// The definition a plain mesh's column gets in a store built from it: a well-known name
    /// keeps its own (and must match it), any other name is a scalar (`f32`) or a label
    /// (`u32`). A command working on plain mesh data blends a column by this kind, the one the
    /// store will give it.
    pub fn of_plain(name: &str, a: &Attribute) -> Result<AttrDef, MeshError> {
        let domain = match a.domain {
            AttrDomain::Vertex => Domain::Vertex,
            AttrDomain::Face => Domain::Face,
        };
        let kind = match a.values {
            AttrValues::F32(_) => AttrKind::Scalar,
            AttrValues::U32(_) => AttrKind::Label,
        };
        let known = [
            AttrDef::mask(),
            AttrDef::face_group(),
            AttrDef::color(),
            AttrDef::hide(),
            AttrDef::uv_hint(),
            AttrDef::uv_density(),
        ];
        match known.into_iter().find(|d| d.name == name) {
            Some(d)
                if d.domain == domain
                    && (d.kind == AttrKind::Scalar) == (kind == AttrKind::Scalar) =>
            {
                Ok(d)
            }
            Some(_) => Err(bad(format!(
                "attribute {name} has the wrong domain or type"
            ))),
            None => Ok(AttrDef::new(name, domain, kind)),
        }
    }
}

/// Checks everything a store needs of a plain mesh, and gives its attribute columns'
/// definitions in name order: finite positions, valid faces, and at most [`MAX_ATTRS`]
/// attribute columns of the right type and length with finite values. A NaN mask would turn the
/// first stroke over it into a NaN position, which marks a dead slot (review 0002, M5); a group
/// id above [`crate::groups::MAX_GROUP`] would leave no fresh id (review 0007, m1).
pub(crate) fn checked(mesh: &PlainMesh) -> Result<Vec<AttrDef>, MeshError> {
    let nv = mesh.positions.len();
    if mesh.positions.iter().flatten().any(|x| !x.is_finite()) {
        return Err(bad("a position is not a finite number".into()));
    }
    check_faces(&mesh.faces, nv)?;
    if mesh.attributes.len() > MAX_ATTRS {
        let n = mesh.attributes.len();
        return Err(bad(format!(
            "{n} attribute columns, more than the {MAX_ATTRS} a mesh can hold"
        )));
    }
    let mut defs = Vec::new();
    for (name, a) in &mesh.attributes {
        let d = AttrDef::of_plain(name, a)?;
        let want = match d.domain {
            Domain::Vertex => nv,
            Domain::Face => mesh.faces.len(),
        };
        let len = match &a.values {
            AttrValues::F32(v) => v.len(),
            AttrValues::U32(v) => v.len(),
        };
        if len != want {
            return Err(bad(format!(
                "attribute {name} has {len} values, not {want}"
            )));
        }
        if let AttrValues::F32(v) = &a.values
            && let Some(i) = v.iter().position(|x| !x.is_finite())
        {
            return Err(bad(format!(
                "attribute {name} value {i} is not a finite number"
            )));
        }
        if let (AttrKind::Flag, AttrValues::U32(v)) = (d.kind, &a.values)
            && let Some(i) = v.iter().position(|&x| x > 1)
        {
            return Err(bad(format!("attribute {name} value {i} is not 0 or 1")));
        }
        if let (true, AttrValues::U32(v)) = (d == AttrDef::face_group(), &a.values) {
            crate::groups::check_ids(v)?;
        }
        defs.push(d);
    }
    Ok(defs)
}

/// A column's values as the store holds them: a flag column packed into bits, and the mask and
/// UV hints [`AttrDef::settled`] in their range, as every writer stores them, so an imported
/// mask keeps the mask's promises (review 0003 m3).
pub(crate) fn attr_vec(values: AttrValues, def: &AttrDef) -> AttrVec {
    match (values, def.kind) {
        (AttrValues::U32(v), AttrKind::Flag) => AttrVec::Bits(v.iter().map(|&x| x != 0).collect()),
        (AttrValues::F32(v), _) => def.settled(AttrVec::F32(v)),
        (AttrValues::U32(v), _) => AttrVec::U32(v),
    }
}

/// Where each vertex and face of an imported plain mesh went (see [`MeshStore::import`]). It
/// holds until the store's topology changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// Each vertex's index in [`MeshStore::to_plain`]'s output.
    pub vertices: Vec<u32>,
    /// Each face's index in [`MeshStore::to_plain`]'s output.
    pub faces: Vec<u32>,
    /// Which of each face's corners the stored face starts with: stored corner `j` is given
    /// corner `(first_corner + j) % corners`.
    pub first_corner: Vec<u8>,
}

impl MeshStore {
    /// Import a plain mesh: checked, sorted along a Morton curve and cut into nodes.
    pub fn from_plain(mesh: &PlainMesh, cfg: NodeConfig) -> Result<MeshStore, MeshError> {
        let defs = checked(mesh)?;
        let attrs = (mesh.attributes.values().zip(&defs))
            .map(|(a, d)| Some(attr_vec(a.values.clone(), d)))
            .collect();
        let flat = Flat {
            pos: mesh
                .positions
                .iter()
                .map(|&p| Vec3::from_array(p))
                .collect(),
            faces: mesh.faces.clone(),
            attrs,
        };
        Ok(MeshStore::from_flat(&flat, cfg, defs)?.0)
    }

    /// Import a plain mesh as [`MeshStore::from_plain`] does, taking it by value so its arrays
    /// are reused instead of copied, and say where each vertex and face went, so the caller can
    /// keep the mesh's own numbering without exporting the store again.
    pub fn import(mesh: PlainMesh, cfg: NodeConfig) -> Result<(MeshStore, Placement), MeshError> {
        let defs = checked(&mesh)?;
        let PlainMesh {
            positions,
            faces,
            attributes,
        } = mesh;
        let attrs = (attributes.into_values().zip(&defs))
            .map(|(a, d)| Some(attr_vec(a.values, d)))
            .collect();
        let pos = positions.into_iter().map(Vec3::from_array).collect();
        let flat = Flat { pos, faces, attrs };
        let (store, placed) = MeshStore::from_flat(&flat, cfg, defs)?;
        let Flat { pos, faces, attrs } = flat;
        drop((pos, attrs));
        let placement = placement(&store, &faces, &placed);
        Ok((store, placement))
    }

    /// Replace every column with a fresh cut of `flat`, numbering the nodes afresh. On error the
    /// mesh is unchanged. Inside an open step, the step records the change and undoes it as
    /// one. Outside one, the undo and redo steps are dropped, because they name nodes by the
    /// old numbering and undoing one would write over the wrong nodes.
    pub(crate) fn replace(&mut self, flat: &Flat) -> Result<Placed, MeshError> {
        let (cols, placed) = crate::build::build(flat, self.cfg, &self.defs)?;
        self.cols = cols;
        if !self.step_open() {
            self.forget_history();
        }
        self.work.sorts += 1;
        self.der = crate::derived::Derived::default();
        self.refresh_all();
        self.drop_broken_mirrors();
        Ok(placed)
    }

    /// Sort the vertices along the Morton curve again and cut new nodes, dropping dead slots.
    /// Restores memory order after topology edits. Inside an open step it is one undo step;
    /// outside one it drops the undo and redo history, which names the old nodes. Says where
    /// each vertex and face of the store's export before ([`MeshStore::to_plain`]) went and
    /// which of its corners each face now starts with, for [`MeshStore::store_corners`].
    pub fn resort(&mut self) -> Result<Placement, MeshError> {
        let flat = self.to_flat();
        let placed = self.replace(&flat)?;
        Ok(placement(self, &flat.faces, &placed))
    }

    /// The one place corner columns turn with their faces. Turns the corner columns `corners`
    /// (groups of four face columns, one per corner) from the corner order `placed` names to
    /// the stored one: the face at export index `placed.faces[i]` takes as its stored corner
    /// `j` what its corner `(placed.first_corner[i] + j) % corners` held (`placed.vertices` is
    /// not read). Every face slot must be live, as right after [`MeshStore::import`] or
    /// [`MeshStore::resort`] and in a detail level's store.
    ///
    /// For a part numbered in the store's own order, from the order given after an import, from
    /// the order before after a re-sort, and from a given numbering's corners when a part
    /// leaves it; and for a level stack renumbered by reconstruct. A part keeping its given
    /// order keeps them as given, since its numbering turns the faces back on export.
    pub fn store_corners(&mut self, corners: &[[String; 4]], placed: &Placement) {
        // With every slot live, a face's export index is its dense slot.
        let mut turn = vec![0u8; placed.faces.len()];
        for (&dense, &first) in placed.faces.iter().zip(&placed.first_corner) {
            turn[dense as usize] = first;
        }
        let mut start = vec![0];
        for t in &self.cols.topo {
            start.push(start[start.len() - 1] + t.faces.len());
        }
        let nodes: Vec<NodeId> = (0..self.node_count())
            .filter(|&n| turn[start[n]..start[n + 1]].iter().any(|&t| t != 0))
            .map(|n| n as NodeId)
            .collect();
        for group in corners {
            let Some(ids) = (group.iter())
                .map(|c| self.attr_id(c).map(AttrId::index))
                .collect::<Option<Vec<usize>>>()
            else {
                continue;
            };
            for &n in &nodes {
                let (n, faces) = (n as usize, &self.cols.topo[n as usize].faces);
                if ids.iter().all(|&a| self.cols.attrs[a][n].is_none()) {
                    continue;
                }
                let held: Vec<AttrVec> = (ids.iter())
                    .map(|&a| match &self.cols.attrs[a][n] {
                        Some(c) => AttrVec::clone(c),
                        None => AttrVec::zeros(self.defs[a].kind, faces.len()),
                    })
                    .collect();
                let mut new = held.clone();
                for (slot, &t) in turn[start[n]..start[n + 1]].iter().enumerate() {
                    let k = arity(faces[slot]);
                    for (j, col) in new.iter_mut().enumerate().take(k) {
                        col.set(slot, held[(t as usize + j) % k].get(slot));
                    }
                }
                for (&a, col) in ids.iter().zip(new) {
                    self.cols.attrs[a][n] = Some(Arc::new(col));
                }
            }
            for &a in &ids {
                let set = ColumnSet::attr(AttrId(a as u8));
                self.mark(&nodes, set);
                self.refresh_state(&nodes, set);
            }
        }
    }

    /// Export live vertices and faces in node order, with every attribute some node holds.
    pub fn to_plain(&self) -> PlainMesh {
        let flat = self.to_flat();
        let mut attributes = BTreeMap::new();
        for (d, a) in self.defs.iter().zip(flat.attrs) {
            let Some(a) = a else { continue };
            let values = match a {
                AttrVec::F32(v) => AttrValues::F32(v),
                AttrVec::U32(v) => AttrValues::U32(v),
                AttrVec::Bits(b) => {
                    AttrValues::U32((0..b.len()).map(|i| u32::from(b.get(i))).collect())
                }
            };
            let domain = match d.domain {
                Domain::Vertex => AttrDomain::Vertex,
                Domain::Face => AttrDomain::Face,
            };
            attributes.insert(d.name.clone(), Attribute { domain, values });
        }
        PlainMesh {
            positions: flat.pos.iter().map(|p| p.to_array()).collect(),
            faces: flat.faces,
            attributes,
        }
    }

    /// Live vertices and faces as a flat mesh in node order. An attribute no node holds is
    /// `None`; otherwise nodes that never wrote it contribute its empty value.
    pub(crate) fn to_flat(&self) -> Flat {
        let cfg = self.cfg;
        let cols = &self.cols;
        // Flat index of every vertex slot (dead slots get NO), and live slots per node.
        let mut index: Vec<Vec<u32>> = Vec::with_capacity(cols.len());
        let mut next = 0u32;
        for p in &cols.pos {
            index.push(
                p.iter()
                    .map(|q| {
                        if q.is_dead() {
                            NO
                        } else {
                            next += 1;
                            next - 1
                        }
                    })
                    .collect(),
            );
        }
        let live: Vec<Vec<usize>> = (index.iter())
            .map(|ix| (0..ix.len()).filter(|&l| ix[l] != NO).collect())
            .collect();
        let live_faces: Vec<Vec<usize>> = (cols.topo.iter())
            .map(|t| {
                (0..t.faces.len())
                    .filter(|&j| t.faces[j][0] != NO)
                    .collect()
            })
            .collect();
        let pos = (live.iter().enumerate())
            .flat_map(|(k, ls)| ls.iter().map(move |&l| cols.pos[k][l]))
            .collect();
        let faces = (live_faces.iter().enumerate())
            .flat_map(|(k, js)| js.iter().map(move |&j| cols.topo[k].faces[j]))
            .map(|mut f| {
                let ar = arity(f);
                for v in &mut f[..ar] {
                    *v = index[cfg.node_of(*v) as usize][cfg.local_of(*v) as usize];
                }
                f
            })
            .collect();
        let attrs = (self.defs.iter().zip(&cols.attrs))
            .map(|(d, col)| {
                col.iter().any(Option::is_some).then(|| {
                    let slots = match d.domain {
                        Domain::Vertex => &live,
                        Domain::Face => &live_faces,
                    };
                    let mut out = AttrVec::zeros(d.kind, 0);
                    for (c, s) in col.iter().zip(slots) {
                        append(&mut out, c.as_deref(), s, d.kind);
                    }
                    // A mirror map names partners by id; a flat mesh names them by index.
                    if crate::mirror::axis_of(d).is_some()
                        && let AttrVec::U32(v) = &mut out
                    {
                        for x in v {
                            let (n, l) = (cfg.node_of(*x) as usize, cfg.local_of(*x) as usize);
                            *x = index.get(n).and_then(|ix| ix.get(l)).copied().unwrap_or(NO);
                        }
                    }
                    out
                })
            })
            .collect();
        Flat { pos, faces, attrs }
    }
}

/// Where `build` put each vertex and face of the source faces `src`, as export indices: right
/// after a build every slot is live, so a vertex's export index is its node's first index plus
/// its slot, and a face's is its place among the sorted faces.
pub(crate) fn placement(store: &MeshStore, src: &[[u32; 4]], placed: &Placed) -> Placement {
    let cfg = store.cfg;
    let mut first = Vec::with_capacity(store.node_count());
    let mut next = 0u32;
    for p in &store.cols.pos {
        first.push(next);
        next += p.len() as u32;
    }
    let vertices = (placed.remap.iter())
        .map(|&v| first[cfg.node_of(v) as usize] + cfg.local_of(v))
        .collect();
    let (mut faces, mut first_corner) = (vec![0; src.len()], vec![0; src.len()]);
    for (j, (stored, i)) in placed.faces.iter().enumerate() {
        let given = &src[*i as usize];
        faces[*i as usize] = j as u32;
        first_corner[*i as usize] = (given[..arity(*given)].iter())
            .position(|&v| placed.remap[v as usize] == stored[0])
            .unwrap_or(0) as u8;
    }
    Placement {
        vertices,
        faces,
        first_corner,
    }
}

/// Append `src[i]` for each slot `i` (the kind's empty value when `src` is `None`).
fn append(out: &mut AttrVec, src: Option<&AttrVec>, slots: &[usize], kind: AttrKind) {
    let start = out.len();
    out.resize(start + slots.len(), kind);
    if let Some(src) = src {
        for (j, &i) in slots.iter().enumerate() {
            out.set(start + j, src.get(i));
        }
    }
}
