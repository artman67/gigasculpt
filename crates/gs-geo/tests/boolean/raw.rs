//! The library called directly, for comparisons: operands imported as the fold imports a clean
//! one (its triangles as drawn, each with its index as face id), results ruled as the fold rules
//! them (epsilon raised to the export tolerance, then `set_tolerance`) or exported plainly.

use gs_core::transform::Similarity;
use gs_geo::boolean::TOLERANCE;
use gs_protocol::PlainMesh;
use manifold_rust::csg_tree::CsgLeafNode;
pub use manifold_rust::csg_tree::CsgNode as Node;
use manifold_rust::linalg::{Mat3x4, Vec3 as V64};
use manifold_rust::manifold::Manifold;
use manifold_rust::types::MeshGL64;
pub use manifold_rust::types::OpType;

/// The mesh imported as the fold imports a clean operand.
fn import(mesh: &PlainMesh) -> Manifold {
    let tris: Vec<[u32; 3]> = mesh.triangles().collect();
    let gl = MeshGL64 {
        num_prop: 3,
        vert_properties: mesh
            .positions
            .iter()
            .flatten()
            .map(|&x| f64::from(x))
            .collect(),
        tri_verts: tris.iter().flatten().map(|&v| u64::from(v)).collect(),
        face_id: (0..tris.len() as u64).collect(),
        ..MeshGL64::default()
    };
    Manifold::from_mesh_gl64(&gl)
}

/// The mesh as a leaf at `at`.
pub fn leaf(mesh: &PlainMesh, at: &Similarity) -> Node {
    let [x, y, z, w] = (at.matrix()).map(|c| V64::new(c[0].into(), c[1].into(), c[2].into()));
    let leaf = CsgLeafNode::with_transform(import(mesh).into_impl(), Mat3x4::from_cols(x, y, z, w));
    Node::leaf_node(leaf)
}

/// A manifold's points rounded to 32 bits and its triangles, read from its mesh as the fold
/// reads its result.
fn export(m: &Manifold) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let imp = m.as_impl();
    let positions = (imp.vert_pos.iter())
        .map(|p| [p.x as f32, p.y as f32, p.z as f32])
        .collect();
    let tris = (0..imp.num_tri())
        .map(|t| std::array::from_fn(|k| imp.halfedge[3 * t + k].start_vert.cast_unsigned()))
        .collect();
    (positions, tris)
}

/// The tree evaluated and rounded to 32 bits with no rule.
pub fn plain(tree: &Node) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    export(&Manifold::from_impl(tree.evaluate()))
}

/// The tree evaluated, ruled and rounded to 32 bits, as sorted triangle keys (`keys`).
pub fn cut(tree: &Node) -> Vec<[u32; 9]> {
    let result = Manifold::from_impl(tree.evaluate());
    let b = result.bounding_box();
    let d = b.max - b.min;
    let tolerance = TOLERANCE * (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
    let mut imp = result.as_impl().clone();
    imp.epsilon = imp.epsilon.max(tolerance);
    let (positions, tris) = export(&Manifold::from_impl(imp).set_tolerance(tolerance));
    keys(&positions, &tris)
}

/// Whether the library's scan finds the mesh crossing itself after a fresh import (`None` if
/// it does not import).
pub fn scan(mesh: &PlainMesh) -> Option<bool> {
    let m = import(mesh);
    (m.status() == manifold_rust::types::Error::NoError).then(|| m.has_self_intersections())
}

/// Each triangle's corner positions as bits, turned to start at the least, sorted: the same
/// keys for the same surface however its points and triangles are ordered.
pub fn keys(positions: &[[f32; 3]], tris: &[[u32; 3]]) -> Vec<[u32; 9]> {
    let mut keys: Vec<[u32; 9]> = (tris.iter())
        .map(|t| {
            let c = t.map(|v| positions[v as usize].map(f32::to_bits));
            let k = (0..3).min_by_key(|&k| c[k]).unwrap_or(0);
            let [a, b, d] = [c[k], c[(k + 1) % 3], c[(k + 2) % 3]];
            [a[0], a[1], a[2], b[0], b[1], b[2], d[0], d[1], d[2]]
        })
        .collect();
    keys.sort_unstable();
    keys
}
