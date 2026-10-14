//! The fast generalized winding number against the exact sum (Girard's theorem, in `common`),
//! and how many terms its tree sums.

use std::collections::BTreeMap;

use gs_geo::voxel::winding_numbers;
use gs_protocol::PlainMesh;

use crate::common::*;

fn numbers(mesh: &PlainMesh, points: &[P]) -> Vec<(f64, u64)> {
    let points: Vec<[f32; 3]> = points.iter().map(|p| p.map(|c| c as f32)).collect();
    winding_numbers(mesh, &points).unwrap_or_else(|e| panic!("{e}"))
}

/// A lattice of points through and around a mesh of size about 1, some close to its surface.
fn lattice() -> Vec<P> {
    (0..729)
        .map(|i| {
            let c = [i % 9, i / 9 % 9, i / 81].map(|k| f64::from(k) * 0.33 - 1.27);
            [c[0], c[1] * 1.05, c[2] * 0.95]
        })
        .collect()
}

#[test]
fn the_fast_winding_number_matches_the_exact_one() {
    let closed = sphere([0.1, -0.2, 0.3], 1.0, 8);
    let open = without(&closed, |c| c.iter().all(|p| p[1] >= 0.2));
    let ring = torus(1.0, 0.35, 24, 12);
    let points = lattice();
    for (what, mesh) in [("sphere", closed), ("open sphere", open), ("torus", ring)] {
        let tris = triangles(&mesh);
        let fast = numbers(&mesh, &points);
        let mut worst = 0.0_f64;
        for (p, &(w, terms)) in points.iter().zip(&fast) {
            worst = worst.max((w - winding(&tris, *p)).abs());
            assert!(
                terms >= 1 && terms <= tris.len() as u64,
                "{what}: {terms} terms"
            );
        }
        // First-order clusters at three times their radius: about 2% at worst (the torus),
        // plenty for an inside-or-outside decision at one half.
        assert!(worst <= 0.025, "{what}: off by {worst}");
    }
}

#[test]
fn the_tree_saves_work_on_a_finer_mesh() {
    // Faces shuffled, so only the tree's own spatial sort can make its clusters compact.
    let mut mesh = sphere([0.1, -0.2, 0.3], 1.0, 24);
    let n = mesh.faces.len();
    mesh.faces = (0..n).map(|f| mesh.faces[f * 7919 % n]).collect();
    let points = lattice();
    let total: u64 = numbers(&mesh, &points).iter().map(|f| f.1).sum();
    let brute = 2 * mesh.faces.len() as u64 * points.len() as u64;
    assert!(
        total * 4 <= brute,
        "{total} terms against {brute} for brute force"
    );
}

/// A ribbon along x from 0 to 8, 0.01 wide: 8 quads, 16 triangles in x order.
fn ribbon() -> PlainMesh {
    let mut positions = Vec::new();
    for i in 0..=8 {
        positions.push([i as f32, 0.0, 0.0]);
        positions.push([i as f32, 0.01, 0.0]);
    }
    let faces = (0..8u32)
        .map(|i| [2 * i, 2 * i + 2, 2 * i + 3, 2 * i + 1])
        .collect();
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

#[test]
fn the_tree_sums_far_clusters_as_one_term_and_near_ones_exactly() {
    // Leaves hold up to 8 triangles, so the ribbon is a root over two leaves: x from 0 to 4
    // (centre 2, radius about 2) and x from 4 to 8 (centre 6). A cluster counts as one dipole
    // when the point is more than three times its radius from its centre.
    let mesh = ribbon();
    let cases = [
        // Far from everything: the root's dipole.
        ([100.0, 0.0, 0.0], 1),
        // Near the first leaf (8 exact), more than 6 from the second's centre (1 dipole).
        ([-0.5, 0.005, 0.3], 9),
        // Between the two: both leaves exact.
        ([4.0, 0.005, 0.2], 16),
    ];
    for (p, terms) in cases {
        let (_, got) = numbers(&mesh, &[p])[0];
        assert_eq!(got, terms, "terms at {p:?}");
    }
}
