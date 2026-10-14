//! gs-geo boolean tests (decision 0057, LB2): manifold-rust's exact engine behind the repair
//! ladder. What the cuts make (volumes, empties, touching boxes, the left fold), where every
//! output face and point came from, the ladder's steps, the 32-bit export rule, and the same bits
//! at any thread count. Research 26's meshes are in `shapes`; its crossing check in `crossing`;
//! the library called directly, for comparisons, in `raw`.

mod crossing;
mod cuts;
mod export;
mod ladder;
mod raw;
mod same_bits;
mod shapes;
mod sources;

use gs_core::transform::Similarity;
use gs_geo::boolean::{self, Booleaned, Operand};
use gs_geo::operation::Operation;
use gs_protocol::{NO_VERTEX, PlainMesh, corners};
use gs_testkit::measure::stats;

/// An operand of a group.
fn op(mesh: &PlainMesh, at: Similarity, operation: Operation) -> Operand<'_> {
    Operand {
        mesh,
        at,
        operation,
    }
}

/// The group cut, or the test fails with the reason.
fn run(group: &[Operand<'_>]) -> Booleaned {
    boolean::fold(group).unwrap_or_else(|e| panic!("the cut failed: {e}"))
}

fn volume(mesh: &PlainMesh) -> f64 {
    stats(mesh).volume
}

/// The relative difference of `x` from `y`.
fn off(x: f64, y: f64) -> f64 {
    (x - y).abs() / y.abs()
}

/// The mesh's triangles, quads split as they are drawn.
fn tris(mesh: &PlainMesh) -> Vec<[u32; 3]> {
    mesh.triangles().collect()
}

/// Every face's corners as points.
fn face_points(mesh: &PlainMesh, f: usize) -> Vec<[f32; 3]> {
    corners(&mesh.faces[f])
        .iter()
        .map(|&v| mesh.positions[v as usize])
        .collect()
}

/// The operands' meshes laid end to end, as `Sources` index them, each point moved by its
/// operand's transform.
fn laid(group: &[Operand<'_>]) -> PlainMesh {
    let mut out = PlainMesh::default();
    for o in group {
        let base = out.positions.len() as u32;
        let moved = o
            .mesh
            .positions
            .iter()
            .map(|&p| o.at.point(gs_core::Vec3::from_array(p)).to_array());
        out.positions.extend(moved);
        let faces = o.mesh.faces.iter().map(|f| {
            let mut f = *f;
            for v in corners_mut(&mut f) {
                *v += base;
            }
            f
        });
        out.faces.extend(faces);
    }
    out
}

fn corners_mut(face: &mut [u32; 4]) -> &mut [u32] {
    let n = if face[3] == NO_VERTEX { 3 } else { 4 };
    &mut face[..n]
}
