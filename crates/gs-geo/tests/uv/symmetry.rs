//! Symmetry (decision 0061 section 7): under a mirror axis the charts and seams mirror edge for
//! edge, on a head whose mirror plane runs along edges (an even lattice) and on one whose faces
//! straddle it (an odd lattice), with a protected area across the plane too; a point without a
//! partner refuses with the count.

use std::collections::BTreeMap;

use gs_geo::uv::{Charts, Refusal, Steer, Symmetry, cut};
use gs_protocol::{PlainMesh, corners};
use gs_testkit::bench_shapes::head;

use crate::{bit, charts, edges, surface};

/// The mirror in x the cutter is given, at decision 0061's 1e-4 of the part's diagonal.
pub(crate) fn in_x(mesh: &PlainMesh) -> Symmetry {
    let (lo, hi) = (mesh.positions.iter()).fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| {
        (
            std::array::from_fn(|k| lo[k].min(p[k])),
            std::array::from_fn(|k| hi[k].max(p[k])),
        )
    });
    let d: f32 = (0..3).map(|k| (hi[k] - lo[k]) * (hi[k] - lo[k])).sum();
    Symmetry {
        axis: 0,
        tolerance: 1e-4 * d.sqrt(),
    }
}

/// Each point's exact mirror in x (`-0` read as `0`), which the head has to the bit.
pub(crate) fn partners(mesh: &PlainMesh) -> Vec<u32> {
    let key = |p: [f32; 3]| p.map(|c| (c + 0.0).to_bits());
    let at: BTreeMap<[u32; 3], u32> = (mesh.positions.iter().enumerate())
        .map(|(v, &p)| (key(p), v as u32))
        .collect();
    (mesh.positions.iter())
        .map(|&[x, y, z]| at[&key([-x, y, z])])
        .collect()
}

/// The charts under `steer`, checked to mirror edge for edge.
fn assert_mirrored(mesh: &PlainMesh, steer: &Steer) -> Charts {
    let c = charts(mesh, steer);
    let partner = partners(mesh);
    let mut sorted: BTreeMap<Vec<u32>, usize> = BTreeMap::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let mut key = corners(face).to_vec();
        key.sort_unstable();
        sorted.insert(key, f);
    }
    let mut twin = vec![None; c.count as usize];
    for (f, face) in mesh.faces.iter().enumerate() {
        let c_f = corners(face);
        let mut key: Vec<u32> = c_f.iter().map(|&v| partner[v as usize]).collect();
        key.sort_unstable();
        let g = sorted[&key];
        let mapped = *twin[c.chart[f] as usize].get_or_insert(c.chart[g]);
        assert_eq!(
            mapped, c.chart[g],
            "face {f}'s chart mirrors onto face {g}'s"
        );
        let c_g = corners(&mesh.faces[g]);
        for k in 0..c_f.len() {
            // The edge from corner k to k + 1 mirrors onto the edge g runs the other way.
            let (a, b) = (
                partner[c_f[(k + 1) % c_f.len()] as usize],
                partner[c_f[k] as usize],
            );
            let j = (0..c_g.len())
                .find(|&j| c_g[j] == a && c_g[(j + 1) % c_g.len()] == b)
                .unwrap_or_else(|| panic!("face {g} runs face {f}'s edge {k} mirrored"));
            assert_eq!(
                bit(&c.seams, f, k),
                bit(&c.seams, g, j),
                "face {f}'s edge {k} and face {g}'s edge {j} are both seams or neither"
            );
        }
    }
    c
}

fn mirrored(mesh: &PlainMesh) -> Steer<'static> {
    Steer {
        symmetry: Some(in_x(mesh)),
        ..Steer::default()
    }
}

#[test]
fn charts_and_seams_mirror_edge_for_edge_with_the_plane_along_edges() {
    let mesh = head(1944);
    assert!(
        mesh.positions.iter().any(|p| p[0] == 0.0),
        "the even head has points on its plane"
    );
    assert_mirrored(&mesh, &mirrored(&mesh));
}

#[test]
fn charts_and_seams_mirror_edge_for_edge_with_faces_across_the_plane() {
    let mesh = head(2166);
    assert!(
        mesh.positions.iter().all(|p| p[0] != 0.0),
        "the odd head's plane runs through faces"
    );
    assert_mirrored(&mesh, &mirrored(&mesh));
}

#[test]
fn a_protected_area_across_the_plane_is_one_chart_and_mirrors_onto_itself() {
    for faces in [1944, 2166] {
        let mesh = head(faces);
        // Protect the front of the face, which the plane runs through.
        let hint: Vec<f32> = (mesh.positions.iter())
            .map(|p| if p[2] > 0.7 { -1.0 } else { 0.0 })
            .collect();
        let steer = Steer {
            hint: Some(&hint),
            ..mirrored(&mesh)
        };
        let c = assert_mirrored(&mesh, &steer);
        let protected = |f: usize| {
            let c = corners(&mesh.faces[f]);
            c.iter().map(|&v| hint[v as usize]).sum::<f32>() / c.len() as f32 <= -0.7
        };
        let front: Vec<usize> = (0..mesh.faces.len()).filter(|&f| protected(f)).collect();
        assert!(
            front.iter().any(|&f| mesh.faces[f]
                .iter()
                .any(|&v| mesh.positions[v as usize][0] < 0.0))
                && front.iter().any(|&f| mesh.faces[f]
                    .iter()
                    .any(|&v| mesh.positions[v as usize][0] > 0.0)),
            "{faces}: the protected front reaches both sides"
        );
        assert!(
            front.iter().all(|&f| c.chart[f] == c.chart[front[0]]),
            "{faces}: the protected front is one chart"
        );
        for (edge, sides) in edges(&mesh.faces) {
            let &[(f, j), (g, _)] = sides.as_slice() else {
                panic!("the head is closed and manifold");
            };
            if protected(f) && protected(g) {
                assert!(
                    !bit(&c.seams, f, j),
                    "{faces}: edge {edge:?} inside the front is no seam"
                );
            }
        }
    }
}

#[test]
fn a_point_without_a_partner_is_refused_with_the_count() {
    let mut mesh = head(1944);
    let moved = (mesh.positions.iter())
        .position(|p| p[0] > 0.3)
        .unwrap_or_else(|| panic!("the head reaches past x = 0.3"));
    mesh.positions[moved][0] += 1e-3;
    let steer = Steer {
        symmetry: Some(in_x(&mesh)),
        ..Steer::default()
    };
    match cut(surface(&mesh), &steer) {
        Err(refusal @ Refusal::Unpaired { count, .. }) => {
            assert_eq!(count, 2, "the moved point and its old partner have none");
            let message = refusal.to_string();
            assert!(
                message.contains("symmetry off"),
                "the message names a remedy: {message}"
            );
        }
        other => panic!("an asymmetric head is refused, not {other:?}"),
    }
}
