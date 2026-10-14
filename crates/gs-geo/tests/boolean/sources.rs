//! Where every output face and point came from (decision 0057, section 6): each face names an
//! input face it lies on, each point a triangle and weights that put it where it is, the earlier
//! operand owns a point on the seam, a point bit-equal to a corner is that corner, and input
//! quads that come through untouched come back whole.

use std::collections::BTreeMap;

use gs_core::Vec3;
use gs_core::math::closest_on_triangle;
use gs_core::transform::Similarity;
use gs_geo::boolean::Booleaned;
use gs_geo::operation::Operation::{Difference, Intersect, Union};
use gs_protocol::{PlainMesh, arity, corners, face_triangles};

use crate::shapes::{cuboid, cutter, small_b, sphere};
use crate::{face_points, laid, op, run};

const I: Similarity = Similarity::IDENTITY;

fn v3(p: [f32; 3]) -> Vec3 {
    Vec3::from_array(p)
}

/// The distance from `p` to input face `f` of `mesh`.
fn to_face(mesh: &PlainMesh, f: usize, p: Vec3) -> f32 {
    let face = mesh.faces[f];
    let at = |k: usize| mesh.positions[face[k] as usize];
    (face_triangles(face, at).iter())
        .map(|t| {
            let c = t.map(|k| v3(at(k)));
            (closest_on_triangle(p, &c).0 - p).len2().sqrt()
        })
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn every_output_face_and_point_names_its_source() {
    let ball = sphere(10);
    for operation in [Union, Difference, Intersect] {
        let what = format!("{operation:?}");
        let group = [op(&ball, I, Union), op(&ball, small_b(), operation)];
        let out = run(&group);
        let input = laid(&group);
        let (first_point, first_face) = (ball.positions.len() as u32, ball.faces.len() as u32);
        assert_eq!(
            out.sources.faces.len(),
            out.mesh.faces.len(),
            "{what}: a source a face"
        );
        assert_eq!(
            out.sources.points.len(),
            out.mesh.positions.len(),
            "{what}: a point"
        );
        // Each face names an input face, and each triangle it is drawn as lies on it (a quad's
        // own centre need not: a quad may bend along its diagonal).
        let mut lowest = vec![u32::MAX; out.mesh.positions.len()];
        for (f, source) in out.sources.faces.iter().enumerate() {
            let Some(g) = *source else {
                panic!("{what}: face {f} names no source")
            };
            assert!(
                (g as usize) < input.faces.len(),
                "{what}: face {f} names face {g}"
            );
            let face = out.mesh.faces[f];
            let at = |k: usize| v3(out.mesh.positions[face[k] as usize]);
            for t in face_triangles(face, |k| at(k).to_array()) {
                let centre = (at(t[0]) + at(t[1]) + at(t[2])) * (1.0 / 3.0);
                let d = to_face(&input, g as usize, centre);
                assert!(d < 1e-5, "{what}: face {f} lies {d} off its source {g}");
            }
            let operand = u32::from(g >= first_face);
            for &v in corners(&out.mesh.faces[f]) {
                lowest[v as usize] = lowest[v as usize].min(operand);
            }
        }
        // Each point lies where its weights put it in its source triangle, of the earlier
        // operand among its faces' (the seam rule).
        for (v, source) in out.sources.points.iter().enumerate() {
            let Some((tri, w)) = *source else {
                panic!("{what}: point {v} names no source")
            };
            let operand = u32::from(tri[0] >= first_point);
            let one = tri.iter().all(|&i| u32::from(i >= first_point) == operand);
            assert!(
                one && (tri[2] as usize) < input.positions.len(),
                "{what}: {tri:?}"
            );
            let sum: f32 = w.iter().sum();
            let weights_ok = w.iter().all(|&x| x >= 0.0) && (sum - 1.0).abs() < 1e-5;
            assert!(weights_ok, "{what}: point {v} weights {w:?}");
            let at = (tri.iter().zip(w)).fold(Vec3::ZERO, |s, (&i, x)| {
                s + v3(input.positions[i as usize]) * x
            });
            let d = (at - v3(out.mesh.positions[v])).len2().sqrt();
            assert!(
                d < 1e-5,
                "{what}: point {v} lies {d} from where its weights put it"
            );
            assert_eq!(
                operand, lowest[v],
                "{what}: point {v} belongs to the earlier operand"
            );
        }
        assert_base_corners_are_corners(&ball, &out, &what);
    }
}

/// A point bit-equal to a corner of the base (`base`, the group's first operand, unmoved) is
/// that corner.
fn assert_base_corners_are_corners(base: &PlainMesh, out: &Booleaned, what: &str) {
    let corners: BTreeMap<[u32; 3], u32> = (base.positions.iter().enumerate())
        .map(|(i, p)| (p.map(f32::to_bits), i as u32))
        .collect();
    let mut kept = 0;
    for (v, p) in out.mesh.positions.iter().enumerate() {
        let Some(&i) = corners.get(&p.map(f32::to_bits)) else {
            continue;
        };
        kept += 1;
        let corner = out.sources.points[v].and_then(|(tri, w)| {
            let k = tri.iter().position(|&t| t == i)?;
            Some(w[k].to_bits() == 1.0_f32.to_bits())
        });
        assert_eq!(
            corner,
            Some(true),
            "{what}: point {v} is corner {i} of the base"
        );
    }
    // Not a vacuous check: even the intersection's small cap keeps dozens of base corners.
    assert!(kept > 30, "{what}: base corners kept: {kept}");
}

#[test]
fn untouched_quads_come_back_whole() {
    // Two boxes apart: every face comes back a quad, corner for corner.
    let (a, b) = (
        cuboid([0.0; 3], [1.0; 3]),
        cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]),
    );
    let group = [op(&a, I, Union), op(&b, I, Union)];
    let out = run(&group);
    let input = laid(&group);
    assert_eq!(out.mesh.faces.len(), 12, "twelve faces");
    for f in 0..12 {
        let g = out.sources.faces[f].map_or(usize::MAX, |g| g as usize);
        assert!(g < 12, "face {f} names a face");
        let same = face_points(&out.mesh, f) == face_points(&input, g);
        assert!(same, "face {f} is face {g} whole");
    }
    // A ball with a bite out of its side: the quads away from the bite come back whole.
    let (ball, bite) = (sphere(10), sphere(8));
    let group = [op(&ball, I, Union), op(&bite, cutter(), Difference)];
    let out = run(&group);
    let input = laid(&group);
    let mut whole = 0;
    for f in 0..out.mesh.faces.len() {
        let g = out.sources.faces[f].map_or(usize::MAX, |g| g as usize);
        if arity(out.mesh.faces[f]) == 4 && g < ball.faces.len() {
            whole += 1;
            let same = face_points(&out.mesh, f) == face_points(&input, g);
            assert!(same, "quad {f} is face {g} whole, corner for corner");
        }
    }
    let share = f64::from(whole) / ball.faces.len() as f64;
    assert!(
        share > 0.9,
        "most of the ball's quads come back: {whole} of {}",
        ball.faces.len()
    );
}
