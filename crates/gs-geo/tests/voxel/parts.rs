//! `remesh_parts`: each part is a solid of its own, so which way a part faces cannot matter, a
//! cavity in a part stays hollow, the pieces of a split part are read as one, and parts facing
//! out remesh exactly as one mesh of them does.

use gs_geo::voxel::{RemeshError, Remeshed, SignMethod, remesh, remesh_parts};
use gs_protocol::PlainMesh;

use crate::common::*;
use crate::shape::{assert_closed_manifold, euler_is, run, settings};

/// `parts` merged in order, remeshed as parts.
fn parts(parts: &[&PlainMesh], resolution: u32) -> Remeshed {
    let all = (parts[1..].iter()).fold(parts[0].clone(), |all, p| merge(&all, p));
    let counts: Vec<usize> = parts.iter().map(|p| p.faces.len()).collect();
    remesh_parts(&all, &counts, &settings(resolution, false)).unwrap_or_else(|e| panic!("{e}"))
}

/// The farthest any coordinate moved between two meshes with the same vertices.
fn moved(a: &PlainMesh, b: &PlainMesh) -> f64 {
    (a.positions.iter().zip(&b.positions))
        .map(|(p, q)| norm(sub(wide(*p), wide(*q))))
        .fold(0.0, f64::max)
}

/// Asserts two remeshes are the same, bit for bit, counters included.
fn assert_same(a: &Remeshed, b: &Remeshed, what: &str) {
    assert!(a.mesh.faces == b.mesh.faces, "{what}: the same quads");
    assert!(
        a.mesh.positions == b.mesh.positions,
        "{what}: in the same places"
    );
    assert_eq!(a.sign, b.sign, "{what}: the same route");
    assert_eq!(a.work, b.work, "{what}: the same work");
}

#[test]
fn an_inside_out_part_fills_where_it_overlaps_another() {
    let a = sphere([0.0; 3], 1.0, 12);
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    // Not vacuous: one winding over both cancels to zero in the overlap, a sealed hollow.
    let overlap = [0.7, 0.15, -0.05];
    let summed = run(&merge(&flipped(&a), &b), 32, false);
    assert!(
        winding(&triangles(&summed.mesh), overlap).abs() < 0.5,
        "one winding: a hollow"
    );
    let out = parts(&[&a, &b], 32);
    let inverted = parts(&[&flipped(&a), &b], 32);
    assert_eq!(inverted.sign, SignMethod::Winding, "closed parts");
    assert_closed_manifold(&inverted, "inside out");
    assert!(
        winding(&triangles(&inverted.mesh), overlap) > 0.5,
        "the overlap is inside"
    );
    assert_eq!(components(&inverted.mesh), 1, "no hollow in the overlap");
    assert!(euler_is(&inverted.mesh, 2), "a sphere");
    assert!(inverted.mesh.faces == out.mesh.faces, "the same quads");
    let moved = moved(&inverted.mesh, &out.mesh);
    assert!(moved <= 1e-5, "the same places, to rounding: {moved}");
}

#[test]
fn a_cavity_in_a_part_stays_hollow_beside_another_part() {
    let hollow = merge(
        &sphere([0.0; 3], 1.0, 12),
        &flipped(&sphere([0.0; 3], 0.5, 8)),
    );
    let b = sphere([1.2, 0.0, 0.0], 0.5, 8);
    let out = parts(&[&hollow, &b], 32);
    assert_closed_manifold(&out, "hollow part and another");
    assert_eq!(components(&out.mesh), 2, "the outer wall and the cavity's");
    assert!(euler_is(&out.mesh, 4), "two spheres");
    let tris = triangles(&out.mesh);
    let at = |q: P| winding(&tris, q);
    assert!(at([0.0; 3]).abs() < 0.5, "the cavity is outside");
    assert!(at([0.75, 0.0, 0.0]) > 0.5, "the wall is inside");
    assert!(at([1.5, 0.0, 0.0]) > 0.5, "the other part is inside");
    assert_same(&out, &run(&merge(&hollow, &b), 32, false), "as one mesh");
}

#[test]
fn parts_facing_out_remesh_as_one_mesh_of_them_does() {
    let a = sphere([0.0; 3], 1.0, 12);
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    let open = without(&a, |c| c.iter().all(|p| p[1] >= 0.8));
    for (what, first) in [("closed", &a), ("open", &open)] {
        let out = parts(&[first, &b], 32);
        let one = run(&merge(first, &b), 32, false);
        assert_same(&out, &one, what);
    }
    assert_eq!(
        parts(&[&open, &b], 32).sign,
        SignMethod::CappedWinding,
        "capped"
    );
}

#[test]
fn the_pieces_of_a_split_part_are_read_as_one() {
    // A sphere cut in two along a wavy seam: each piece has a hole where it was cut, and only
    // together are they closed. Both keep every vertex, so the seam's points are shared.
    let whole = sphere([0.0; 3], 1.0, 12);
    let above = |f: &[u32; 4]| {
        let c = corners(&whole, f);
        let mid: P = std::array::from_fn(|a| c.iter().map(|p| p[a]).sum::<f64>() / 4.0);
        mid[1] > 0.3 * libm::sin(3.0 * mid[0]) + 0.2 * mid[2]
    };
    let piece = |keep: bool| PlainMesh {
        faces: (whole.faces.iter().filter(|f| above(f) == keep).copied()).collect(),
        ..whole.clone()
    };
    let (top, bottom) = (piece(true), piece(false));
    assert_eq!(
        run(&top, 8, false).sign,
        SignMethod::CappedWinding,
        "a piece is open"
    );
    let out = parts(&[&top, &bottom], 32);
    assert_eq!(out.sign, SignMethod::Winding, "closed together");
    let one = run(&whole, 32, false);
    assert!(
        out.mesh.faces == one.mesh.faces,
        "the part they were cut from"
    );
    assert!(
        out.mesh.positions == one.mesh.positions,
        "in the same places"
    );
}

#[test]
fn a_messy_part_takes_the_winding_number_and_the_others_keep_their_rows() {
    let a = sphere([0.0; 3], 1.0, 8);
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    let out = parts(&[&doubled(&a), &b], 24);
    assert_eq!(
        out.sign,
        SignMethod::GeneralizedWinding,
        "the most general route"
    );
    assert!(
        out.work.winding_terms > 0,
        "the messy part by winding number"
    );
    assert!(out.work.crossing_tests > 0, "the clean part by rows");
    assert_closed_manifold(&out, "messy and clean");
    assert_eq!(components(&out.mesh), 1, "one shell");
    assert!(euler_is(&out.mesh, 2), "a sphere");
    let clean = volume(&parts(&[&a, &b], 24).mesh);
    let got = volume(&out.mesh);
    assert!(
        (got / clean - 1.0).abs() <= 0.01,
        "volume {got} against {clean}"
    );
}

#[test]
fn parts_give_the_same_bits_on_1_3_and_24_threads() {
    // Rows of three solids rewritten into one count, and a winding number beside them, on a
    // grid of more rows than one task takes.
    let a = flipped(&sphere([0.0; 3], 1.0, 8));
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    let c = without(&sphere([-0.6, 0.5, 0.3], 0.6, 8), |q| {
        q.iter().all(|p| p[1] >= 0.9)
    });
    let d = doubled(&sphere([0.2, -0.8, 0.0], 0.5, 6));
    let runs: Vec<(Vec<u32>, String)> = [1, 3, 24]
        .into_iter()
        .map(|t| {
            gs_par::with_threads(t, || {
                let out = parts(&[&a, &b, &c, &d], 48);
                let mut bits: Vec<u32> = out
                    .mesh
                    .positions
                    .iter()
                    .flatten()
                    .map(|x| x.to_bits())
                    .collect();
                bits.extend(out.mesh.faces.iter().flatten());
                (bits, format!("{:?}", out.work))
            })
        })
        .collect();
    for (t, run) in [1, 3, 24].iter().zip(&runs) {
        assert!(run.0 == runs[0].0, "bits on {t} threads");
        assert_eq!(run.1, runs[0].1, "work on {t} threads");
    }
}

#[test]
fn face_counts_that_do_not_add_up_are_refused() {
    let cube = cuboid([0.0; 3], [1.0; 3], 1);
    for counts in [vec![5], vec![6, 1], vec![]] {
        let err = remesh_parts(&cube, &counts, &settings(8, false)).err();
        assert_eq!(err, Some(RemeshError::BadParts), "{counts:?}");
    }
    assert!(remesh(&cube, &settings(8, false)).is_ok(), "one part");
}
