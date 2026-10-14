//! The mesh-diff heatmap (catalog item `core.mesh-diff-heatmap`) through gs-ui's render host:
//! `mesh.diff` with `heatmap` draws `b` alone, white where no point moved along `a`'s normals,
//! red where they moved out and blue where they moved in, and states the scale it drew at.

use gs_protocol::PlainMesh;
use gs_testkit::fixtures::icosphere;
use serde_json::{Value, json};

use crate::looks::{Shot, picture, run, snapshot};
use crate::materials::empty;

/// A unit sphere with its front cap (the points with z above 0.9) moved out by `by` of their
/// distance from the center, or in when `by` is negative. The cap is small, so seen through the
/// front view's perspective it stays well inside the sphere's outline.
fn pushed(by: f32) -> PlainMesh {
    let mut mesh = icosphere(4);
    for p in &mut mesh.positions {
        if p[2] > 0.9 {
            *p = p.map(|c| c * (1.0 + by));
        }
    }
    mesh
}

/// `mesh.diff` of `a` against `b` with a heatmap from the front, 128 pixels square: the result
/// and the picture.
fn diff(e: &mut gs_engine::Engine, a: &str, b: &str) -> (Value, Shot) {
    seen(e, a, b, "front")
}

/// `mesh.diff` of `a` against `b` with a heatmap from `view`, 128 pixels square.
fn seen(e: &mut gs_engine::Engine, a: &str, b: &str, view: &str) -> (Value, Shot) {
    let args = json!({ "a": a, "b": b, "heatmap": { "view": view, "size": 128 } });
    let out = run(e, "mesh.diff", args);
    let shot = picture(&out["heatmap"]);
    (out, shot)
}

/// The pixel at `x`, `y`.
fn at(shot: &Shot, x: u32, y: u32) -> [i32; 3] {
    let i = ((y * shot.0 + x) * 4) as usize;
    [0, 1, 2].map(|k| i32::from(shot.1[i + k]))
}

/// Red, blue or neither: a channel 40 levels above both others.
fn tint([r, g, b]: [i32; 3]) -> &'static str {
    if r > g.max(b) + 40 {
        "red"
    } else if b > r.max(g) + 40 {
        "blue"
    } else {
        "none"
    }
}

/// Every pixel's tint.
fn tints(shot: &Shot) -> impl Iterator<Item = &'static str> + '_ {
    (shot.1.chunks(4)).map(|p| tint([0, 1, 2].map(|k| i32::from(p[k]))))
}

/// A part against itself is white wherever it shows: white paint changes nothing, so it draws
/// exactly as the part unpainted does in MatCap White, and the stated scale is 0.
#[test]
fn a_part_against_itself_draws_white() {
    let mut e = empty();
    run(
        &mut e,
        "part.create",
        json!({ "name": "ball", "mesh": pushed(0.0) }),
    );
    let (out, shot) = diff(&mut e, "part:ball", "part:ball");
    assert_eq!(out["heatmap"]["scale"], json!(0.0), "{}", out["heatmap"]);
    run(&mut e, "material.select", json!({ "name": "white" }));
    let plain = json!({ "view": "front", "size": 128, "shading": "matcap" });
    assert!(shot == snapshot(&mut e, plain), "white wherever it shows");
}

/// Pushing the front cap out turns it red and pushing it in turns it blue, while the rest of
/// the sphere, which did not move, keeps no tint, its far side seen from the back included;
/// the stated scale is the largest displacement along the normals.
#[test]
fn a_patch_pushed_out_turns_red_and_pushed_in_turns_blue() {
    let mut e = empty();
    for (name, by) in [("ball", 0.0), ("out", 0.1), ("in", -0.1)] {
        run(
            &mut e,
            "part.create",
            json!({ "name": name, "mesh": pushed(by) }),
        );
    }
    for (b, color, other) in [("out", "red", "blue"), ("in", "blue", "red")] {
        let (out, shot) = diff(&mut e, "part:ball", &format!("part:{b}"));
        let along = &out["displacement"]["along_normal"];
        let largest = [&along["min"], &along["max"]]
            .map(|v| v.as_f64().unwrap_or_default().abs())
            .into_iter()
            .fold(0.0, f64::max);
        let scale = out["heatmap"]["scale"].as_f64().unwrap_or_default();
        assert!(
            (scale - largest).abs() < 1e-9 && scale > 0.09,
            "{b}: {scale} {along}"
        );
        assert_eq!(tint(at(&shot, 64, 64)), color, "{b}: the cap");
        // The middle row: the sphere's own pixels, and the outer ones, which did not move.
        let row: Vec<[i32; 3]> = (0..shot.0).map(|x| at(&shot, x, 64)).collect();
        let covered: Vec<usize> = (0..row.len()).filter(|&x| row[x] != row[0]).collect();
        let (first, last) = (covered[0], covered[covered.len() - 1]);
        let width = (last - first) as f64;
        let rim = |x: usize| ((x - first) as f64 / width - 0.5).abs() > 0.43;
        for x in covered.iter().copied().filter(|&x| rim(x)) {
            assert_eq!(tint(row[x]), "none", "{b}: x {x}, {:?}", row[x]);
        }
        assert!(tints(&shot).all(|t| t != other), "{b}: no {other}");
        let (_, back) = seen(&mut e, "part:ball", &format!("part:{b}"), "back");
        assert!(tints(&back).all(|t| t == "none"), "{b}: the far side");
    }
}
