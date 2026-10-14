//! Hard borders (decision 0061 section 5): with `groups` on every edge between two groups is a
//! chart border, with `keep_seams` every seam of the given layout stays one, and non-manifold
//! edges and edges whose faces disagree on their winding are always borders.

use std::collections::BTreeMap;

use gs_geo::uv::{Corners, Steer, seams};
use gs_protocol::PlainMesh;
use gs_testkit::bench_shapes::{ring, sleeve};

use crate::{bit, charts, edges, groups};

#[test]
fn every_edge_between_two_groups_is_a_border() {
    let ring = ring(2000);
    let groups = groups(&ring);
    let steer = Steer {
        groups: Some(&groups),
        ..Steer::default()
    };
    let c = charts(&ring, &steer);
    let mut band = vec![None; c.count as usize];
    for (f, &k) in c.chart.iter().enumerate() {
        let first = *band[k as usize].get_or_insert(groups[f]);
        assert_eq!(first, groups[f], "chart {k} holds one band only");
    }
    for (edge, sides) in edges(&ring.faces) {
        let &[(f, j), (g, _)] = sides.as_slice() else {
            panic!("the ring is closed and manifold");
        };
        if groups[f] != groups[g] {
            assert!(
                bit(&c.seams, f, j),
                "edge {edge:?} between groups is a seam"
            );
        }
    }
}

/// UVs that put a seam on every edge between faces whose centres fall in different half-unit
/// slices along x: each corner reads its point's position and its face's slice.
fn slices(mesh: &PlainMesh) -> Vec<Corners> {
    let slice = |face: &[u32; 4]| {
        let x: f32 = face.iter().map(|&v| mesh.positions[v as usize][0]).sum();
        (x / 2.0).floor()
    };
    (mesh.faces.iter())
        .map(|face| {
            face.map(|v| {
                let p = mesh.positions[v as usize];
                [p[2] + 10.0 * slice(face), p[1]]
            })
        })
        .collect()
}

#[test]
fn every_kept_seam_stays_one_and_added_seams_counts_the_rest() {
    let mesh = sleeve(2000);
    let keep = slices(&mesh);
    let kept = seams(&mesh.faces, &keep);
    assert!(kept.iter().any(|&s| s != 0), "the layout has seams to keep");
    let steer = Steer {
        keep: Some(&keep),
        ..Steer::default()
    };
    let c = charts(&mesh, &steer);
    let mut added = 0;
    for (edge, sides) in edges(&mesh.faces) {
        let (f, k) = sides[0];
        if bit(&kept, f, k) {
            assert!(bit(&c.seams, f, k), "kept seam {edge:?} is still a seam");
        } else if bit(&c.seams, f, k) {
            added += 1;
        }
    }
    assert_eq!(
        c.added_seams, added,
        "added_seams counts the new seam edges"
    );
}

/// Three quads on one spine edge (a non-manifold edge), and two quads beside them that share
/// an edge but are wound against each other.
fn book() -> PlainMesh {
    let h = 0.866_025_4;
    let positions = vec![
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [-0.5, h, 0.0],
        [-0.5, h, 1.0],
        [-0.5, -h, 0.0],
        [-0.5, -h, 1.0],
        [5.0, 0.0, 0.0],
        [5.0, 0.0, 1.0],
        [6.0, 0.0, 0.0],
        [6.0, 0.0, 1.0],
        [7.0, 0.0, 0.0],
        [7.0, 0.0, 1.0],
    ];
    // The pages all run 0 to 1 along the spine; faces 3 and 4 both run 11 to 10.
    let faces = vec![
        [0, 1, 3, 2],
        [0, 1, 5, 4],
        [0, 1, 7, 6],
        [8, 9, 11, 10],
        [10, 12, 13, 11],
    ];
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

#[test]
fn non_manifold_and_miswound_edges_are_borders() {
    let mesh = book();
    let c = charts(&mesh, &Steer::default());
    for f in 0..3 {
        assert!(bit(&c.seams, f, 0), "page {f}'s spine edge is a seam");
    }
    let pages = [c.chart[0], c.chart[1], c.chart[2]];
    assert!(
        pages[0] != pages[1] && pages[1] != pages[2] && pages[0] != pages[2],
        "the three pages are three charts: {pages:?}"
    );
    assert_ne!(c.chart[3], c.chart[4], "miswound neighbours are two charts");
    assert!(
        bit(&c.seams, 3, 2) && bit(&c.seams, 4, 3),
        "their shared edge is a seam on both"
    );
    for (f, k) in [(0, 1), (0, 2), (0, 3), (3, 0), (4, 0)] {
        assert!(
            !bit(&c.seams, f, k),
            "open edge {k} of face {f} has nothing across"
        );
    }
}
