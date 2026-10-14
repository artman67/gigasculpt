//! Hole closing (decision 0002, amended 2026-10-12): with 1, the default, each hole is capped
//! across its rim; with 0 the inside is read the same way, then the surface the caps made is
//! removed, so an open input stays open about where its rim was. A closed input remeshes the
//! same with either. Checked with the independent oracles in `common`.

use gs_geo::voxel::{CloseHoles, Remeshed, remesh, remesh_holes};
use gs_protocol::{AttrDomain, AttrValues, PlainMesh};
use gs_testkit::{Expect, validate};

use crate::common::*;
use crate::holes::rim;
use crate::shape::{assert_closed_manifold, settings};

/// `mesh` remeshed at resolution 24, closing holes as `close` says.
fn remeshed(mesh: &PlainMesh, close: CloseHoles) -> Remeshed {
    remesh_holes(mesh, &settings(24, false), close).unwrap_or_else(|e| panic!("remesh failed: {e}"))
}

/// The unit sphere open at the top, where y is 0.55 or more.
fn open_sphere() -> PlainMesh {
    without(&sphere([0.0; 3], 1.0, 12), |c| {
        c.iter().all(|p| p[1] >= 0.55)
    })
}

#[test]
fn hole_closing_0_leaves_a_hole_about_its_rim_where_1_closes_it() {
    let open = open_sphere();
    let (left, closed) = (
        remeshed(&open, CloseHoles::Off),
        remeshed(&open, CloseHoles::Large),
    );
    let h = left.grid.voxel;
    assert_closed_manifold(&closed, "hole closing 1");
    assert_eq!(
        left.work.holes, 1,
        "the inside is read with the hole capped"
    );
    assert_eq!(left.sign, closed.sign, "the same way as with 1");
    let expect = Expect {
        manifold: true,
        closed: false,
        euler: Some(1),
    };
    if let Err(defects) = validate(&left.mesh, expect) {
        panic!("hole closing 0: one open, manifold piece: {defects:?}");
    }
    assert_eq!(components(&left.mesh), 1, "one piece");
    // Its rim runs about where the input's did: within two voxels of it.
    let (input_rim, new_rim) = (rim(&open), rim(&left.mesh));
    assert!(!new_rim.is_empty(), "a rim");
    for p in new_rim.iter().flatten() {
        let off = (input_rim.iter())
            .map(|e| segment_distance(*p, e[0], e[1]))
            .fold(f64::MAX, f64::min);
        assert!(off <= 2.0 * h, "{p:?} is {} voxels off the rim", off / h);
    }
    // Nothing across the hole: every point within half a voxel of the input, or 1.5 within two
    // voxels of the rim, where the corner was rounded off before the cap's surface went.
    let near = Nearest::new(triangles(&open), h);
    for p in surface_points(&left.mesh) {
        let corner = (input_rim.iter()).any(|e| segment_distance(p, e[0], e[1]) < 2.0 * h);
        let allowed = if corner { 1.5 } else { 0.5 } * h;
        let off = near.distance(p);
        assert!(off <= allowed, "{p:?} is {} voxels off the input", off / h);
    }
    // And the input is kept as with 1: within half a voxel of the result away from the rim.
    let result = Nearest::new(triangles(&left.mesh), h);
    for p in surface_points(&open) {
        let corner = (input_rim.iter()).any(|e| segment_distance(p, e[0], e[1]) < 2.0 * h);
        let off = result.distance(p);
        assert!(corner || off <= 0.5 * h, "input point {p:?} is {off} off");
    }
    assert!(
        left.mesh.faces.len() < closed.mesh.faces.len(),
        "the patch is gone: {} faces against {}",
        left.mesh.faces.len(),
        closed.mesh.faces.len()
    );
}

#[test]
fn a_closed_input_remeshes_the_same_with_either_and_1_is_the_default() {
    let ball = sphere([0.0; 3], 1.0, 10);
    let (off, large) = (
        remeshed(&ball, CloseHoles::Off),
        remeshed(&ball, CloseHoles::Large),
    );
    let plain = remesh(&ball, &settings(24, false)).unwrap_or_else(|e| panic!("{e}"));
    for (what, out) in [("0", &off), ("remesh", &plain)] {
        assert_eq!(
            out.mesh.positions, large.mesh.positions,
            "{what}: positions"
        );
        assert_eq!(out.mesh.faces, large.mesh.faces, "{what}: faces");
    }
    let open = open_sphere();
    let default = remesh(&open, &settings(24, false)).unwrap_or_else(|e| panic!("{e}"));
    let capped = remeshed(&open, CloseHoles::Large);
    assert_eq!(default.mesh.faces, capped.mesh.faces, "remesh closes holes");
    assert_eq!(default.work, capped.work, "with the same work");
}

#[test]
fn hole_closing_0_gives_the_kept_surface_its_sources() {
    let open = open_sphere();
    let mut painted = open.clone();
    let mask = (0..open.positions.len()).map(|v| (v % 7) as f32 / 6.0);
    painted.attributes.insert(
        "mask".into(),
        column(AttrDomain::Vertex, AttrValues::F32(mask.collect())),
    );
    let (plain, carried) = (
        remeshed(&open, CloseHoles::Off),
        remeshed(&painted, CloseHoles::Off),
    );
    assert_eq!(carried.mesh.faces, plain.mesh.faces, "the same surface");
    assert_eq!(
        carried.mesh.positions, plain.mesh.positions,
        "in the same places"
    );
    let sources = &carried.sources;
    assert_eq!(
        sources.points.len(),
        carried.mesh.positions.len(),
        "a source per vertex"
    );
    assert_eq!(
        sources.faces.len(),
        carried.mesh.faces.len(),
        "and per face"
    );
    assert!(
        sources.faces.iter().all(Option::is_some),
        "only surface from the input is kept"
    );
}
