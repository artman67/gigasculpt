//! Pulling a shell off a mesh: Extract.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, GROUP, HIDE, MASK, NO_VERTEX, PlainMesh};

use super::{corners, face_edges, gather, used};
use crate::math::{cross, sub};

/// The faces `faces` of `mesh` (by index, ascending, each once) as a closed shell.
///
/// Two layers of the region's vertices, moved along their normals: to `+|thickness|` and
/// `-|thickness|` with `double`, else to 0 and `thickness`. A vertex normal is the normalized
/// sum of the area-weighted normals of the region's faces around it, summed in face order in
/// `f64`. The outer layer (the one farther along the normals) keeps the faces' winding and
/// comes first, the inner one is reversed (its first corner kept first), and every boundary
/// edge of the region (an edge one region face uses) gets a quad joining the layers, wound to
/// match, in the order of its face and that face's edges.
///
/// The face column `group` says which is which: 1 the outer layer, 2 the inner, 3 the rim.
/// `mask` and `hide` are dropped; every other vertex column is copied to both layers, and every
/// other face column is taken from the region face (a rim face takes the face beside it).
pub fn extract(mesh: &PlainMesh, faces: &[u32], thickness: f32, double: bool) -> PlainMesh {
    let verts = used(mesh, faces);
    let n = verts.len() as u32;
    let mut remap = vec![NO_VERTEX; mesh.positions.len()];
    for (new, &old) in verts.iter().enumerate() {
        remap[old as usize] = new as u32;
    }
    let region: Vec<[u32; 4]> = (faces.iter())
        .map(|&f| {
            mesh.faces[f as usize].map(|v| if v == NO_VERTEX { v } else { remap[v as usize] })
        })
        .collect();

    let normals = normals(&verts, &region, mesh);
    let t = f64::from(thickness);
    let (outer, inner) = if double {
        (t.abs(), -t.abs())
    } else {
        (t.max(0.0), t.min(0.0))
    };
    let layer = |offset: f64| {
        (verts.iter().zip(&normals)).map(move |(&v, normal)| {
            let p = mesh.positions[v as usize];
            std::array::from_fn(|i| (f64::from(p[i]) + normal[i] * offset) as f32)
        })
    };
    let positions: Vec<[f32; 3]> = layer(outer).chain(layer(inner)).collect();

    // Region edges by their two ends: how many region faces use each.
    let mut uses: BTreeMap<[u32; 2], u32> = BTreeMap::new();
    for &face in &region {
        for [a, b] in face_edges(face) {
            *uses.entry([a.min(b), a.max(b)]).or_default() += 1;
        }
    }
    let down = |v: u32| if v == NO_VERTEX { v } else { v + n };
    let mut out_faces: Vec<[u32; 4]> = region.clone();
    out_faces.extend(region.iter().map(|&face| reversed(face).map(down)));
    let mut rim_of = Vec::new();
    for (k, &face) in region.iter().enumerate() {
        for [a, b] in face_edges(face) {
            if uses[&[a.min(b), a.max(b)]] == 1 {
                out_faces.push([b, a, a + n, b + n]);
                rim_of.push(faces[k]);
            }
        }
    }

    let count = region.len();
    let mut attributes = BTreeMap::new();
    for (name, column) in &mesh.attributes {
        if matches!(name.as_str(), MASK | HIDE | GROUP) {
            continue;
        }
        let at: Vec<u32> = match column.domain {
            AttrDomain::Vertex => verts.iter().chain(&verts).copied().collect(),
            AttrDomain::Face => (faces.iter().chain(faces).chain(&rim_of))
                .copied()
                .collect(),
        };
        let values = gather(&column.values, &at);
        attributes.insert(
            name.clone(),
            Attribute {
                domain: column.domain,
                values,
            },
        );
    }
    let group = (std::iter::repeat_n(1, count))
        .chain(std::iter::repeat_n(2, count))
        .chain(std::iter::repeat_n(3, rim_of.len()))
        .collect();
    attributes.insert(
        GROUP.to_string(),
        Attribute {
            domain: AttrDomain::Face,
            values: AttrValues::U32(group),
        },
    );
    PlainMesh {
        positions,
        faces: out_faces,
        attributes,
    }
}

/// The unit normal of each region vertex: the sum of the area-weighted normals of the region
/// faces around it, in face order, normalized (zero where they cancel).
fn normals(verts: &[u32], region: &[[u32; 4]], mesh: &PlainMesh) -> Vec<[f64; 3]> {
    let at = |v: u32| mesh.positions[verts[v as usize] as usize].map(f64::from);
    let mut sum = vec![[0.0f64; 3]; verts.len()];
    for &face in region {
        // Twice the vector area: the cross product of the diagonals (of the two edges for a
        // triangle).
        let [a, b, c, d] = face;
        let area = if d == NO_VERTEX {
            cross(sub(at(b), at(a)), sub(at(c), at(a)))
        } else {
            cross(sub(at(c), at(a)), sub(at(d), at(b)))
        };
        for &v in corners(&face) {
            let s = &mut sum[v as usize];
            *s = std::array::from_fn(|i| s[i] + area[i]);
        }
    }
    (sum.into_iter())
        .map(|s| {
            let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
            if len > 0.0 {
                s.map(|c| c / len)
            } else {
                [0.0; 3]
            }
        })
        .collect()
}

/// A face with its corners reversed, the first kept first.
fn reversed([a, b, c, d]: [u32; 4]) -> [u32; 4] {
    if d == NO_VERTEX {
        [a, c, b, d]
    } else {
        [a, d, c, b]
    }
}
