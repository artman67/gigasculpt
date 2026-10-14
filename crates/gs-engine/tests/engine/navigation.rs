//! A view's move settings through commands (decision 0017 as amended): the snap step and the
//! Recenter Rate (canvas.navigation), Auto Adjust Distance and Align To Object
//! (canvas.perspective), and a right press that moves the pivot (canvas.local-pivot); all of
//! them logged, so a replay gives every camera back.

use gs_core::Vec3;
use gs_engine::EngineConfig;
use gs_protocol::Role;
use gs_render::Camera;
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::support::{EngineSession, agent, camera, exact, human};
use crate::views::log_path;

/// The size an agent's view is seen at.
const SIZE: [f32; 2] = [1024.0, 1024.0];
const VIEW: &str = "agent:test";

/// Where `p` is drawn.
fn pixel(c: &Camera, p: [f32; 3]) -> [f32; 2] {
    c.project(p, SIZE)
        .unwrap_or_else(|| panic!("{p:?} is behind the eye"))
}

/// How far apart two pixels are.
fn apart(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    (dx * dx + dy * dy).sqrt()
}

/// How far from the middle of the picture `p` is drawn.
fn from_middle(c: &Camera, p: [f32; 3]) -> f32 {
    apart(pixel(c, p), [SIZE[0] * 0.5, SIZE[1] * 0.5])
}

/// Puts the view back on the camera `exact` gave.
fn reset(s: &mut Session, exact: &str) {
    let camera: Value = serde_json::from_str(exact).unwrap_or_else(|e| panic!("{e}"));
    s.ok("view.set", json!({ "camera": camera }));
}

/// The view's last-edit point.
fn last_edit(s: &mut Session) -> Option<[f32; 3]> {
    serde_json::from_value(s.query("view.get", json!({}))["last_edit"].clone()).ok()
}

/// canvas.navigation: the default snap lands on an axis view exactly; a 15 degree step lands on
/// whole steps of turn and height with a level horizon; a step that does not divide 90 is
/// refused.
#[test]
fn a_snap_lands_on_whole_steps_with_a_level_horizon() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    let odd = json!({ "right_deg": 37, "up_deg": 21, "mode": "free", "snap": true });
    s.ok("view.orbit", odd);
    let (right, up, forward) = camera(&mut s, VIEW).axes();
    for v in [right, up, forward] {
        let sorted = {
            let mut a = v.to_array().map(f32::abs);
            a.sort_by(f32::total_cmp);
            a
        };
        let axis = sorted[0] < 1e-6 && sorted[1] < 1e-6 && (sorted[2] - 1.0).abs() < 1e-6;
        assert!(axis, "an axis view within 1e-6: {v:?}");
    }
    s.ok("view.settings", json!({ "snap_deg": 15 }));
    // Where the camera looks from at every whole step of turn and of height.
    let step = 15f32.to_radians();
    let stepped: Vec<Vec3> = (0..24)
        .flat_map(|a| (-6..=6).map(move |e| (a as f32 * step, e as f32 * step)))
        .map(|(a, e)| Camera::turned(a, e).axes().2)
        .collect();
    // The cosine of 10.7 degrees: the nearest step is never more than half a step's diagonal
    // away.
    let near = 0.982_612_8;
    let turns = [(37.0, 21.0), (-52.0, 33.0), (118.0, -71.0), (10.0, 80.0)];
    for (r, u) in turns {
        let free = json!({ "right_deg": r, "up_deg": u, "mode": "free" });
        s.ok("view.orbit", free);
        let was = camera(&mut s, VIEW).axes().2;
        s.ok("view.orbit", json!({ "snap": true }));
        let (right, _, forward) = camera(&mut s, VIEW).axes();
        let moved = was.dot(forward);
        assert!(moved > near, "the snap went too far: cosine {moved}");
        let back = -forward;
        let gap = stepped
            .iter()
            .map(|&v| (v - back).len2())
            .fold(f32::INFINITY, f32::min);
        assert!(gap < 1e-10, "{back:?} is a whole step of turn and height");
        assert!(right.y.abs() < 1e-6, "a level horizon: {right:?}");
    }
    let e = s.err("view.settings", json!({ "snap_deg": 40 }));
    assert_eq!(e.path.as_deref(), Some("/snap_deg"), "{}", e.message);
}

/// canvas.navigation: with the Recenter Rate at 0 an orbit keeps the pivot on its pixel; at 0.5
/// a 90 degree turn halves its distance from the middle, the same in one call or in nine; at 1
/// any turn centres it.
#[test]
fn the_recenter_rate_slides_the_pivot_to_the_middle_as_the_view_turns() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("view.frame", json!({}));
    let start = exact(&mut s, VIEW);
    let pivot = [0.6, 0.3, 0.7];
    let off = from_middle(&camera(&mut s, VIEW), pivot);
    assert!(off > 50.0, "the pivot is off the middle: {off}");
    let turn = |deg: f32| json!({ "right_deg": deg, "pivot": pivot });
    s.ok("view.orbit", turn(90.0));
    let kept = from_middle(&camera(&mut s, VIEW), pivot);
    assert!(
        (kept - off).abs() < 0.05,
        "rate 0 keeps it: {kept} against {off}"
    );
    reset(&mut s, &start);
    s.ok("view.settings", json!({ "recenter_rate": 0.5 }));
    s.ok("view.orbit", turn(90.0));
    let once = from_middle(&camera(&mut s, VIEW), pivot);
    assert!((once - off * 0.5).abs() < 0.05, "halved: {once} of {off}");
    reset(&mut s, &start);
    for _ in 0..9 {
        s.ok("view.orbit", turn(10.0));
    }
    let nine = from_middle(&camera(&mut s, VIEW), pivot);
    assert!(
        (nine - once).abs() < 0.05,
        "nine small turns: {nine} against {once}"
    );
    s.ok("view.settings", json!({ "recenter_rate": 1 }));
    s.ok("view.orbit", turn(1.0));
    let centred = from_middle(&camera(&mut s, VIEW), pivot);
    assert!(centred < 0.01, "rate 1 centres it: {centred}");
    let e = s.err("view.settings", json!({ "recenter_rate": 1.5 }));
    assert_eq!(e.path.as_deref(), Some("/recenter_rate"), "{}", e.message);
}

/// canvas.perspective: without Auto Adjust Distance three halving zooms keep the camera's
/// distance and take the eye through the model, a zoom toward a pixel keeps the surface under
/// it on that pixel, and orthographic zooms scale as with it. canvas.local-pivot: about the
/// last edit it is on anyway, and the edit point keeps its pixel.
#[test]
fn without_auto_adjust_distance_a_zoom_keeps_the_perspective_and_may_pass_through_the_model() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("view.frame", json!({}));
    let start = exact(&mut s, VIEW);
    let before = camera(&mut s, VIEW);
    s.ok("view.settings", json!({ "auto_distance": false }));
    for _ in 0..3 {
        s.ok("view.dolly", json!({ "factor": 0.5 }));
    }
    let after = camera(&mut s, VIEW);
    assert_eq!(after.distance, before.distance, "the perspective stays");
    assert_eq!(after.orientation, before.orientation, "and the turn");
    assert!(
        after.depth(Vec3::ZERO) < 0.0,
        "the eye passed the sphere's centre"
    );
    // Toward a pixel: the surface under it stays there.
    reset(&mut s, &start);
    let at = [640.0, 430.0];
    s.ok("view.touch", json!({ "at": at }));
    let surface = last_edit(&mut s).unwrap_or_else(|| panic!("the press is on the sphere"));
    s.ok("view.dolly", json!({ "factor": 0.6, "toward": at }));
    let moved = camera(&mut s, VIEW);
    assert_eq!(moved.distance, before.distance, "the distance stays");
    let off = apart(pixel(&moved, surface), at);
    assert!(off < 0.05, "the surface point moved {off} px");
    // About the last edit, Auto Adjust Distance is on anyway.
    s.ok("view.settings", json!({ "pivot_rule": "last-edit" }));
    let near = camera(&mut s, VIEW);
    s.ok("view.dolly", json!({ "factor": 0.5 }));
    let local = camera(&mut s, VIEW);
    assert!(
        (local.distance - near.distance * 0.5).abs() < 1e-5,
        "scaled: {local:?}"
    );
    let off = apart(pixel(&local, surface), at);
    assert!(off < 0.05, "the edit point moved {off} px");
    // Orthographic: the same as with it on.
    s.ok("view.settings", json!({ "pivot_rule": "target" }));
    s.ok("view.lens", json!({ "projection": "orthographic" }));
    let flat = exact(&mut s, VIEW);
    s.ok("view.dolly", json!({ "factor": 0.5 }));
    let without = exact(&mut s, VIEW);
    reset(&mut s, &flat);
    s.ok("view.settings", json!({ "auto_distance": true }));
    s.ok("view.dolly", json!({ "factor": 0.5 }));
    assert_eq!(exact(&mut s, VIEW), without, "orthographic zooms alike");
}

/// The width of `part` on the picture, in pixels.
fn width(s: &mut Session, part: &str) -> f32 {
    let c = camera(s, VIEW);
    let positions = s.mesh(part).positions;
    let xs = positions.iter().map(|&p| pixel(&c, p)[0]);
    let (lo, hi) = xs.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
        (lo.min(x), hi.max(x))
    });
    hi - lo
}

/// canvas.perspective's Check: without perspective two equal spheres at different depths are
/// drawn the same size; with it the nearer is larger, more so at a wider angle of view.
#[test]
fn equal_spheres_at_two_depths_look_equal_only_without_perspective() {
    let mut s = agent();
    let near = json!({ "name": "near", "radius": 0.5, "center": [-1, 0, 1] });
    s.ok("primitive.sphere", near);
    let far = json!({ "name": "far", "radius": 0.5, "center": [1, 0, -1] });
    s.ok("primitive.sphere", far);
    let ratio = |s: &mut Session, lens: Value| {
        s.ok("view.lens", lens);
        s.ok("view.frame", json!({}));
        width(s, "near") / width(s, "far")
    };
    let flat = ratio(&mut s, json!({ "projection": "orthographic" }));
    assert!((flat - 1.0).abs() < 1e-4, "the same size: {flat}");
    let narrow = ratio(
        &mut s,
        json!({ "projection": "perspective", "fov_deg": 35 }),
    );
    let wide = ratio(&mut s, json!({ "fov_deg": 70 }));
    assert!(narrow > 1.05, "the nearer is larger: {narrow}");
    assert!(wide > narrow, "more so wider: {wide} against {narrow}");
}

/// canvas.perspective's Check: at a 40 degree angle of view, a cube panned to the right edge
/// with Align To Object on is the centred picture moved sideways; off, it shows more of its
/// left side; in orthographic both settings give the same camera.
#[test]
fn align_to_object_pans_the_picture_without_turning_the_model() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    s.ok("view.lens", json!({ "fov_deg": 40 }));
    s.ok("view.frame", json!({}));
    let start = exact(&mut s, VIEW);
    let centred = camera(&mut s, VIEW);
    let points = s.mesh("cube").positions;
    // The left side's width on the picture: its front edge's pixel less its back edge's.
    let left = |c: &Camera| pixel(c, [-1.0, 0.0, 1.0])[0] - pixel(c, [-1.0, 0.0, -1.0])[0];
    let pan = json!({ "dx": 410 });
    s.ok("view.settings", json!({ "align_to_object": true }));
    s.ok("view.pan", pan.clone());
    let on = camera(&mut s, VIEW);
    for &p in &points {
        let [x, y] = pixel(&centred, p);
        let off = apart(pixel(&on, p), [x + 410.0, y]);
        assert!(off < 0.01, "{p:?} moved {off} px off a sideways move");
    }
    reset(&mut s, &start);
    s.ok("view.settings", json!({ "align_to_object": false }));
    s.ok("view.pan", pan.clone());
    let off = camera(&mut s, VIEW);
    assert!(
        left(&off) > left(&centred) + 1.0 && left(&off) > 0.0,
        "off shows its left side: {} against {}",
        left(&off),
        left(&centred)
    );
    s.ok("view.lens", json!({ "projection": "orthographic" }));
    s.ok("view.frame", json!({}));
    let flat = exact(&mut s, VIEW);
    s.ok("view.pan", pan.clone());
    let without = exact(&mut s, VIEW);
    reset(&mut s, &flat);
    s.ok("view.settings", json!({ "align_to_object": true }));
    s.ok("view.pan", pan);
    assert_eq!(exact(&mut s, VIEW), without, "orthographic pans alike");
}

/// canvas.local-pivot: a right press over the model moves the last-edit point to the surface
/// under it, and turning about the last edit then keeps that point on its pixel; one over empty
/// canvas, or with Right-drag moves the pivot off, changes nothing.
#[test]
fn a_right_press_on_the_model_moves_the_pivot_and_one_on_empty_canvas_does_not() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    s.ok("view.frame", json!({}));
    s.ok("view.settings", json!({ "pivot_rule": "last-edit" }));
    let at = [600.0, 450.0];
    s.ok("view.touch", json!({ "at": at }));
    let point = last_edit(&mut s).unwrap_or_else(|| panic!("the press is on the sphere"));
    let length = Vec3::from_array(point).len2().sqrt();
    assert!((length - 1.0).abs() < 0.02, "on the surface: {point:?}");
    assert!(apart(pixel(&camera(&mut s, VIEW), point), at) < 0.05);
    s.ok("view.touch", json!({ "at": [5, 5] }));
    assert_eq!(
        last_edit(&mut s),
        Some(point),
        "empty canvas changes nothing"
    );
    s.ok("view.orbit", json!({ "right_deg": 40, "up_deg": 15 }));
    let off = apart(pixel(&camera(&mut s, VIEW), point), at);
    assert!(off < 0.05, "the view turned about it: {off} px");
    s.ok("view.settings", json!({ "right_drag_pivot": false }));
    s.ok("view.touch", json!({ "at": [512, 512] }));
    assert_eq!(
        last_edit(&mut s),
        Some(point),
        "the setting off changes nothing"
    );
}

/// A right press lands on the nearest part by scene distance, each part where its transform
/// puts it (decision 0029, section 4): a cube made inside the sphere and moved out in front of
/// it takes the press, and moved behind it, the sphere does.
#[test]
fn a_right_press_lands_on_the_nearest_part_where_its_move_put_it() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    s.ok("primitive.cube", json!({ "size": 0.6 }));
    let to = |z: f32| json!({ "part": "cube", "op": { "move": [0.0, 0.0, z] } });
    s.ok("transform.apply", to(1.6));
    s.ok("view.frame", json!({}));
    s.ok("view.settings", json!({ "pivot_rule": "last-edit" }));
    let middle = json!({ "at": [512, 512] });
    s.ok("view.touch", middle.clone());
    let front = last_edit(&mut s).unwrap_or_else(|| panic!("the press is on the cube"));
    assert!((front[2] - 1.9).abs() < 1e-3, "the cube's face: {front:?}");
    s.ok("transform.apply", to(-3.2));
    s.ok("view.touch", middle);
    let back = last_edit(&mut s).unwrap_or_else(|| panic!("the press is on the sphere"));
    assert!((back[2] - 1.0).abs() < 0.02, "the sphere's front: {back:?}");
}

/// Every setting is logged with `view.settings` and every move that reads one with what it
/// read, so a replay gives the camera, the settings and the pivot back bit for bit.
#[test]
fn the_settings_are_logged_and_a_replay_gives_every_camera_back() {
    let log = log_path("navigation-settings");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({}));
    s.ok("view.frame", json!({}));
    let settings = json!({ "pivot_rule": "under-pointer", "recenter_rate": 0.3, "snap_deg": 30,
        "auto_distance": false, "align_to_object": true });
    s.ok("view.settings", settings);
    for k in 0..20 {
        let args = json!({ "right_deg": 1.1, "up_deg": 0.4, "at": [600, 450], "gesture": 3,
            "end": k == 19 });
        s.ok("view.orbit", args);
    }
    s.ok(
        "view.orbit",
        json!({ "right_deg": 20, "snap": true, "at": [600, 450] }),
    );
    s.ok("view.pan", json!({ "dx": 120, "dy": -40 }));
    s.ok("view.dolly", json!({ "factor": 0.6, "toward": [500, 500] }));
    s.ok("view.settings", json!({ "pivot_rule": "last-edit" }));
    s.ok("view.touch", json!({ "at": [520, 500] }));
    s.ok("view.orbit", json!({ "right_deg": -25 }));
    s.ok("view.settings", json!({ "right_drag_pivot": false }));
    let keys = [
        "camera",
        "pivot_rule",
        "pivot",
        "last_edit",
        "recenter_rate",
        "snap_deg",
        "auto_distance",
        "align_to_object",
        "right_drag_pivot",
    ];
    let seen = |s: &mut Session| {
        let view = s.query("view.get", json!({}));
        keys.map(|k| view[k].to_string())
    };
    let want = seen(&mut s);
    let mut replay = human();
    replay.replay_log(&log);
    std::fs::remove_dir_all(log.parent().unwrap_or(&log)).ok();
    assert_eq!(seen(&mut replay), want, "bit for bit");
}
