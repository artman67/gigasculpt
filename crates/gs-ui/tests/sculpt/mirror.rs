//! The cursor's mirror rings (research 23, G18): with symmetry on, a quieter ring shows at
//! each place a stroke's symmetry repeats the dab under the pen.

use serde_json::{Value, json};

use crate::rig::{Window, pen_at, with_sphere};

/// The mirror rings' centres, in window pixels.
fn mirror_rings(w: &mut Window) -> Vec<[f64; 2]> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport").cloned();
    let widgets = view.and_then(|v| v["widgets"].as_array().cloned());
    (widgets.unwrap_or_default().iter())
        .filter(|x| x["label"] == "Mirror ring")
        .map(|x| {
            let r = |i: usize| x["rect"][i].as_f64().unwrap_or_default();
            [f64::midpoint(r(0), r(2)), f64::midpoint(r(1), r(3))]
        })
        .collect()
}

/// Where the window draws the scene point `p`, in window pixels, its 3D view at `rect`.
fn drawn_at(w: &Window, rect: [f32; 4], p: [f64; 3]) -> [f64; 2] {
    let camera = w.host.lock().shell.main.camera;
    let size = [rect[2] - rect[0], rect[3] - rect[1]];
    let seen = camera.project(p.map(|c| c as f32), size);
    let [x, y] = seen.unwrap_or_else(|| panic!("{p:?} is behind the eye"));
    [f64::from(rect[0] + x), f64::from(rect[1] + y)]
}

/// With symmetry on, a ring shows at each place a stroke repeats the dab under the pen: across
/// each mirror plane and at each radial copy, where that point is drawn. None shows with
/// symmetry off, off the model, or with the gizmo tool on, where the pen does not sculpt.
#[test]
fn a_mirror_ring_shows_where_symmetry_repeats_the_dab_only_with_symmetry_on() {
    let (mut w, rect) = with_sphere(None);
    // Right of and above the sphere's middle, so no repeat falls on the pen.
    let x = f32::midpoint(rect[0], rect[2]) + 40.0;
    let y = f32::midpoint(rect[1], rect[3]) - 25.0;
    let hit = &w.picked(x, y)["hit"]["position"];
    let h: [f64; 3] = std::array::from_fn(|i| hit[i].as_f64().unwrap_or(f64::NAN));
    assert!(
        h.iter().all(|c| c.is_finite()),
        "the pen is over the sphere"
    );
    let mut t = 0;
    let mut hover = |w: &mut Window, at: [f32; 2]| {
        t += 1;
        w.pen(pen_at(t, at[0], at[1], 0.0));
        mirror_rings(w)
    };
    let none: Vec<[f64; 2]> = Vec::new();
    assert_eq!(hover(&mut w, [x, y]), none, "no symmetry, no mirror ring");

    let [a, b, c] = h;
    let signs = |m: u32| std::array::from_fn(|i| if m >> i & 1 == 1 { -h[i] } else { h[i] });
    let quarter_turns_about_y = vec![[c, b, -a], [-a, b, -c], [-c, b, a]];
    let cases: [(Value, Vec<[f64; 3]>); 4] = [
        (json!({ "mirror": { "x": true } }), vec![[-a, b, c]]),
        // Parallel's copy lands at the mirrored place too (decision 0044, section 2).
        (json!({ "parallel": { "x": true } }), vec![[-a, b, c]]),
        (
            json!({ "mirror": { "x": true, "y": true, "z": true } }),
            (1..8).map(signs).collect(),
        ),
        (
            json!({ "radial": { "axis": "y", "count": 4 } }),
            quarter_turns_about_y,
        ),
    ];
    for (symmetry, repeats) in cases {
        w.ok("brush.set", json!({ "symmetry": symmetry }));
        let rings = hover(&mut w, [x, y]);
        assert_eq!(rings.len(), repeats.len(), "{symmetry}: {rings:?}");
        for p in repeats {
            let want = drawn_at(&w, rect, p);
            let near = |r: &&[f64; 2]| libm::hypot(r[0] - want[0], r[1] - want[1]) < 1.5;
            assert!(
                rings.iter().any(|r| near(&r)),
                "{symmetry}: {p:?} at {want:?}, {rings:?}"
            );
        }
    }

    // Off the model nothing shows, nor with the gizmo tool on.
    assert_eq!(
        hover(&mut w, [rect[0] + 8.0, rect[1] + 8.0]),
        none,
        "off the sphere"
    );
    w.ok("ui.gizmo", json!({ "tool": true }));
    assert_eq!(hover(&mut w, [x, y]), none, "the gizmo tool is on");
    w.ok("ui.gizmo", json!({ "tool": false }));
    assert_eq!(hover(&mut w, [x, y]).len(), 3, "radial 4, back on");
    w.ok("brush.set", json!({ "symmetry": {} }));
    assert_eq!(hover(&mut w, [x, y]), none, "symmetry off again");
}
