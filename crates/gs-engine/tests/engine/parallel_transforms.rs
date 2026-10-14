//! Parallel symmetry in `transform.apply` (decision 0044, section 5 as amended): a point across
//! a Parallel plane from the frame's origin moves by the transform about the origin's
//! reflection, unreflected, `p' = M o + L (p - M o) + t`; across a Mirror plane it moves by the
//! mirrored transform, as before; a far point is weighted as Mirror weights it; a rigid move
//! under Parallel moves the whole part; Radial is still refused. Each reference is worked out
//! here, independently of the engine.

use gs_testkit::Session;
use serde_json::{Value, json};

// `cycled(p, o)`: `p` turned 120 degrees about [1, 1, 1] through `o`, X to Y, Y to Z, Z to X.
use crate::gizmo::cycled;
use crate::support::{agent, worst};

/// The world frame at `origin`.
fn at(origin: [f32; 3]) -> Value {
    json!({ "origin": origin, "axes": [0.0, 0.0, 0.0, 1.0] })
}

/// A sphere symmetric in x, y and z, with points on each plane.
fn sphere() -> Session {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    s
}

fn positions(s: &mut Session) -> Vec<[f32; 3]> {
    s.mesh("sphere").positions
}

/// `v` mirrored across x = 0.
fn flip_x(v: [f32; 3]) -> [f32; 3] {
    [-v[0], v[1], v[2]]
}

/// `v` mirrored across y = 0.
fn flip_y(v: [f32; 3]) -> [f32; 3] {
    [v[0], -v[1], v[2]]
}

/// The bits of `p`, to compare points exactly.
fn bits(p: [f32; 3]) -> [u32; 3] {
    p.map(f32::to_bits)
}

/// The 120 degree turn about [1, 1, 1], rigid, about a frame at `o`, with `symmetry`.
fn turned(s: &mut Session, o: [f32; 3], symmetry: &Value) {
    let turn = json!({
        "frame": at(o),
        "op": { "rotate": { "axis": [1.0, 1.0, 1.0], "degrees": 120 } },
        "symmetry": symmetry,
    });
    s.ok("transform.apply", turn);
}

#[test]
fn a_turn_under_x_parallel_turns_the_far_side_the_same_way_about_the_reflected_origin() {
    let mut s = sphere();
    let start = positions(&mut s);
    let o = [0.5, 0.2, 0.1];
    turned(&mut s, o, &json!({ "parallel": { "x": true } }));
    // A point on the plane counts on the origin's side and is not held.
    let want: Vec<[f32; 3]> = (start.iter())
        .map(|&p| cycled(p, if p[0] < 0.0 { flip_x(o) } else { o }))
        .collect();
    let got = positions(&mut s);
    assert!(worst(&got, &want) < 2e-6, "{}", worst(&got, &want));
    let on_plane = start.iter().filter(|p| p[0] == 0.0).count();
    assert!(on_plane > 0, "the sphere has points on the plane");
}

#[test]
fn a_scale_under_x_parallel_stretches_the_far_side_along_the_same_turned_axes() {
    let mut s = sphere();
    let start = positions(&mut s);
    let o = [0.5, 0.2, 0.1];
    // The frame turned 45 degrees about Z: its X is the diagonal (1, 1, 0) / sqrt 2. Under
    // Mirror the far side would stretch along the other diagonal.
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let axes = [[half, half, 0.0], [-half, half, 0.0], [0.0, 0.0, 1.0]];
    let factors = [1.5, 0.5, 1.2];
    // sin and cos of 22.5 degrees, the quaternion's half angle.
    let (sin, cos) = (0.382_683_432_365_089_8, 0.923_879_532_511_286_7);
    let stretch = json!({
        "frame": { "origin": o, "axes": [0.0, 0.0, sin, cos] },
        "op": { "scale": factors },
        "symmetry": { "parallel": { "x": true } },
    });
    s.ok("transform.apply", stretch);
    let scaled = |p: [f32; 3], c: [f32; 3]| -> [f32; 3] {
        let v: [f64; 3] = std::array::from_fn(|i| f64::from(p[i]) - f64::from(c[i]));
        let along = |a: [f64; 3]| (0..3).map(|i| v[i] * a[i]).sum::<f64>();
        std::array::from_fn(|i| {
            let moved: f64 = (0..3)
                .map(|k| factors[k] * along(axes[k]) * axes[k][i])
                .sum();
            (f64::from(c[i]) + moved) as f32
        })
    };
    let want: Vec<[f32; 3]> = (start.iter())
        .map(|&p| scaled(p, if p[0] < 0.0 { flip_x(o) } else { o }))
        .collect();
    let got = positions(&mut s);
    assert!(worst(&got, &want) < 1e-5, "{}", worst(&got, &want));
}

#[test]
fn a_rigid_move_under_x_parallel_moves_the_whole_part_as_a_move_without_symmetry() {
    let moved = |symmetry: Value| {
        let mut s = sphere();
        let mut args = json!({ "frame": at([0.5, 0.2, 0.1]), "op": { "move": [0.3, 0.1, -0.2] } });
        if !symmetry.is_null() {
            args["symmetry"] = symmetry;
        }
        s.ok("transform.apply", args);
        s.hash()
    };
    assert_eq!(
        moved(json!({ "parallel": { "x": true, "z": true } })),
        moved(Value::Null),
        "the same part transform and the same points, bit for bit"
    );

    let mut s = sphere();
    let around = json!({ "radial": { "axis": "y", "count": 4 } });
    let move_around = json!({ "op": { "move": [0.0, 0.1, 0.0] }, "symmetry": around });
    let refused = s.err("transform.apply", move_around);
    assert_eq!(
        refused.path.as_deref(),
        Some("/symmetry/radial"),
        "{refused:?}"
    );
}

#[test]
fn x_parallel_with_y_mirror_gives_each_region_its_map_and_keeps_the_y_plane() {
    let mut s = sphere();
    let start = positions(&mut s);
    let o = [0.5, 0.5, 0.1];
    let mixed = json!({ "mirror": { "y": true }, "parallel": { "x": true } });
    turned(&mut s, o, &mixed);
    let across = |p: [f32; 3], a: usize| p[a] != 0.0 && (p[a] < 0.0) != (o[a] < 0.0);
    // The place map reflects the origin across both planes the point lies across; the
    // directions reflect across the Y Mirror plane alone. A point on the Y plane keeps its y.
    let want: Vec<[f32; 3]> = (start.iter())
        .map(|&p| {
            let (x, y) = (across(p, 0), across(p, 1));
            let c = [
                if x { -o[0] } else { o[0] },
                if y { -o[1] } else { o[1] },
                o[2],
            ];
            let mut q = if y {
                flip_y(cycled(flip_y(p), flip_y(c)))
            } else {
                cycled(p, c)
            };
            if p[1] == 0.0 {
                q[1] = p[1];
            }
            q
        })
        .collect();
    let got = positions(&mut s);
    assert!(worst(&got, &want) < 2e-6, "{}", worst(&got, &want));
    let regions =
        |x: bool, y: bool| (start.iter()).any(|&p| across(p, 0) == x && across(p, 1) == y);
    let all = [(false, false), (true, false), (false, true), (true, true)];
    assert!(
        all.iter().all(|&(x, y)| regions(x, y)),
        "points in every region"
    );
    let held = (start.iter().zip(&got)).filter(|(p, _)| p[1] == 0.0);
    assert!(held.clone().count() > 0, "points on the Y plane");
    assert!(held.clone().all(|(_, q)| q[1] == 0.0), "stay on it");
}

#[test]
fn under_x_parallel_a_far_point_is_weighted_as_mirror_weights_it_and_moves_unreflected() {
    // A soft move on a sphere with a mask that is not symmetric: the faces seen from the right
    // are left unmasked, the rest masked 0.75.
    let moved = |symmetry: Value, distance: &str| {
        let mut s = sphere();
        let right = json!({ "select": { "by": "facing" }, "camera": "right" });
        s.ok("visibility.hide", right);
        s.ok("mask.set", json!({ "value": 0.75 }));
        s.ok("visibility.show_all", json!({}));
        let mut args = json!({
            "frame": at([0.5, 0.1, 0.0]),
            "op": { "move": [0.2, 0.15, -0.1] },
            "falloff": 0,
            "reach": "150%",
            "distance": distance,
        });
        if !symmetry.is_null() {
            args["symmetry"] = symmetry;
        }
        let start = positions(&mut s);
        s.ok("transform.apply", args);
        (start, positions(&mut s))
    };
    for distance in ["straight", "surface"] {
        let (start, alone) = moved(Value::Null, distance);
        let (_, parallel) = moved(json!({ "parallel": { "x": true } }), distance);
        let (_, mirror) = moved(json!({ "mirror": { "x": true } }), distance);
        let near = |i: &usize| start[*i][0] >= 0.0;
        let (near, far): (Vec<usize>, Vec<usize>) = (0..start.len()).partition(near);
        assert!(
            near.iter().all(|&i| bits(parallel[i]) == bits(alone[i])),
            "{distance}: the origin's side and the plane move as with no symmetry"
        );
        let unreflected = |i: &usize| {
            let (p, a, b) = (start[*i], parallel[*i], mirror[*i]);
            ((a[0] - p[0]) + (b[0] - p[0])).abs() < 1e-6 && a[1..] == b[1..]
        };
        assert!(
            far.iter().all(unreflected),
            "{distance}: the far side moves by Mirror's weight, its x unreflected"
        );
        let moving = (far.iter())
            .filter(|&&i| bits(parallel[i]) != bits(start[i]))
            .count();
        assert!(moving * 2 > far.len(), "{distance}: the far side moves");
    }
}
