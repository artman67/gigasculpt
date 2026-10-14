//! `repair_operand` (decision 0057 section 2, repair 5): an open or messy operand voxel remeshed
//! alone with its holes closed, turned right way out first when its capped volume is negative.

use gs_geo::voxel::{Repaired, SignMethod, repair_operand};
use gs_protocol::{AttrDomain, AttrValues, NO_VERTEX, PlainMesh};

use crate::common::*;
use crate::fold::bits;
use crate::shape::{assert_closed_manifold, euler_is, run, settings};

fn repair(mesh: &PlainMesh, resolution: u32) -> Repaired {
    repair_operand(mesh, &settings(resolution, false)).unwrap_or_else(|e| panic!("{e}"))
}

/// `mesh` with a fin: a triangle on its first face's first edge, standing a twentieth of the
/// edge's distance from the middle out past it.
fn finned(mut mesh: PlainMesh) -> PlainMesh {
    let [a, b] = [0, 1].map(|k| mesh.positions[mesh.faces[0][k] as usize]);
    mesh.positions
        .push(std::array::from_fn(|i| 1.05 * (a[i] + b[i]) / 2.0));
    let tip = (mesh.positions.len() - 1) as u32;
    mesh.faces
        .push([mesh.faces[0][0], mesh.faces[0][1], tip, NO_VERTEX]);
    mesh
}

#[test]
fn an_open_operand_comes_out_closed_whichever_way_it_faces() {
    let open = without(&sphere([0.0; 3], 1.0, 12), |q| {
        q.iter().all(|p| p[1] >= 0.8)
    });
    let right = repair(&open, 32);
    let wrong = repair(&flipped(&open), 32);
    for (out, turned, what) in [(&right, false, "facing out"), (&wrong, true, "inside out")] {
        assert_eq!(out.turned, turned, "{what}: turned");
        assert_eq!(
            out.remeshed.sign,
            SignMethod::CappedWinding,
            "{what}: capped"
        );
        assert_closed_manifold(&out.remeshed, what);
        assert!(euler_is(&out.remeshed.mesh, 2), "{what}: a sphere");
    }
    // The rim is not flat, and its cap's ears are cut from the other end of the loop when the
    // operand is turned over, so the two caps differ a little (0.5 percent of the volume here).
    let (a, b) = (volume(&right.remeshed.mesh), volume(&wrong.remeshed.mesh));
    assert!((a / b - 1.0).abs() <= 0.01, "the same solid: {a} and {b}");
}

#[test]
fn an_inside_out_messy_operand_is_turned_right_way_out() {
    // Item 127: read as it is, an inside-out messy part winds -1 inside and reads empty.
    let ball = sphere([0.0; 3], 1.0, 8);
    let inverted = flipped(&doubled(&ball));
    assert!(
        run(&inverted, 32, false).mesh.faces.is_empty(),
        "today it reads empty"
    );
    let mut painted = inverted.clone();
    let n = painted.positions.len();
    let mask = (0..n).map(|v| v as f32 / n as f32).collect();
    painted.attributes.insert(
        "mask".into(),
        column(AttrDomain::Vertex, AttrValues::F32(mask)),
    );
    let out = repair(&painted, 32);
    assert!(out.turned, "turned");
    assert_eq!(out.remeshed.sign, SignMethod::GeneralizedWinding, "messy");
    assert_closed_manifold(&out.remeshed, "turned");
    let want = volume(&run(&ball, 32, false).mesh);
    let got = volume(&out.remeshed.mesh);
    assert!(
        (got / want - 1.0).abs() <= 0.01,
        "volume {got} against {want}"
    );
    let points = &out.remeshed.sources.points;
    assert_eq!(
        points.len(),
        out.remeshed.mesh.positions.len(),
        "a source per point"
    );
    assert!(
        points
            .iter()
            .flatten()
            .all(|(t, _)| t.iter().all(|&v| (v as usize) < n)),
        "sources name the operand's own points"
    );
    let right = repair(&doubled(&ball), 32);
    assert!(!right.turned, "a messy part facing out is left as it is");
    assert!(
        right.remeshed.mesh.faces == out.remeshed.mesh.faces,
        "the same quads"
    );
}

#[test]
fn a_fin_is_read_by_the_winding_number_and_adds_no_volume() {
    // Decision 0057's open question measured a 1M sphere with a fin at 256 within the time, so
    // the fin stays and the generalized winding number reads the whole operand, whichever way
    // it faces.
    let ball = sphere([0.0; 3], 1.0, 12);
    let want = volume(&run(&ball, 32, false).mesh);
    let fin = finned(ball);
    for (mesh, turned, what) in [
        (&fin, false, "facing out"),
        (&flipped(&fin), true, "inside out"),
    ] {
        let out = repair(mesh, 32);
        assert_eq!(out.turned, turned, "{what}: turned");
        assert_eq!(
            out.remeshed.sign,
            SignMethod::GeneralizedWinding,
            "{what}: messy"
        );
        assert_closed_manifold(&out.remeshed, what);
        let got = volume(&out.remeshed.mesh);
        assert!(
            (got / want - 1.0).abs() <= 0.01,
            "{what}: volume {got} against {want}"
        );
    }
}

#[test]
fn repair_gives_the_same_bits_on_1_3_and_24_threads() {
    let open = without(&sphere([0.0; 3], 1.0, 12), |q| {
        q.iter().all(|p| p[1] >= 0.8)
    });
    let messy = flipped(&doubled(&finned(open)));
    let runs: Vec<(Vec<u32>, String, bool)> = [1, 3, 24]
        .into_iter()
        .map(|t| {
            gs_par::with_threads(t, || {
                let out = repair_operand(&messy, &settings(48, true)).unwrap();
                let (bits, rest) = bits(&out.remeshed);
                (bits, rest, out.turned)
            })
        })
        .collect();
    assert!(
        runs[0].2 && !runs[0].0.is_empty(),
        "turned, and something to compare"
    );
    for (t, run) in [1, 3, 24].iter().zip(&runs) {
        assert!(run.0 == runs[0].0, "bits on {t} threads");
        assert_eq!(run.1, runs[0].1, "work on {t} threads");
    }
}
