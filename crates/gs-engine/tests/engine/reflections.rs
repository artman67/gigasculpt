//! Reflections (decision 0041, sections 2 and 9): a scale with a factor below zero turns points
//! inside out, so every command that takes one refuses it and names `mesh.flip`, which runs,
//! while a factor of zero still flattens. `mesh.flip` itself: `flip.rs`.

use gs_testkit::{Session, code};
use serde_json::json;

use crate::support::human;

fn volume(s: &mut Session) -> f64 {
    let stats = s.query("mesh.stats", json!({ "part": "sphere" }));
    stats["parts"][0]["volume"].as_f64().unwrap_or(f64::NAN)
}

#[test]
fn a_scale_below_zero_is_refused_on_every_path_and_names_mesh_flip() {
    let mut s = human();
    s.ok("primitive.sphere", json!({}));
    let (before, inside) = (s.hash(), volume(&mut s));
    assert!(inside > 4.0, "a unit sphere holds about 4.19: {inside}");
    // One factor below zero, three, and two unequal ones: under a falloff the last turns the
    // band of points whose weight puts one factor below zero and not the other inside out.
    for scale in [
        [-1.0, 1.0, 1.0],
        [1.0, 1.0, -0.5],
        [-1.0; 3],
        [-1.0, -2.0, 1.0],
    ] {
        let op = json!({ "scale": scale });
        for (command, args) in [
            ("transform.apply", json!({ "op": op })),
            (
                "gizmo.drag",
                json!({ "phase": "update", "transform": { "op": op } }),
            ),
            ("part.duplicate", json!({ "transform": { "op": op } })),
        ] {
            let err = s.err(command, args);
            let said = format!("{} {}", err.message, err.hint.unwrap_or_default());
            assert_eq!(
                code(err.code),
                "E_ARGS_INVALID",
                "{command} {scale:?}: {said}"
            );
            assert!(
                err.path.is_some_and(|p| p.ends_with("/op/scale")),
                "{command} {scale:?} points at the scale"
            );
            assert!(
                said.contains("mesh.flip") && !said.contains("planned"),
                "{command} names mesh.flip, which exists: {said}"
            );
        }
    }
    assert_eq!(s.hash(), before, "nothing turned inside out");
    // The command the refusal names runs, and turns nothing inside out.
    s.ok("mesh.flip", json!({ "axis": "x" }));
    let flipped = volume(&mut s);
    assert!(
        (flipped - inside).abs() < 1e-9 * inside,
        "flipped, the sphere still holds {inside}: {flipped}"
    );
    s.ok(
        "transform.apply",
        json!({ "op": { "scale": [0.0, 1.0, 1.0] } }),
    );
    assert!(
        volume(&mut s).abs() < 1e-9,
        "a factor of zero still flattens"
    );
}

/// Mirrored back about the origin a copy gives back every bit, as a sign change is exact. About
/// the part's middle, `2 middle - x` is rounded to f32 each time, so a point comes back within
/// one f32 step at the part's far side on that axis; the other axes and the faces come back
/// exactly.
#[test]
fn a_mirror_copy_mirrored_back_is_exact_about_the_origin_and_within_a_step_about_the_middle() {
    let mut s = human();
    s.ok(
        "primitive.sphere",
        json!({ "subdivisions": 4, "center": [0.3, 0.2, 0.1] }),
    );
    let start = s.mesh("sphere");
    let far = (start.positions.iter()).fold(0.0_f32, |m, p| m.max(p[0].abs()));
    for center in ["origin", "part"] {
        let names = [1, 2].map(|k| format!("{center}-{k}"));
        s.ok(
            "part.mirror_copy",
            json!({ "part": "sphere", "center": center, "name": names[0] }),
        );
        s.ok(
            "part.mirror_copy",
            json!({ "part": names[0], "center": center, "name": names[1] }),
        );
        let twice = s.mesh(&names[1]);
        assert_eq!(twice.faces, start.faces, "{center}: the faces turn back");
        let off = (start.positions.iter().zip(&twice.positions))
            .inspect(|(a, b)| assert_eq!(a[1..], b[1..], "{center}: y and z exact"))
            .map(|(a, b)| (a[0] - b[0]).abs())
            .fold(0.0_f32, f32::max);
        let step = if center == "origin" {
            0.0
        } else {
            far * f32::EPSILON
        };
        assert!(off <= step, "{center}: x back within {step}, off by {off}");
    }
}
