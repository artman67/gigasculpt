//! Cutting a mesh into pieces, joining meshes into one, and pulling a shell off a mesh: the
//! geometry of the part list's split, merge and extract (decision 0028, section 4). Pure
//! functions, mesh in and meshes out, like Voxel Remesh.
//!
//! Every attribute column of a `PlainMesh` carries by its domain: a vertex column follows its
//! vertices and a face column its faces. Nothing is welded, capped or reordered: a piece holds
//! its faces in their order and the vertices they use in ascending original order.

mod extract;
mod merge;

use std::collections::BTreeMap;

use gs_protocol::{
    AttrDomain, AttrValues, Attribute, GROUP, HIDE, MASK, NO_VERTEX, PlainMesh, arity, corners,
};

pub use extract::extract;
pub use merge::{MergeError, merge};

/// One piece of a split mesh.
#[derive(Clone, Debug)]
pub struct Piece {
    /// The label its faces had.
    pub label: u32,
    /// The piece.
    pub mesh: PlainMesh,
}

/// Cuts `mesh` into one piece per label: `labels` has one label per face (a face past its end
/// counts as label 0). The pieces come in the order of each label's first face. A vertex on a
/// seam goes into every piece that uses it, so the pieces' faces add up to the mesh's exactly
/// and their vertices to the mesh's plus the seam copies.
pub fn split(mesh: &PlainMesh, labels: &[u32]) -> Vec<Piece> {
    let mut slot: BTreeMap<u32, usize> = BTreeMap::new();
    let mut pieces: Vec<(u32, Vec<u32>)> = Vec::new();
    for f in 0..mesh.faces.len() {
        let label = labels.get(f).copied().unwrap_or(0);
        let at = *slot.entry(label).or_insert_with(|| {
            pieces.push((label, Vec::new()));
            pieces.len() - 1
        });
        pieces[at].1.push(f as u32);
    }
    let mut remap = vec![NO_VERTEX; mesh.positions.len()];
    (pieces.into_iter())
        .map(|(label, faces)| Piece {
            label,
            mesh: sub_mesh(mesh, &faces, &mut remap),
        })
        .collect()
}

/// The faces `faces` of `mesh` (by index, in that order) as a piece of their own, every column
/// carried, as [`split`] makes each piece.
pub fn piece(mesh: &PlainMesh, faces: &[u32]) -> PlainMesh {
    sub_mesh(mesh, faces, &mut vec![NO_VERTEX; mesh.positions.len()])
}

/// Each face's `hide` value as a label: 1 hidden, 0 not (and 0 everywhere without the column).
pub fn by_hidden(mesh: &PlainMesh) -> Vec<u32> {
    let hide = face_u32(mesh, HIDE);
    hide.into_iter().map(|h| u32::from(h != 0)).collect()
}

/// Each face's `group` value as a label (0 everywhere without the column).
pub fn by_groups(mesh: &PlainMesh) -> Vec<u32> {
    face_u32(mesh, GROUP)
}

/// Each face's shell as a label: faces sharing an edge are in one shell (faces that only touch
/// at a point are not), and shells are numbered from 0 in the order of their lowest face.
pub fn by_shells(mesh: &PlainMesh) -> Vec<u32> {
    let n = mesh.faces.len();
    let mut edges: Vec<([u32; 2], u32)> = Vec::new();
    for (f, &face) in mesh.faces.iter().enumerate() {
        for [a, b] in face_edges(face) {
            edges.push(([a.min(b), a.max(b)], f as u32));
        }
    }
    edges.sort_unstable();
    // Union-find over faces; a root is always the lowest face of its shell.
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let find = |parent: &mut Vec<u32>, mut f: u32| {
        while parent[f as usize] != f {
            let up = parent[parent[f as usize] as usize];
            parent[f as usize] = up;
            f = up;
        }
        f
    };
    for pair in edges.windows(2) {
        if pair[0].0 == pair[1].0 {
            let (a, b) = (find(&mut parent, pair[0].1), find(&mut parent, pair[1].1));
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    let mut label_of = vec![u32::MAX; n];
    let mut next = 0;
    (0..n as u32)
        .map(|f| {
            let root = find(&mut parent, f) as usize;
            if label_of[root] == u32::MAX {
                label_of[root] = next;
                next += 1;
            }
            label_of[root]
        })
        .collect()
}

/// Each face as masked (1) or not (0): masked when the mean of its corners' `mask` values,
/// summed in corner order, is at least `threshold` (the selectors' rule, decision 0015). A mesh
/// without a mask column is unmasked.
pub fn by_mask(mesh: &PlainMesh, threshold: f32) -> Vec<u32> {
    let mask = match mesh.attributes.get(MASK) {
        Some(Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(v),
        }) => Some(v),
        _ => None,
    };
    let m = |v: u32| mask.and_then(|m| m.get(v as usize)).copied().unwrap_or(0.0);
    (mesh.faces.iter())
        .map(|face| {
            let corners = corners(face);
            let mean = corners.iter().map(|&v| m(v)).sum::<f32>() / corners.len() as f32;
            u32::from(mean >= threshold)
        })
        .collect()
}

/// The faces `faces` of `mesh` (by index, in that order) as a mesh of their own, with the
/// vertices they use in ascending order and every column carried. `remap` is scratch space,
/// one entry per vertex of `mesh`.
fn sub_mesh(mesh: &PlainMesh, faces: &[u32], remap: &mut [u32]) -> PlainMesh {
    let verts = used(mesh, faces);
    for (new, &old) in verts.iter().enumerate() {
        remap[old as usize] = new as u32;
    }
    let renumber = |v: u32| if v == NO_VERTEX { v } else { remap[v as usize] };
    PlainMesh {
        positions: verts.iter().map(|&v| mesh.positions[v as usize]).collect(),
        faces: (faces.iter())
            .map(|&f| mesh.faces[f as usize].map(renumber))
            .collect(),
        attributes: columns(mesh, &verts, faces),
    }
}

/// The vertices the faces `faces` of `mesh` use, ascending, each once.
fn used(mesh: &PlainMesh, faces: &[u32]) -> Vec<u32> {
    let mut verts: Vec<u32> = (faces.iter())
        .flat_map(|&f| corners(&mesh.faces[f as usize]).iter().copied())
        .collect();
    verts.sort_unstable();
    verts.dedup();
    verts
}

/// Every column of `mesh`, its vertex columns taken at `verts` and its face columns at
/// `faces`.
fn columns(mesh: &PlainMesh, verts: &[u32], faces: &[u32]) -> BTreeMap<String, Attribute> {
    (mesh.attributes.iter())
        .map(|(name, column)| {
            let at = match column.domain {
                AttrDomain::Vertex => verts,
                AttrDomain::Face => faces,
            };
            let values = gather(&column.values, at);
            (
                name.clone(),
                Attribute {
                    domain: column.domain,
                    values,
                },
            )
        })
        .collect()
}

/// The values at `at`, in that order; 0 past the end of a short column.
fn gather(values: &AttrValues, at: &[u32]) -> AttrValues {
    fn take<T: Copy + Default>(v: &[T], at: &[u32]) -> Vec<T> {
        (at.iter())
            .map(|&i| v.get(i as usize).copied().unwrap_or_default())
            .collect()
    }
    match values {
        AttrValues::F32(v) => AttrValues::F32(take(v, at)),
        AttrValues::U32(v) => AttrValues::U32(take(v, at)),
    }
}

/// The face column `name` as whole numbers, one per face: 0 where it is missing, short or of
/// another kind.
fn face_u32(mesh: &PlainMesh, name: &str) -> Vec<u32> {
    let values = match mesh.attributes.get(name) {
        Some(Attribute {
            domain: AttrDomain::Face,
            values: AttrValues::U32(v),
        }) => v.as_slice(),
        _ => &[],
    };
    (0..mesh.faces.len())
        .map(|f| values.get(f).copied().unwrap_or(0))
        .collect()
}

/// The edges of a face in winding order, each as (from, to).
fn face_edges(face: [u32; 4]) -> impl Iterator<Item = [u32; 2]> {
    let n = arity(face);
    (0..n).map(move |i| [face[i], face[(i + 1) % n]])
}
