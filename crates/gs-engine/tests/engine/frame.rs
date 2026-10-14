//! Frame against canvas.frame's Check (decision 0017): it fits the faces that show, hidden
//! parts never count, and a second press frames the active part. Through commands, the way the
//! window's F key and an agent call it.

use gs_render::Camera;
use gs_testkit::Session;
use serde_json::json;

use crate::groups::ids;
use crate::support::{agent, camera};

const VIEWPORT: [u32; 2] = [800, 600];
const SIZE: [f32; 2] = [800.0, 600.0];

/// The positions of the vertices of `part`'s faces that show.
fn shown_points(s: &mut Session, part: &str) -> Vec<[f32; 3]> {
    let mesh = s.mesh(part);
    let hide = ids(&mesh, "hide");
    let shown = (mesh.faces.iter().zip(&hide)).filter(|(_, h)| **h == 0);
    shown
        .flat_map(|(f, _)| f.iter().filter(|&&v| v != u32::MAX).copied())
        .map(|v| mesh.positions[v as usize])
        .collect()
}

/// How far toward the frame's edge the farthest of `points` reaches, as a fraction of the
/// half-frame (0.9 is the margin framing keeps).
fn reach(c: &Camera, points: &[[f32; 3]]) -> f32 {
    (points.iter())
        .filter_map(|p| c.project(*p, SIZE))
        .map(|[x, y]| ((2.0 * x / SIZE[0] - 1.0).abs()).max((1.0 - 2.0 * y / SIZE[1]).abs()))
        .fold(0.0, f32::max)
}

/// Research 27's probe: a small Mask Pen patch on a sphere of 6,144 faces, every other face
/// hidden, framed; then every face shown, framed again.
#[test]
fn frame_fits_only_the_faces_that_show() {
    let mut s = agent();
    s.ok(
        "primitive.sphere",
        json!({ "name": "s", "subdivisions": 32 }),
    );
    let samples = [[0.0, 0.0, 1.0], [0.03, 0.02, 1.0]].map(|p| json!({ "position": p }));
    let pen = json!({ "part": "s", "brush": "Mask Pen", "size": "8%", "samples": samples });
    s.ok("brush.stroke", pen);
    let masked = json!({ "by": "masked", "threshold": 0.01, "rule": "any" });
    s.ok("visibility.only", json!({ "select": masked }));
    let patch = shown_points(&mut s, "s");
    let faces = s.mesh("s").faces.len();
    assert!(
        !patch.is_empty() && patch.len() < faces / 10,
        "a small patch shows: {} points of {faces} faces",
        patch.len()
    );
    s.ok(
        "view.frame",
        json!({ "target": "all", "viewport": VIEWPORT }),
    );
    let near = camera(&mut s, "agent:test");
    s.ok("visibility.show_all", json!({}));
    s.ok(
        "view.frame",
        json!({ "target": "all", "viewport": VIEWPORT }),
    );
    let all = camera(&mut s, "agent:test");
    assert!(
        near.distance < 0.5 * all.distance,
        "the patch alone is framed nearer: {} against {}",
        near.distance,
        all.distance
    );
    let filled = reach(&near, &patch);
    assert!(
        filled <= 0.9 + 1e-3 && filled > 0.6,
        "the patch fills the view inside the margin: {filled}"
    );
}

/// The F key's call: `view.frame` with nothing but the viewport.
fn press(s: &mut Session) -> Camera {
    s.ok("view.frame", json!({ "viewport": VIEWPORT }));
    camera(s, "agent:test")
}

/// The camera an explicit `view.frame` with `args` gives, from the same turn.
fn framed(s: &mut Session, args: serde_json::Value) -> Camera {
    let mut args = args;
    args["viewport"] = json!(VIEWPORT);
    s.ok("view.frame", args);
    camera(s, "agent:test")
}

/// canvas.frame: a second press frames the selected part (the active one, decision 0033) when
/// several show, another goes back to all; a press after the view moved frames all again.
#[test]
fn a_second_press_frames_the_active_part_and_another_goes_back_to_all() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "name": "ball" }));
    s.ok(
        "primitive.cube",
        json!({ "name": "box", "size": 0.5, "center": [3, 0, 0] }),
    );
    s.ok("view.orbit", json!({ "right_deg": 30, "up_deg": 20 }));
    let all = press(&mut s);
    let second = press(&mut s);
    let third = press(&mut s);
    assert_eq!(
        framed(&mut s, json!({ "target": "all" })),
        all,
        "a press frames all"
    );
    let part = framed(&mut s, json!({ "target": "part", "part": "box" }));
    assert_eq!(second, part, "the second press frames the active part");
    assert!(second.distance < 0.5 * all.distance, "nearer: {second:?}");
    assert_eq!(third, all, "another press goes back to all");
    s.ok(
        "view.frame",
        json!({ "target": "all", "viewport": VIEWPORT }),
    );
    s.ok("view.orbit", json!({ "right_deg": 10 }));
    let moved = press(&mut s);
    let moved_all = framed(&mut s, json!({ "target": "all" }));
    assert_eq!(moved, moved_all, "after a move a press frames all");
    s.ok(
        "part.set_visible",
        json!({ "parts": ["ball"], "visible": false }),
    );
    assert_eq!(press(&mut s), press(&mut s), "one part showing: all again");
}

/// A part 1000 units away, hidden, leaves the framing as it was before the part was made.
#[test]
fn a_hidden_far_part_leaves_the_framing_unchanged() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "name": "near" }));
    s.ok(
        "view.frame",
        json!({ "target": "all", "viewport": VIEWPORT }),
    );
    let alone = camera(&mut s, "agent:test");
    s.ok(
        "primitive.sphere",
        json!({ "name": "far", "center": [1000, 0, 0] }),
    );
    s.ok("part.select", json!({ "part": "near" }));
    s.ok(
        "part.set_visible",
        json!({ "parts": ["far"], "visible": false }),
    );
    s.ok(
        "view.frame",
        json!({ "target": "all", "viewport": VIEWPORT }),
    );
    assert_eq!(camera(&mut s, "agent:test"), alone);
}
