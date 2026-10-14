//! Infinite Depth's Move clause through `brush.stroke` and a camera (catalog
//! `brush-settings.depth`, decision 0053, section 4): with Z on, a Move drag on a cube's front
//! also moves its back face inside the brush circle, the same at every camera angle, and moves
//! nothing outside it.

use std::collections::BTreeMap;

use gs_engine::EngineConfig;
use gs_protocol::{PlainMesh, Role};
use gs_testkit::{Session, fixtures};
use serde_json::{Value, json};

/// The brush radius, in scene units.
const R: f32 = 0.3;

/// A camera 4 units from the middle of the cube's front (z = 1), turned by `azimuth` and
/// `elevation` degrees: the middle pixel shows that point at every angle.
fn camera(azimuth: f32, elevation: f32) -> Value {
    json!({
        "azimuth_deg": azimuth,
        "elevation_deg": elevation,
        "target": [0.0, 0.0, 1.0],
        "distance": 4.0,
    })
}

/// The cube before and after a Move drag from the middle pixel, 60 pixels right and 20 down,
/// through `camera`, with Infinite Depth and its Z switch on.
fn drag(camera: &Value) -> (PlainMesh, PlainMesh) {
    let mut s = Session::with(EngineConfig::default(), Role::Agent);
    s.add_part("cube", &fixtures::cube(32));
    let before = s.mesh("cube");
    let samples: Vec<Value> = (0..=20)
        .map(|i| json!({ "position": [512.0 + 3.0 * i as f32, 512.0 + i as f32] }))
        .collect();
    let settings = json!({ "depth": { "infinite": true, "axes": [false, false, true] } });
    let args = json!({
        "brush": "Move",
        "camera": camera,
        "size": R,
        "samples": samples,
        "settings": settings,
    });
    let done = s.ok("brush.stroke", args);
    assert_eq!(done.result["samples_missed"], json!(0), "{}", done.result);
    (before, s.mesh("cube"))
}

/// How far vertex `i` moved, as a vector.
fn step(before: &PlainMesh, after: &PlainMesh, i: usize) -> [f32; 3] {
    std::array::from_fn(|k| after.positions[i][k] - before.positions[i][k])
}

/// The Check: from three camera angles, every vertex that moves lies inside the circle of
/// radius R around the z axis, the back face's points near the axis move, and each moves as
/// the front point in front of it does.
#[test]
fn with_z_on_a_move_on_a_cube_front_moves_its_back_inside_the_circle_at_any_angle() {
    for (azimuth, elevation) in [(0.0, 0.0), (35.0, 20.0), (-30.0, -25.0)] {
        let (before, after) = drag(&camera(azimuth, elevation));
        let angle = format!("azimuth {azimuth}, elevation {elevation}");
        let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits()];
        let front: BTreeMap<[u32; 2], usize> = (before.positions.iter().enumerate())
            .filter(|(_, p)| p[2] > 0.99)
            .map(|(i, &p)| (key(p), i))
            .collect();
        let mut back = 0;
        for (i, &p) in before.positions.iter().enumerate() {
            let d = step(&before, &after, i);
            let size = d.iter().map(|x| x * x).sum::<f32>().sqrt();
            let off_axis = libm::hypotf(p[0], p[1]);
            if size > 0.0 {
                assert!(
                    off_axis < 1.001 * R,
                    "{angle}: point {i} at {p:?} moved by {d:?}, outside the circle"
                );
            }
            if p[2] < -0.99 && off_axis < 0.5 * R {
                let twin = front[&key(p)];
                let f = step(&before, &after, twin);
                assert!(size > 1e-3, "{angle}: back point {i} at {p:?} moved {size}");
                let gap = (0..3).map(|k| (d[k] - f[k]).abs()).fold(0.0, f32::max);
                assert!(
                    gap <= 1e-5,
                    "{angle}: back point {i} moved {d:?}, front {f:?}"
                );
                back += 1;
            }
        }
        assert!(back > 10, "{angle}: {back} back points near the axis");
    }
}
