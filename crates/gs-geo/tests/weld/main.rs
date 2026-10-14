//! gs-geo weld and repair tests (decision 0034, section 4): duplicate faces go, crowded edges
//! split until every edge has at most two faces, and the counts say what changed.

use gs_geo::weld::{exact, near, repair};
use gs_protocol::NO_VERTEX;

fn tri(a: u32, b: u32, c: u32) -> [u32; 4] {
    [a, b, c, NO_VERTEX]
}

/// The most faces any edge has.
fn most_faces_on_an_edge(faces: &[[u32; 4]]) -> usize {
    let mut edges: Vec<(u32, u32)> = faces
        .iter()
        .flat_map(|f| {
            let k = if f[3] == NO_VERTEX { 3 } else { 4 };
            (0..k).map(move |i| (f[i].min(f[(i + 1) % k]), f[i].max(f[(i + 1) % k])))
        })
        .collect();
    edges.sort_unstable();
    edges
        .chunk_by(|a, b| a == b)
        .map(<[_]>::len)
        .max()
        .unwrap_or(0)
}

#[test]
fn a_duplicate_pair_and_a_four_face_edge_come_out_with_two_faces_per_edge() {
    let faces = [
        // Four faces on the edge 0-1: two running 0 to 1, two running 1 to 0.
        tri(0, 1, 2),
        tri(1, 0, 3),
        tri(0, 1, 4),
        tri(1, 0, 5),
        // A fold: the same three points both ways round.
        tri(2, 6, 7),
        tri(2, 7, 6),
        // The same face twice.
        tri(3, 6, 7),
        tri(6, 7, 3),
    ];
    assert_eq!(most_faces_on_an_edge(&faces), 4);
    let out = repair(&faces, 8);
    assert_eq!(
        out.duplicate_faces, 3,
        "the fold's two faces and the repeated one"
    );
    assert_eq!(out.crowded_edges, 1);
    assert_eq!(out.from, [0, 1, 2, 3, 6]);
    // The second pair on the edge gets its own copies of points 0 and 1, as points 8 and 9.
    assert_eq!(out.copies, [0, 1]);
    assert_eq!(
        out.faces,
        [
            tri(0, 1, 2),
            tri(1, 0, 3),
            tri(8, 9, 4),
            tri(9, 8, 5),
            tri(3, 6, 7)
        ]
    );
    assert_eq!(most_faces_on_an_edge(&out.faces), 2);
}

#[test]
fn a_clean_mesh_comes_through_repair_unchanged() {
    let faces = [tri(0, 1, 2), tri(0, 2, 3), tri(0, 3, 1), tri(1, 3, 2)];
    let out = repair(&faces, 4);
    assert_eq!((out.duplicate_faces, out.crowded_edges), (0, 0));
    assert_eq!(out.copies, Vec::<u32>::new());
    assert_eq!(out.faces, faces);
}

#[test]
fn the_tolerance_weld_joins_points_within_reach_and_the_exact_one_only_equal_ones() {
    let points = [
        [0.0, 0.0, 0.0],
        [1e-4, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [-0.0, 0.0, 0.0],
        [1.0, 5e-4, 0.0],
    ];
    assert_eq!(exact(&points), [0, 1, 2, 0, 4]);
    assert_eq!(near(&points, 1e-3), [0, 0, 2, 0, 2]);
    assert_eq!(near(&points, 0.0), exact(&points));
}
