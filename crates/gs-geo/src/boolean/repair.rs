//! The repair ladder, steps 1 to 4 (decision 0057, section 2): an operand checked, imported, and
//! turned or rebuilt when it needs it, ready for any number of cuts.

use std::fmt;
use std::sync::Arc;

use gs_protocol::{PlainMesh, face_triangles};
use manifold_rust::impl_mesh::ManifoldImpl;
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, MeshGL64, WindingRule};

use super::{Step, Unfit, check};

/// An operand through the ladder: the library's mesh of it, and the triangles and faces the
/// face ids it carries index.
#[derive(Clone)]
pub struct Prepared {
    /// The library's mesh, shared by every leaf that uses it.
    pub(super) imp: Arc<ManifoldImpl>,
    /// The original id on every triangle of `imp`.
    pub(super) id: i32,
    /// The operand's points as it came in.
    pub(super) positions: Vec<[f32; 3]>,
    /// The operand's faces as it came in.
    pub(super) faces: Vec<[u32; 4]>,
    /// Its triangles as drawn, by point; a face id in `imp` is an index here.
    pub(super) tris: Vec<[u32; 3]>,
    /// Each triangle's face.
    pub(super) tri_face: Vec<u32>,
    steps: Vec<Step>,
}

impl Prepared {
    /// The repairs the ladder made, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }
}

impl fmt::Debug for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prepared")
            .field("points", &self.positions.len())
            .field("faces", &self.faces.len())
            .field("steps", &self.steps)
            .finish_non_exhaustive()
    }
}

/// The operand through steps 1 to 4, or why it cannot be used.
pub fn prepare(mesh: &PlainMesh) -> Result<Prepared, Unfit> {
    check::faces_and_points(mesh)?;
    let (tris, tri_face) = triangles(mesh);
    check::closed(&tris)?;
    let mut m = import(&mesh.positions, &tris)?;
    let mut steps = Vec::new();
    if check::any_shell_negative(mesh, &tris, &tri_face) {
        let turned = m.repair_orientation();
        refused(3, &turned)?;
        if turned.as_impl().halfedge != m.as_impl().halfedge {
            steps.push(Step::TurnedOut);
            m = turned;
        }
    }
    if m.has_self_intersections() {
        m = rebuild(&m, tris.len())?;
        steps.push(Step::Rebuilt);
    }
    let imp = m.into_impl();
    let id = imp
        .mesh_relation
        .tri_ref
        .first()
        .map_or(-1, |r| r.original_id);
    Ok(Prepared {
        imp: Arc::new(imp),
        id,
        positions: mesh.positions.clone(),
        faces: mesh.faces.clone(),
        tris,
        tri_face,
        steps,
    })
}

/// The mesh's triangles as drawn (`face_triangles`) and each one's face.
fn triangles(mesh: &PlainMesh) -> (Vec<[u32; 3]>, Vec<u32>) {
    let mut tris = Vec::with_capacity(2 * mesh.faces.len());
    let mut tri_face = Vec::with_capacity(2 * mesh.faces.len());
    for (f, &face) in mesh.faces.iter().enumerate() {
        for t in face_triangles(face, |k| mesh.positions[face[k] as usize]) {
            tris.push(t.map(|k| face[k]));
            tri_face.push(f as u32);
        }
    }
    (tris, tri_face)
}

/// The triangles imported, each with its index as face id; no triangles is the empty solid.
pub(super) fn import(positions: &[[f32; 3]], tris: &[[u32; 3]]) -> Result<Manifold, Unfit> {
    if tris.is_empty() {
        return Ok(Manifold::from_mesh_gl64(&MeshGL64::default()));
    }
    let gl = MeshGL64 {
        num_prop: 3,
        vert_properties: positions.iter().flatten().map(|&x| f64::from(x)).collect(),
        tri_verts: tris.iter().flatten().map(|&v| u64::from(v)).collect(),
        face_id: (0..tris.len() as u64).collect(),
        ..MeshGL64::default()
    };
    let m = Manifold::from_mesh_gl64(&gl);
    refused(1, &m)?;
    Ok(m)
}

/// `Refused` at this step if the library gave an error.
fn refused(step: u8, m: &Manifold) -> Result<(), Unfit> {
    match m.status() {
        Error::NoError => Ok(()),
        e => Err(Unfit::Refused {
            step,
            why: e.to_string(),
        }),
    }
}

/// Step 4: `rebuild_solid` with "inside at least once", each corner carrying its triangle's
/// face id as a constant channel, which the rebuild keeps and drops face ids; the result is
/// imported again with the channel as its face ids. `ids` is how many face ids there are (the
/// operand's triangles).
fn rebuild(m: &Manifold, ids: usize) -> Result<Manifold, Unfit> {
    let channelled = Manifold::from_mesh_gl64(&channel_mesh(m.as_impl()));
    refused(4, &channelled)?;
    let r = channelled.rebuild_solid(WindingRule::Positive);
    refused(4, &r)?;
    let imp = r.as_impl();
    let why = |why: &str| Unfit::Refused {
        step: 4,
        why: why.to_string(),
    };
    if r.is_empty() {
        return Err(why("nothing was left inside"));
    }
    if imp.num_prop == 0 {
        return Err(why("the face channel did not come through"));
    }
    let face_of = |t: usize| {
        let at =
            |k: usize| imp.properties[imp.halfedge[3 * t + k].prop_vert as usize * imp.num_prop];
        let f = at(0);
        let id = f as u32;
        let whole = f64::from(id).to_bits() == f.to_bits() && (id as usize) < ids;
        let same = (1..3).all(|k| at(k).to_bits() == f.to_bits());
        (whole && same).then_some(u64::from(id))
    };
    let face_id = (0..imp.num_tri())
        .map(face_of)
        .collect::<Option<Vec<u64>>>()
        .ok_or_else(|| why("a triangle came out without its face"))?;
    let gl = MeshGL64 {
        num_prop: 3,
        vert_properties: imp.vert_pos.iter().flat_map(|p| [p.x, p.y, p.z]).collect(),
        tri_verts: (imp.halfedge.iter()).map(|h| h.start_vert as u64).collect(),
        face_id,
        ..MeshGL64::default()
    };
    let again = Manifold::from_mesh_gl64(&gl);
    refused(4, &again)?;
    Ok(again)
}

/// The mesh with its own corners, each carrying its triangle's face id as a fourth property,
/// merged back to one point per position by the merge vectors.
fn channel_mesh(imp: &ManifoldImpl) -> MeshGL64 {
    let n = imp.num_tri();
    let mut props = Vec::with_capacity(12 * n);
    let mut first = vec![u64::MAX; imp.vert_pos.len()];
    let (mut from, mut to) = (Vec::new(), Vec::new());
    for t in 0..n {
        let face = f64::from(imp.mesh_relation.tri_ref[t].face_id);
        for k in 0..3 {
            let v = imp.halfedge[3 * t + k].start_vert as usize;
            let corner = (3 * t + k) as u64;
            let p = imp.vert_pos[v];
            props.extend([p.x, p.y, p.z, face]);
            if first[v] == u64::MAX {
                first[v] = corner;
            } else {
                from.push(corner);
                to.push(first[v]);
            }
        }
    }
    MeshGL64 {
        num_prop: 4,
        vert_properties: props,
        tri_verts: (0..3 * n as u64).collect(),
        merge_from_vert: from,
        merge_to_vert: to,
        ..MeshGL64::default()
    }
}
