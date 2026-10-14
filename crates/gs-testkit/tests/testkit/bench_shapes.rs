//! Research 28's four bench shapes are what they claim: closed or open as named, the right genus,
//! the ring's eight bands, the head's mirror, and research 28's sizes from its face counts.

use std::collections::BTreeSet;

use gs_protocol::{AttrValues, GROUP, NO_VERTEX, PlainMesh};
use gs_testkit::bench_shapes::{all, head, ring, sleeve, sphere};
use gs_testkit::measure::{norm, stats};
use gs_testkit::{Expect, validate};

fn all_quads(mesh: &PlainMesh) -> bool {
    mesh.faces.iter().all(|f| f[3] != NO_VERTEX)
}

fn radius(p: [f32; 3]) -> f64 {
    norm(p.map(f64::from))
}

#[test]
fn the_sphere_is_a_closed_quad_sphere() {
    let mesh = sphere(726);
    assert_eq!(mesh.faces.len(), 6 * 11 * 11, "research 28's base size");
    assert!(all_quads(&mesh), "quads only");
    assert_eq!(
        validate(&mesh, Expect::closed(2)),
        Ok(()),
        "a closed sphere"
    );
    let off = (mesh.positions.iter())
        .map(|&p| (radius(p) - 1.0).abs())
        .fold(0.0, f64::max);
    assert!(off < 1e-6, "every point on the unit sphere ({off})");
    assert!(stats(&mesh).volume > 4.0, "wound outward");
}

#[test]
fn the_head_is_a_closed_sphere_mirrored_in_x() {
    let mesh = head(726);
    assert_eq!(mesh.faces.len(), 726, "the sphere's lattice");
    assert!(all_quads(&mesh), "quads only");
    assert_eq!(
        validate(&mesh, Expect::closed(2)),
        Ok(()),
        "a closed sphere"
    );
    assert!(stats(&mesh).volume > 0.0, "wound outward");
    let bits = |p: [f32; 3]| p.map(f32::to_bits);
    let points: BTreeSet<[u32; 3]> = mesh.positions.iter().map(|&p| bits(p)).collect();
    for &[x, y, z] in &mesh.positions {
        assert!(
            points.contains(&bits([-x, y, z])),
            "({x}, {y}, {z}) has its mirror"
        );
    }
    let nose = mesh.positions.iter().map(|p| p[2]).fold(0.0, f32::max);
    assert!(
        nose > 1.0,
        "the nose passes the plain ellipsoid's 0.92: {nose}"
    );
}

#[test]
fn the_ring_is_a_closed_torus_of_eight_bands() {
    let mesh = ring(5000);
    let (m, k) = (156, 32);
    assert_eq!(
        mesh.faces.len(),
        m * k,
        "{m} stations of {k} profile points"
    );
    assert!(all_quads(&mesh), "quads only");
    assert_eq!(
        validate(&mesh, Expect::closed(0)),
        Ok(()),
        "a closed ring of genus 1"
    );
    assert!(stats(&mesh).volume > 0.0, "wound outward");
    let Some(AttrValues::U32(groups)) = mesh.attributes.get(GROUP).map(|a| &a.values) else {
        panic!("the ring carries a face group column");
    };
    let bands: BTreeSet<u32> = groups.iter().copied().collect();
    assert_eq!(bands, (1..=8).collect(), "one group per profile segment");
    for band in bands {
        let count = groups.iter().filter(|&&g| g == band).count();
        assert_eq!(
            count % m,
            0,
            "band {band} runs all the way round ({count} faces)"
        );
    }
    assert_eq!(ring(672).faces.len(), 56 * 12, "research 28's base size");
}

#[test]
fn the_sleeve_is_an_open_tube_with_two_rims() {
    let mesh = sleeve(784);
    assert_eq!(mesh.faces.len(), 28 * 28, "research 28's base size");
    assert!(all_quads(&mesh), "quads only");
    assert_eq!(validate(&mesh, Expect::open(0)), Ok(()), "an open tube");
    assert_eq!(stats(&mesh).boundary_edges, 2 * 28, "two rims of 28 edges");
}

#[test]
fn research_28s_hundred_thousand_triangle_row_comes_back_from_its_face_count() {
    let sizes: Vec<(&str, usize)> = all(50_000)
        .iter()
        .map(|(n, m)| (*n, m.faces.len()))
        .collect();
    let row = [
        ("sphere", 49_686),
        ("head", 49_686),
        ("ring", 50_000),
        ("sleeve", 50_176),
    ];
    assert_eq!(
        sizes, row,
        "sphere and head 91, ring 500 by 100, sleeve 224 by 224"
    );
}
