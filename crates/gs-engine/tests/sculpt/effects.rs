//! A stroke's reported effect against the test kit's slow reference worked out from the whole
//! scene before and after: positions, vertex columns (the mask), face columns (hidden faces)
//! and columns a stroke adds. And what the stroke's options do to the surface: the Brush
//! Modifier, radial symmetry, invert, axis lock, and the view of a world stroke.

use gs_protocol::PlainMesh;
use gs_testkit::measure::stats;
use gs_testkit::reference::effect;
use serde_json::{Value, json};

use crate::{line, sphere_session, stroke, with};

/// A stroke of screen samples through the front view.
fn on_screen(brush: &str, points: &[[f32; 2]], extra: &Value) -> Value {
    let samples: Vec<Value> = points.iter().map(|p| json!({ "position": p })).collect();
    let args = json!({ "brush": brush, "camera": "front", "samples": samples });
    with(args, extra)
}

#[test]
fn reported_effects_match_the_reference() {
    let mut s = sphere_session(None);
    let across = line([-0.3, 0.1, 0.95], [0.3, 0.0, 0.95], 6);
    // Select Rect's press and release, corner to corner.
    let rect = [[300.0, 300.0], [700.0, 700.0]];
    let calls = [
        stroke("Mask Pen", &across, &json!({})),
        stroke(
            "Mask Pen",
            &line([0.0, -0.4, 0.9], [0.0, 0.4, 0.9], 5),
            &json!({}),
        ),
        stroke("Mask Pen", &across, &json!({ "invert": true })),
        on_screen("Select Rect", &rect, &json!({})),
        on_screen(
            "Select Rect",
            &[[700.0, 300.0], [450.0, 600.0]],
            &json!({ "invert": true }),
        ),
        stroke("Draw", &across, &json!({ "size": 0.2 })),
    ];
    for args in calls {
        let before = s.scene();
        let done = s.ok("brush.stroke", args.clone());
        let after = s.scene();
        let want = effect(&before, &after);
        let got = done
            .effect
            .unwrap_or_else(|| panic!("no effect for {args}"));
        let bits = |b: Option<[[f32; 3]; 2]>| b.map(|c| c.map(|p| p.map(f32::to_bits)));
        assert!(
            want.verts_touched > 0,
            "the stroke changed something: {args}"
        );
        assert_eq!(got.verts_touched, want.verts_touched, "{args}");
        assert_eq!(
            bits(got.bbox.map(|[a, b]| [a.0, b.0])),
            bits(want.bbox),
            "{args}"
        );
        assert_eq!(got.parts, want.parts, "{args}");
        assert!(!got.topology_changed, "{args}");
    }
}

/// A stroke's effect names what it changed: a mask stroke the mask and not the positions (both
/// when the mask is new and when it already was there), a sculpting stroke the positions and
/// not the mask the part now holds, a selection the hidden faces.
#[test]
fn a_stroke_names_what_it_changed() {
    let mut s = sphere_session(None);
    let across = line([-0.3, 0.1, 0.95], [0.3, 0.0, 0.95], 6);
    let below = line([-0.3, -0.3, 0.9], [0.3, -0.3, 0.9], 6);
    let above = line([-0.3, 0.4, 0.87], [0.3, 0.4, 0.87], 6);
    let rect = [[300.0, 300.0], [700.0, 700.0]];
    let calls = [
        (stroke("Mask Pen", &across, &json!({})), "mask"),
        (stroke("Mask Pen", &below, &json!({})), "mask"),
        (stroke("Draw", &above, &json!({ "size": 0.2 })), "positions"),
        (on_screen("Select Rect", &rect, &json!({})), "hide"),
    ];
    for (args, name) in calls {
        let done = s.ok("brush.stroke", args.clone());
        let changed = done.effect.map(|e| e.changed).unwrap_or_default();
        let want = [("sphere".to_string(), vec![name.to_string()])];
        assert_eq!(changed, want.into(), "{args}");
    }
}

/// How far each vertex of the sphere moves, from `before` to `after`.
fn moves(before: &PlainMesh, after: &PlainMesh) -> Vec<[f64; 3]> {
    (before.positions.iter().zip(&after.positions))
        .map(|(a, b)| std::array::from_fn(|k| f64::from(b[k]) - f64::from(a[k])))
        .collect()
}

/// The moves of one dab of `brush` on the front of the sphere, with `extra` arguments.
fn one_dab(brush: &str, extra: &Value) -> Vec<[f64; 3]> {
    let mut s = sphere_session(None);
    let before = s.mesh("sphere");
    let args = with(json!({ "size": 0.3 }), extra);
    s.ok("brush.stroke", stroke(brush, &[[0.1, 0.05, 0.99]], &args));
    moves(&before, &s.mesh("sphere"))
}

/// `a - k b`, vertex by vertex.
fn less(a: &[[f64; 3]], k: f64, b: &[[f64; 3]]) -> Vec<[f64; 3]> {
    (a.iter().zip(b))
        .map(|(a, b)| std::array::from_fn(|i| a[i] - k * b[i]))
        .collect()
}

/// The length of all the moves together.
fn length(v: &[[f64; 3]]) -> f64 {
    v.iter().flatten().map(|x| x * x).sum::<f64>().sqrt()
}

#[test]
fn the_brush_modifier_adds_a_pinch_or_an_inflate() {
    // At m the modifier adds what Pinch (m above 0) or Inflate does at m% of Draw's strength.
    const DRAW_STRENGTH: f64 = 0.4;
    let plain = one_dab("Draw", &json!({}));
    for m in [100.0, 50.0, -100.0, -50.0] {
        let added = less(&one_dab("Draw", &json!({ "modifier": m })), 1.0, &plain);
        let brush = if m > 0.0 { "Pinch" } else { "Inflate" };
        let strength = DRAW_STRENGTH * f64::abs(m) / 100.0;
        let alone = one_dab(brush, &json!({ "strength": strength }));
        assert!(length(&alone) > 1e-3, "{brush} moves the surface");
        assert!(
            length(&less(&added, 1.0, &alone)) < 1e-4 * length(&alone),
            "the modifier at {m} adds {brush} at strength {strength}"
        );
    }
}

#[test]
fn radial_symmetry_repeats_a_dab_around_its_axis() {
    let mut s = sphere_session(None);
    let before = s.mesh("sphere");
    let args = json!({ "size": 0.2, "symmetry": { "radial": { "axis": "z", "count": 4 } } });
    s.ok("brush.stroke", stroke("Draw", &[[0.45, 0.45, 0.77]], &args));
    let after = s.mesh("sphere");
    let mut quarters = [0; 4];
    for (a, m) in before.positions.iter().zip(moves(&before, &after)) {
        if m.iter().any(|&x| x != 0.0) {
            quarters[usize::from(a[0] < 0.0) + 2 * usize::from(a[1] < 0.0)] += 1;
        }
    }
    assert!(
        quarters.iter().all(|&n| n > 0),
        "every quarter turn gets its copy: {quarters:?}"
    );
}

#[test]
fn an_inverted_draw_digs_and_a_locked_axis_holds() {
    let path = line([-0.3, 0.1, 0.95], [0.3, 0.0, 0.95], 6);
    let mut s = sphere_session(None);
    let before = s.mesh("sphere");
    let volume = |m| stats(m).volume;
    s.ok(
        "brush.stroke",
        stroke("Draw", &path, &json!({ "invert": true })),
    );
    assert!(
        volume(&s.mesh("sphere")) < volume(&before) - 1e-4,
        "an inverted Draw digs in"
    );
    s.ok("history.undo", json!({}));
    let lock = json!({ "lock": { "x": true } });
    s.ok("brush.stroke", stroke("Draw", &path, &lock));
    let m = moves(&before, &s.mesh("sphere"));
    assert!(
        m.iter().any(|d| d[2] != 0.0),
        "the locked stroke still moves along z"
    );
    assert!(m.iter().all(|d| d[0] == 0.0), "nothing moves along x");
}

#[test]
fn a_world_stroke_without_a_view_looks_into_the_surface() {
    // A Clip Curve across the front: which side it clips depends on which way the camera
    // looks, so the stroke with no view clips the side the one seen from the front does.
    let path = line([-1.2, 0.5, 1.0], [1.2, 0.4, 1.0], 4);
    let clipped = |extra: &Value| {
        let mut s = sphere_session(None);
        let before = s.mesh("sphere");
        s.ok("brush.stroke", stroke("Clip Curve", &path, extra));
        let m = moves(&before, &s.mesh("sphere"));
        let ys: Vec<f64> = (before.positions.iter().zip(&m))
            .filter(|(_, d)| d.iter().any(|&x| x != 0.0))
            .map(|(p, _)| f64::from(p[1]))
            .collect();
        assert!(!ys.is_empty(), "the clip moved something: {extra}");
        ys.iter().sum::<f64>() / ys.len() as f64
    };
    let (unseen, front) = (
        clipped(&json!({})),
        clipped(&json!({ "view_dir": [0, 0, -1] })),
    );
    assert!(
        (unseen - 0.45) * (front - 0.45) > 0.0,
        "both clip the same side of the line: {unseen} and {front}"
    );
}

/// How many faces of `mesh` have zero area (corners that coincide or line up).
fn flat_faces(mesh: &PlainMesh) -> usize {
    let p = |v: u32| mesh.positions[v as usize].map(f64::from);
    let twice_area = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| {
        let (u, w) = (
            [0, 1, 2].map(|k| b[k] - a[k]),
            [0, 1, 2].map(|k| c[k] - a[k]),
        );
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        n[0] * n[0] + n[1] * n[1] + n[2] * n[2]
    };
    (mesh.faces.iter())
        .filter(|f| {
            let corners = if f[3] == u32::MAX { &f[..3] } else { &f[..] };
            let a = p(corners[0]);
            (corners.windows(2).skip(1)).all(|w| twice_area(a, p(w[0]), p(w[1])) == 0.0)
        })
        .count()
}

/// Clip Curve presses everything on one side of its line into a plane, so on a cube the faces
/// that crossed that plane end with zero area. They are legal (review 0002, M2): the stroke
/// lands, the part stays valid, and undo and redo restore each state exactly.
#[test]
fn clip_curve_on_a_cube_leaves_flat_faces_and_undoes_exactly() {
    let mut s = sphere_session(None);
    s.ok(
        "primitive.cube",
        json!({ "name": "cube", "subdivisions": 8 }),
    );
    let before = s.hash();
    let path = line([-2.0, 0.3, 2.0], [2.0, 0.3, 2.0], 4);
    // Seen along -z, the cut is the plane y = 0.3 exactly, so the pressed faces are exactly
    // flat whatever the rounding.
    let extra = json!({ "part": "cube", "view_dir": [0.0, 0.0, -1.0] });
    s.ok("brush.stroke", stroke("Clip Curve", &path, &extra));
    let after = s.hash();
    assert_ne!(after, before, "the clip changed the cube");
    assert!(
        flat_faces(&s.mesh("cube")) > 0,
        "and left faces of zero area"
    );
    let checked = s.query("mesh.validate", json!({ "part": "cube" }));
    assert_eq!(checked["valid"], json!(true), "{checked}");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "undo restores the cube exactly");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "redo restores the clip exactly");
}
