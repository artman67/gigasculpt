//! The subdivision numbering shared with ZTL files (decision 0006). Kinds from
//! ARCHITECTURE 10.1: faces worked out by hand from the rule, and differential tests against a
//! slow, obviously correct version of the rule on random meshes (shared edges in both
//! directions, high-valence vertices, meshes past one bucket batch).

use std::collections::BTreeMap;

use gs_core::{NO, level_vertex_counts, subdivide_corners, subdivide_faces};
use gs_testkit::SplitMix64;

/// The rule, slowly: a map from each edge to its number, filled in face and corner order.
/// Also returns each new vertex's sources (an edge's ends, or a face's corners).
fn reference(faces: &[[u32; 4]], vertex_count: u32) -> (Vec<[u32; 4]>, u32, Vec<Vec<u32>>) {
    let mut edge: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut sources = Vec::new();
    let mut out = Vec::new();
    for f in faces {
        let c: Vec<u32> = f.iter().copied().filter(|&v| v != NO).collect();
        let n = c.len();
        let mut e = Vec::new();
        for i in 0..n {
            let key = (c[i].min(c[(i + 1) % n]), c[i].max(c[(i + 1) % n]));
            e.push(*edge.entry(key).or_insert_with(|| {
                sources.push(vec![key.0, key.1]);
                vertex_count + sources.len() as u32 - 1
            }));
        }
        sources.push(c.clone());
        let m = vertex_count + sources.len() as u32 - 1;
        for k in 0..n {
            let mut q = [c[k], e[k], m, e[(k + n - 1) % n]];
            q.rotate_right(if k + 1 == n { 3 } else { k });
            out.push(q);
        }
    }
    (out, vertex_count + sources.len() as u32, sources)
}

fn reference_faces(faces: &[[u32; 4]], vertex_count: u32) -> (Vec<[u32; 4]>, u32) {
    let (out, count, _) = reference(faces, vertex_count);
    (out, count)
}

/// Random triangles and quads over `n` vertices, distinct corners, some vertices shared by
/// many faces.
fn random_faces(rng: &mut SplitMix64, n: usize, count: usize) -> Vec<[u32; 4]> {
    (0..count)
        .map(|_| {
            let mut corners: Vec<u32> = Vec::new();
            let want = 3 + rng.below(2).min(n - 3);
            while corners.len() < want {
                // A third of the corners come from the first few vertices: high valence.
                let v = if rng.below(3) == 0 {
                    rng.below(4.min(n))
                } else {
                    rng.below(n)
                } as u32;
                if !corners.contains(&v) {
                    corners.push(v);
                }
            }
            corners.resize(4, NO);
            [corners[0], corners[1], corners[2], corners[3]]
        })
        .collect()
}

/// A grid of `w` by `h` quads.
fn grid(w: u32, h: u32) -> (Vec<[u32; 4]>, u32) {
    let mut faces = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let corner = y * (w + 1) + x;
            faces.push([corner, corner + 1, corner + w + 2, corner + w + 1]);
        }
    }
    (faces, (w + 1) * (h + 1))
}

#[test]
fn one_quad_and_one_triangle_split_as_the_rule_says() {
    assert_eq!(
        subdivide_faces(&[[0, 1, 2, 3]], 4),
        Some((
            vec![[0, 4, 8, 7], [4, 1, 5, 8], [8, 5, 2, 6], [7, 8, 6, 3]],
            9
        ))
    );
    // Edges 0-1, 1-2 and 2-0 become 3, 4 and 5; the center is 6.
    assert_eq!(
        subdivide_faces(&[[0, 1, 2, NO]], 3),
        Some((vec![[0, 3, 6, 5], [3, 1, 4, 6], [5, 6, 4, 2]], 7))
    );
    // Two quads sharing the edge 1-2 (walked 1 to 2 by the first, 2 to 1 by the second): it
    // is numbered once, by the first face, and the second face's center follows its own new
    // edges.
    let (faces, count) = subdivide_faces(&[[0, 1, 2, 3], [1, 4, 5, 2]], 6).unwrap();
    assert_eq!(count, 6 + 7 + 2);
    assert_eq!(
        faces[4..],
        [
            [1, 11, 14, 7],
            [11, 4, 12, 14],
            [14, 12, 5, 13],
            [7, 14, 13, 2]
        ]
    );
}

#[test]
fn bad_faces_and_empty_meshes() {
    assert_eq!(
        subdivide_faces(&[[0, 1, 2, 7]], 7),
        None,
        "index at the count"
    );
    assert_eq!(
        subdivide_faces(&[[0, 1, 9, 2]], 7),
        None,
        "index past the count"
    );
    assert_eq!(
        subdivide_faces(&[[NO, 1, 2, 3]], 4),
        None,
        "a missing first corner"
    );
    assert_eq!(subdivide_faces(&[], 5), Some((vec![], 5)));
    assert_eq!(level_vertex_counts(&[[0, 1, 2, 7]], 7, 2), None);
    assert_eq!(level_vertex_counts(&[], 5, 2), Some(vec![5, 5, 5]));
}

#[test]
fn random_meshes_match_the_rule_and_the_level_counts() {
    let mut rng = SplitMix64::new(6);
    for round in 0..40 {
        let n = 3 + rng.below(if round < 30 { 12 } else { 6000 });
        let count = 1 + rng.below(n * 2);
        let faces = random_faces(&mut rng, n, count);
        let mut level = (faces.clone(), n as u32);
        let mut counts = vec![u64::from(level.1)];
        for depth in 0..3 {
            let fast = subdivide_faces(&level.0, level.1).unwrap();
            assert_eq!(
                fast,
                reference_faces(&level.0, level.1),
                "round {round}, depth {depth}"
            );
            counts.push(u64::from(fast.1));
            level = fast;
        }
        assert_eq!(
            level_vertex_counts(&faces, n as u32, 3),
            Some(counts),
            "round {round}"
        );
    }
}

#[test]
fn a_big_grid_matches_the_rule() {
    let (faces, count) = grid(300, 220);
    let fast = subdivide_faces(&faces, count).unwrap();
    assert_eq!(fast, reference_faces(&faces, count));
    // V + E + F: 301 x 221 vertices, 300 x 221 + 301 x 220 edges, 300 x 220 faces.
    assert_eq!(fast.1, 301 * 221 + 300 * 221 + 301 * 220 + 300 * 220);
}

#[test]
fn corner_values_follow_the_new_corners() {
    // Corner values that are a linear function of the corner's vertex position stay one after
    // a step, with the new vertices at edge middles and face centers.
    let mut rng = SplitMix64::new(9);
    let faces = random_faces(&mut rng, 40, 60);
    let (children, _, sources) = reference(&faces, 40);
    let mut pos: Vec<[f64; 2]> = (0..40)
        .map(|_| [rng.below(64) as f64 / 8.0, rng.below(64) as f64 / 8.0])
        .collect();
    for s in &sources {
        let n = s.len() as f64;
        let p = s.iter().fold([0.0, 0.0], |a, &v| {
            [a[0] + pos[v as usize][0] / n, a[1] + pos[v as usize][1] / n]
        });
        pos.push(p);
    }
    let value = |v: u32| {
        let p = pos[v as usize];
        [2.0 * p[0] - p[1], p[0] + 0.5 * p[1]]
    };
    let values: Vec<[[f32; 2]; 4]> = (faces.iter())
        .map(|f| {
            f.map(|v| {
                if v == NO {
                    [9.0; 2]
                } else {
                    value(v).map(|x| x as f32)
                }
            })
        })
        .collect();
    let mut out = Vec::new();
    let corners = |f: usize| if faces[f][3] == NO { 3 } else { 4 };
    subdivide_corners(&values, corners, |c| out.push(c));
    assert_eq!(out.len(), children.len());
    for (c, (face, got)) in children.iter().zip(&out).enumerate() {
        for k in 0..4 {
            let want = value(face[k]);
            let close = (0..2).all(|i| (f64::from(got[k][i]) - want[i]).abs() < 1e-5);
            assert!(close, "child {c} corner {k}: {:?} against {want:?}", got[k]);
        }
    }
}
