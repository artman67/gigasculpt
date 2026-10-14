//! The result handed back (decision 0057, sections 1 and 6): the export rule, then the mesh read
//! out of the library with each face's and point's source, kept quads joined (`quads`), and the
//! 32-bit scan.

mod quads;

use gs_core::Vec3 as V32;
use gs_core::math::closest_weights;
use gs_protocol::PlainMesh;
use manifold_rust::impl_mesh::ManifoldImpl;
use manifold_rust::linalg::{Vec3, Vec4};
use manifold_rust::manifold::Manifold;

use super::tree::Placed;
use super::{Booleaned, Leaf, TOLERANCE, repair};
use crate::voxel::Sources;

/// No leaf: a triangle whose source the library lost (none did in any measured run).
const NOWHERE: u32 = u32::MAX;
/// No source corner at a triangle corner's place.
const NO_CORNER: u8 = 3;

/// Where an output triangle came from: its leaf, its triangle there, and for each of its
/// corners the corner of that triangle at the same place, if any.
#[derive(Clone, Copy)]
struct Traced {
    leaf: u32,
    src: u32,
    corner: [u8; 3],
}

impl Traced {
    /// All three corners are corners of the source triangle: the triangle came through untouched.
    fn untouched(&self) -> bool {
        let [a, b, c] = self.corner;
        self.leaf != NOWHERE && a != b && b != c && c != a && a.max(b).max(c) < NO_CORNER
    }

    /// Wound as its source (an even turn of its corners), not against it.
    fn as_wound(&self) -> bool {
        matches!(self.corner, [0, 1, 2] | [1, 2, 0] | [2, 0, 1])
    }
}

/// The group's leaves, with where each one's points, faces and triangles start when the
/// operands are laid end to end.
struct Layout<'a> {
    group: &'a [Leaf<'a>],
    placed: &'a [Placed],
    ids: Vec<(i32, u32)>,
    points: Vec<u32>,
    faces: Vec<u32>,
    tris: Vec<u64>,
}

impl<'a> Layout<'a> {
    fn new(group: &'a [Leaf<'a>], placed: &'a [Placed]) -> Self {
        let mut ids: Vec<(i32, u32)> = (placed.iter().enumerate())
            .map(|(l, p)| (p.id, l as u32))
            .collect();
        ids.sort_unstable();
        let starts = |count: &dyn Fn(&Leaf<'_>) -> usize| {
            let mut at = 0;
            (group.iter())
                .map(|l| {
                    let start = at;
                    at += count(l);
                    start
                })
                .collect::<Vec<usize>>()
        };
        Layout {
            group,
            placed,
            ids,
            points: starts(&|l| l.operand.positions.len())
                .iter()
                .map(|&s| s as u32)
                .collect(),
            faces: starts(&|l| l.operand.faces.len())
                .iter()
                .map(|&s| s as u32)
                .collect(),
            tris: starts(&|l| l.operand.tris.len())
                .iter()
                .map(|&s| s as u64)
                .collect(),
        }
    }

    /// The leaf whose triangles carry this original id.
    fn leaf(&self, id: i32) -> Option<usize> {
        let i = self.ids.binary_search_by_key(&id, |&(k, _)| k).ok()?;
        Some(self.ids[i].1 as usize)
    }

    /// Point `v` of leaf `l`, moved by the library's transform exactly as the library moves it.
    fn world(&self, l: usize, v: u32) -> Vec3 {
        let p = self.group[l].operand.positions[v as usize].map(f64::from);
        self.placed[l].transform * Vec4::new(p[0], p[1], p[2], 1.0)
    }
}

/// The library's result made into the group's result.
pub(super) fn finish(result: ManifoldImpl, placed: &[Placed], group: &[Leaf<'_>]) -> Booleaned {
    if result.is_empty() {
        return Booleaned::default();
    }
    let ruled = rule(result);
    let imp = ruled.as_impl();
    let lay = Layout::new(group, placed);
    let traced = trace(imp, &lay);
    let (new_of, count) = renumber(imp);
    let positions: Vec<[f32; 3]> = (imp.vert_pos.iter().zip(&new_of))
        .filter(|&(_, &n)| n != NOWHERE)
        .map(|(p, _)| [p.x as f32, p.y as f32, p.z as f32])
        .collect();
    let corners = |t: usize| -> [u32; 3] {
        std::array::from_fn(|k| new_of[imp.halfedge[3 * t + k].start_vert as usize])
    };
    let points = point_sources(imp, &lay, &traced, &new_of, count);
    let (faces, face_sources) = quads::faces(&lay, &traced, &corners, &positions);
    let mesh = PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    };
    let tris: Vec<[u32; 3]> = mesh.triangles().collect();
    // The scan, after a fresh import as the next cut would make it.
    let crosses = match repair::import(&mesh.positions, &tris) {
        Ok(m) => m.has_self_intersections(),
        Err(_) => true,
    };
    Booleaned {
        mesh,
        sources: Sources {
            points,
            faces: face_sources,
        },
        repaired: Vec::new(),
        crosses,
    }
}

/// The export rule: edges shorter than `TOLERANCE` times the bounding box diagonal collapsed by
/// the library's `set_tolerance`, after raising the mesh's epsilon to the same tolerance (outside
/// a boolean the library collapses only up to its epsilon, manifold #1671).
fn rule(result: ManifoldImpl) -> Manifold {
    let m = Manifold::from_impl(result);
    let b = m.bounding_box();
    let d = b.max - b.min;
    let tolerance = TOLERANCE * (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    let mut imp = m.into_impl();
    imp.epsilon = imp.epsilon.max(tolerance);
    Manifold::from_impl(imp).set_tolerance(tolerance)
}

/// Each output triangle's source, from the original id and face id the library carried.
fn trace(imp: &ManifoldImpl, lay: &Layout<'_>) -> Vec<Traced> {
    gs_par::map(imp.num_tri(), |t| {
        let r = imp.mesh_relation.tri_ref[t];
        let lost = Traced {
            leaf: NOWHERE,
            src: 0,
            corner: [NO_CORNER; 3],
        };
        let Some(l) = lay.leaf(r.original_id) else {
            return lost;
        };
        let tris = &lay.group[l].operand.tris;
        let Some(&source) = usize::try_from(r.face_id).ok().and_then(|s| tris.get(s)) else {
            return lost;
        };
        let at = source.map(|v| lay.world(l, v));
        let corner = std::array::from_fn(|k| {
            let p = imp.vert_pos[imp.halfedge[3 * t + k].start_vert as usize];
            at.iter()
                .position(|&c| c == p)
                .map_or(NO_CORNER, |j| j as u8)
        });
        Traced {
            leaf: l as u32,
            src: r.face_id as u32,
            corner,
        }
    })
}

/// The library's points that triangles use, numbered in order: each one's new number
/// (`NOWHERE` for unused ones), and how many.
fn renumber(imp: &ManifoldImpl) -> (Vec<u32>, usize) {
    let mut new_of = vec![NOWHERE; imp.vert_pos.len()];
    for h in &imp.halfedge {
        new_of[h.start_vert as usize] = 0;
    }
    let mut count = 0;
    for n in &mut new_of {
        if *n == 0 {
            *n = count as u32;
            count += 1;
        }
    }
    (new_of, count)
}

/// Each point's source: the corner of the earliest leaf among its triangles (the seam rule),
/// one at a source corner first, then the lowest triangle; a point at a source corner is that
/// corner, any other gets its weights in the source triangle.
fn point_sources(
    imp: &ManifoldImpl,
    lay: &Layout<'_>,
    traced: &[Traced],
    new_of: &[u32],
    count: usize,
) -> Vec<Option<([u32; 3], [f32; 3])>> {
    let mut owner = vec![(NOWHERE, true, 0_usize, 0_usize); count];
    for (t, tr) in traced.iter().enumerate() {
        for k in 0..3 {
            let v = new_of[imp.halfedge[3 * t + k].start_vert as usize] as usize;
            let key = (tr.leaf, tr.corner[k] == NO_CORNER, t, k);
            if key < owner[v] {
                owner[v] = key;
            }
        }
    }
    gs_par::map(count, |v| {
        let (leaf, _, t, k) = owner[v];
        if leaf == NOWHERE {
            return None;
        }
        let (l, tr) = (leaf as usize, traced[t]);
        let source = lay.group[l].operand.tris[tr.src as usize];
        let named = source.map(|i| i + lay.points[l]);
        if tr.corner[k] != NO_CORNER {
            let mut w = [0.0; 3];
            w[tr.corner[k] as usize] = 1.0;
            return Some((named, w));
        }
        let to32 = |p: Vec3| V32::new(p.x as f32, p.y as f32, p.z as f32);
        let at = to32(imp.vert_pos[imp.halfedge[3 * t + k].start_vert as usize]);
        let corners = source.map(|i| to32(lay.world(l, i)));
        // `Sources` weights are 0 or more; the search's are, to rounding.
        Some((named, closest_weights(at, &corners).map(|w| w.max(0.0))))
    })
}
