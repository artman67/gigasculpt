//! Screen-space strokes and picking: a stroke's screen sample lands where `pick.ray` says the
//! same pixel hits, a stroke whose rays all miss says how far they passed (as `pick.ray` does
//! for the same pixel), and the picks agree with brute force over the part's vertices.

use gs_testkit::code;
use serde_json::{Value, json};

use crate::sphere_session;

/// What `pick.ray` says the front view's pixel (`x`, `y`) of a 1024 by 1024 image hits.
fn pick(s: &mut gs_testkit::Session, x: f32, y: f32) -> Value {
    s.query("pick.ray", json!({ "camera": "front", "at": [x, y] }))
}

#[test]
fn a_screen_sample_lands_where_pick_ray_says() {
    let forward = {
        let mut s = sphere_session(None);
        pick(&mut s, 512.0, 512.0)["direction"].clone()
    };
    for (brush, [x, y]) in [
        ("Draw", [512.0, 512.0]),
        ("Clay", [430.5, 600.25]),
        ("Mask Pen", [600.0, 400.0]),
        ("Inflate", [300.0, 520.0]),
    ] {
        let mut screen = sphere_session(None);
        let at = pick(&mut screen, x, y)["hit"]["position"].clone();
        assert!(at.is_array(), "the pixel ({x}, {y}) shows the sphere");
        let args = json!({
            "brush": brush, "camera": "front", "size": 0.15,
            "samples": [{ "position": [x, y] }],
        });
        screen.ok("brush.stroke", args);
        let mut world = sphere_session(None);
        let args = json!({
            "brush": brush, "size": 0.15, "view_dir": forward,
            "samples": [{ "position": at }],
        });
        world.ok("brush.stroke", args);
        assert_eq!(screen.hash(), world.hash(), "{brush} at ({x}, {y})");
    }
}

#[test]
fn a_stroke_that_never_hits_says_how_far_its_rays_passed() {
    let mut s = sphere_session(None);
    let before = s.hash();
    let corners = [[20.0, 20.0], [40.0, 30.0], [60.0, 25.0]];
    let passed: Vec<f64> = (corners.iter())
        .map(|&[x, y]| pick(&mut s, x, y)["missed_by"].as_f64().unwrap_or(f64::NAN))
        .collect();
    let nearest = passed.iter().copied().fold(f64::INFINITY, f64::min);
    let samples: Vec<Value> = corners.iter().map(|p| json!({ "position": p })).collect();
    let done = s.ok(
        "brush.stroke",
        json!({ "brush": "Draw", "camera": "front", "samples": samples }),
    );
    assert_eq!(s.hash(), before, "nothing changed");
    assert_eq!(done.result["samples_missed"], json!(3), "{}", done.result);
    let reason = done.effect.and_then(|e| e.noop_reason).unwrap_or_default();
    let said = reason
        .split("passed ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .and_then(|n| n.parse::<f64>().ok())
        .unwrap_or_else(|| panic!("no distance in: {reason}"));
    assert!(reason.contains("never hit sphere"), "{reason}");
    assert!(
        (said - nearest).abs() < 1e-6,
        "{said} against pick.ray's {nearest}"
    );
}

#[test]
fn picks_agree_with_the_vertices() {
    let mut s = sphere_session(None);
    let mesh = s.mesh("sphere");
    let down = json!({ "origin": [0.05, 5.0, -0.02], "direction": [0.0, -2.0, 0.0] });
    let hit = s.query("pick.ray", down)["hit"].clone();
    let y = hit["position"][1].as_f64().unwrap_or_default();
    assert!(y > 0.99 && y <= 1.0, "it hits the top: {hit}");
    let distance = hit["distance"].as_f64().unwrap_or_default();
    assert!(
        (distance - (5.0 - y)).abs() < 1e-5,
        "distance along the ray: {hit}"
    );
    assert!(
        hit["normal"][1].as_f64().unwrap_or_default() > 0.99,
        "facing up: {hit}"
    );
    let corner = hit["vertex"].as_u64().unwrap_or(u64::MAX);
    let nearest = s.query("pick.nearest", json!({ "position": hit["position"] }));
    assert_eq!(
        nearest["found"]["vertex"],
        json!(corner),
        "the face corner nearest the hit"
    );
    let point = [0.3f32, 1.4, -0.2];
    let near = s.query("pick.nearest", json!({ "position": point }))["found"].clone();
    let gap = |q: [f32; 3]| {
        (0..3)
            .map(|k| f64::from(q[k] - point[k]) * f64::from(q[k] - point[k]))
            .sum::<f64>()
            .sqrt()
    };
    let best = mesh
        .positions
        .iter()
        .map(|&q| gap(q))
        .fold(f64::INFINITY, f64::min);
    let v = near["vertex"].as_u64().unwrap_or(u64::MAX) as usize;
    assert!(
        (gap(mesh.positions[v]) - best).abs() < 1e-6,
        "vertex {v} is the nearest"
    );
    let said = near["distance"].as_f64().unwrap_or_default();
    assert!((said - best).abs() < 1e-5, "{said} against {best}");
    // On a sphere about the origin a vertex's normal points the way the vertex lies.
    let p = mesh.positions[v].map(f64::from);
    let length = p.iter().map(|x| x * x).sum::<f64>().sqrt();
    let off: f64 = (0..3)
        .map(|k| near["normal"][k].as_f64().unwrap_or_default() - p[k] / length)
        .map(|d| d * d)
        .sum();
    assert!(off < 1e-3, "the normal points outward: {near}");
    let err = s.err("pick.ray", json!({ "camera": "front" }));
    assert_eq!(code(err.code), "E_ARGS_INVALID", "{}", err.message);
}

#[test]
fn a_size_in_pixels_is_a_radius_at_the_depth_it_lands() {
    let mut s = sphere_session(None);
    let hit = pick(&mut s, 512.0, 512.0)["hit"].clone();
    let depth = hit["distance"].as_f64().unwrap_or_default();
    let args = json!({
        "brush": "Draw", "camera": "front", "size_px": 40,
        "samples": [{ "position": [512.0, 512.0] }],
    });
    let done = s.ok("brush.stroke", args);
    let radius = done.result["radius"].as_f64().unwrap_or_default();
    // 1024 pixels span the 45 degree field of view at that depth.
    let want = 40.0 * 2.0 * depth * (std::f64::consts::SQRT_2 - 1.0) / 1024.0;
    assert!(
        (radius - want).abs() < 1e-5 * want,
        "{radius} against {want}"
    );
}

#[test]
fn drags_hooks_and_gestures_reach_past_the_surface() {
    // From the middle of the front view to a corner, where nothing is: a drag and a hook
    // follow the pen on the plane through where they grabbed, and a gesture is drawn on a
    // plane facing the camera, so the samples off the part still count.
    for (brush, path) in [
        ("Move", [[512.0, 512.0], [40.0, 40.0]]),
        ("Pull Out", [[512.0, 512.0], [40.0, 40.0]]),
        ("Clip Curve", [[40.0, 300.0], [1000.0, 260.0]]),
    ] {
        let mut s = sphere_session(None);
        let before = s.mesh("sphere");
        let samples: Vec<Value> = path.iter().map(|p| json!({ "position": p })).collect();
        let args = json!({ "brush": brush, "camera": "front", "samples": samples });
        let done = s.ok("brush.stroke", args);
        assert_eq!(
            done.result["samples_missed"],
            json!(0),
            "{brush}: {}",
            done.result
        );
        let after = s.mesh("sphere");
        assert_ne!(
            after.positions, before.positions,
            "{brush} changed the part"
        );
        if brush == "Move" {
            // The front view looks along -z: sliding on the plane through the grab moves
            // nothing toward or away from the camera.
            let deepest = (before.positions.iter().zip(&after.positions))
                .map(|(a, b)| (a[2] - b[2]).abs())
                .fold(0.0, f32::max);
            assert!(deepest < 1e-4, "a vertex moved {deepest} along the view");
        }
    }
}

#[test]
fn the_edges_of_what_a_stroke_takes() {
    let mut s = sphere_session(None);
    // Exactly as far as the brush reaches from the vertex at the front pole: it reached the
    // part, with nothing left to do there.
    let edge = s.ok(
        "brush.stroke",
        json!({ "brush": "Draw", "size": 0.5, "samples": [{ "position": [0.0, 0.0, 1.5] }] }),
    );
    let reason = edge.effect.and_then(|e| e.noop_reason).unwrap_or_default();
    assert!(
        reason.contains("reached sphere but changed nothing"),
        "{reason}"
    );
    // The largest coordinate taken, and tilt that is not a number.
    let far = json!({ "brush": "Draw", "samples": [{ "position": [1e18, 0.0, 0.0] }] });
    s.ok("brush.stroke", far);
    let tilt =
        json!({ "brush": "Draw", "samples": [{ "position": [0, 0, 1], "tilt": [1e39, 0] }] });
    let err = s.err("brush.stroke", tilt);
    assert_eq!(
        err.path.as_deref(),
        Some("/samples/0/tilt"),
        "{}",
        err.message
    );
    let infinite = json!({ "origin": [0, 0, 5], "direction": [-1e39, 0, 0] });
    let err = s.err("pick.ray", infinite);
    assert_eq!(code(err.code), "E_ARGS_INVALID", "{}", err.message);
}
