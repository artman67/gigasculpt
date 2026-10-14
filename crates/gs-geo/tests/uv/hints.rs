//! Hints (decision 0061 sections 5 and 6): protect is hard, so no seam runs between two
//! protected faces and a band the disc cut must cross is refused with its face count, a point
//! and the remedy; attract pulls a border onto the painted edges.

use gs_geo::uv::{Refusal, Steer, cut};
use gs_protocol::{PlainMesh, corners};
use gs_testkit::bench_shapes::{sleeve, sphere};

use crate::{bit, charts, edges, grid, surface};

/// Whether face `f` is protected under `hint`: the mean of its corners' values is -0.7 or below.
fn protected(mesh: &PlainMesh, hint: &[f32], f: usize) -> bool {
    let c = corners(&mesh.faces[f]);
    let sum: f32 = c.iter().map(|&v| hint[v as usize]).sum();
    sum / c.len() as f32 <= -0.7
}

/// Protect (-1) on every point with `inside` true, nothing elsewhere.
fn paint(mesh: &PlainMesh, inside: impl Fn([f32; 3]) -> bool) -> Vec<f32> {
    (mesh.positions.iter())
        .map(|&p| if inside(p) { -1.0 } else { 0.0 })
        .collect()
}

#[test]
fn no_seam_runs_between_two_protected_faces() {
    let mesh = sphere(2000);
    let hint = paint(&mesh, |p| p[1] > 0.2);
    let steer = Steer {
        hint: Some(&hint),
        ..Steer::default()
    };
    let c = charts(&mesh, &steer);
    let mut inside = 0;
    for (edge, sides) in edges(&mesh.faces) {
        let &[(f, j), (g, _)] = sides.as_slice() else {
            panic!("the sphere is closed and manifold");
        };
        if protected(&mesh, &hint, f) && protected(&mesh, &hint, g) {
            inside += 1;
            assert!(
                !bit(&c.seams, f, j),
                "edge {edge:?} inside the cap is no seam"
            );
        }
    }
    assert!(inside > 100, "the cap holds edges to test: {inside}");
}

#[test]
fn a_protected_band_the_cut_must_cross_is_refused_with_its_size_a_point_and_the_remedy() {
    let mesh = sleeve(2025);
    let hint = paint(&mesh, |p| p[1].abs() < 0.28);
    let band = (0..mesh.faces.len())
        .filter(|&f| protected(&mesh, &hint, f))
        .count();
    assert!(band > 0, "the band holds faces");
    let steer = Steer {
        hint: Some(&hint),
        ..Steer::default()
    };
    match cut(surface(&mesh), &steer) {
        Err(refusal @ Refusal::Protected { faces, point }) => {
            assert_eq!(faces, band, "the refusal counts the band's faces");
            assert!(point[1].abs() < 0.28, "the point {point:?} is on the band");
            let message = refusal.to_string();
            assert!(
                message.contains("erase protect along a strip"),
                "the message names the remedy: {message}"
            );
        }
        other => panic!("a closed protected band is refused, not {other:?}"),
    }
}

#[test]
fn attract_pulls_the_border_onto_the_painted_edges() {
    let mesh = grid(16);
    let plain = charts(&mesh, &Steer::default());
    assert_eq!(plain.count, 1, "a flat square is one chart");
    assert!(plain.seams.iter().all(|&s| s == 0), "and has no seams");

    let line = |p: [f32; 3]| p[0].to_bits() == 0.5_f32.to_bits();
    let hint: Vec<f32> = (mesh.positions.iter())
        .map(|&p| if line(p) { 1.0 } else { 0.0 })
        .collect();
    let steer = Steer {
        hint: Some(&hint),
        ..Steer::default()
    };
    let c = charts(&mesh, &steer);
    for (edge, sides) in edges(&mesh.faces) {
        let painted = edge.iter().all(|&v| line(mesh.positions[v as usize]));
        for &(f, k) in &sides {
            assert_eq!(
                bit(&c.seams, f, k),
                painted,
                "edge {edge:?} is a seam if painted"
            );
        }
    }
}
