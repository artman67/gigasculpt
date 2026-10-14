//! The primitives against geometry (decision 0047): the cylinder and the sphere's uv form
//! checked against what they are by definition, the axis as a metamorphic relation (turning to
//! an axis is cycling the coordinates), the sphere arguments old logs carry, and a log replay.

use gs_protocol::{ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::{Expect, Session, validate};
use serde_json::{Value, json};

use crate::support::agent;

/// `mesh.stats` of one part.
fn stats(s: &mut Session, part: &str) -> Value {
    s.query("mesh.stats", json!({ "part": part }))["parts"][0].clone()
}

/// Vertices, faces, triangles, quads and boundary edges, as `mesh.stats` counts them.
fn counts(s: &mut Session, part: &str) -> Value {
    let stats = stats(s, part);
    json!([
        stats["vertices"],
        stats["faces"],
        stats["triangles"],
        stats["quads"],
        stats["boundary_edges"]
    ])
}

/// A face's corners, without the triangle marker.
fn corners(face: &[u32; 4]) -> Vec<u32> {
    face.iter().copied().filter(|&v| v != NO_VERTEX).collect()
}

/// The faces with a corner at vertex `v`, as (triangles, quads).
fn faces_at(mesh: &PlainMesh, v: u32) -> (usize, usize) {
    let at = mesh.faces.iter().map(corners).filter(|c| c.contains(&v));
    at.fold(
        (0, 0),
        |(t, q), c| if c.len() == 3 { (t + 1, q) } else { (t, q + 1) },
    )
}

/// The extent of the mesh along each world axis.
fn extents(mesh: &PlainMesh) -> [f64; 3] {
    std::array::from_fn(|i| {
        let values = mesh.positions.iter().map(|p| f64::from(p[i]));
        let (lo, hi) = values.fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        hi - lo
    })
}

/// Newell's normal of a face, in f64: it points to the side the face is wound
/// counter-clockwise from.
fn normal(mesh: &PlainMesh, face: &[u32; 4]) -> [f64; 3] {
    let points: Vec<[f64; 3]> = corners(face)
        .iter()
        .map(|&v| mesh.positions[v as usize].map(f64::from))
        .collect();
    let mut n = [0.0; 3];
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

/// Every face of a convex mesh around `center` faces away from it.
fn assert_normals_out(mesh: &PlainMesh, center: [f64; 3], what: &str) {
    for (f, face) in mesh.faces.iter().enumerate() {
        let c = corners(face);
        let mid: [f64; 3] = std::array::from_fn(|i| {
            let sum: f64 = c
                .iter()
                .map(|&v| f64::from(mesh.positions[v as usize][i]))
                .sum();
            sum / c.len() as f64 - center[i]
        });
        let n = normal(mesh, face);
        let out = n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2];
        assert!(out > 0.0, "{what}: face {f} faces inward ({out})");
    }
}

const AXES: [(&str, usize); 3] = [("x", 0), ("y", 1), ("z", 2)];

/// Catalog `tool-misc.primitive-panel`: a cylinder made along X has its length along world X,
/// and so on for each axis.
#[test]
fn a_cylinder_made_along_an_axis_has_its_length_along_it() {
    let mut s = agent();
    for (axis, index) in AXES {
        let name = format!("along-{axis}");
        s.ok(
            "primitive.cylinder",
            json!({
                "name": name, "radius": 0.5, "height": 3, "segments": 12, "divisions": 3,
                "cap_rings": 2, "axis": axis, "center": [1, 2, 3]
            }),
        );
        assert_eq!(counts(&mut s, &name), json!([74, 84, 24, 60, 0]), "{name}");
        let extents = extents(&s.mesh(&name));
        for (i, got) in extents.iter().enumerate() {
            let want = if i == index { 3.0 } else { 1.0 };
            assert!((got - want).abs() < 1e-6, "{name}: extent {i} is {got}");
        }
    }
    s.ok("primitive.cylinder", json!({ "name": "plain" }));
    let plain = s.mesh("plain");
    assert_eq!(extents(&plain).map(f64::round), [2.0, 2.0, 2.0]);
    assert_eq!(counts(&mut s, "plain"), json!([482, 512, 64, 448, 0]));
}

/// Catalog `tool-misc.primitive-panel`: a radial primitive has exactly one center vertex per
/// cap, ringed by a fan of triangles; every other face is a quad.
#[test]
fn a_cylinder_has_one_welded_center_vertex_per_cap() {
    let mut s = agent();
    for (axis, index) in AXES {
        let name = format!("caps-{axis}");
        let center = [1.0, -2.0, 0.5];
        s.ok(
            "primitive.cylinder",
            json!({ "name": name, "segments": 10, "height": 4, "axis": axis, "center": center }),
        );
        let mesh = s.mesh(&name);
        let on_axis: Vec<u32> = (0..mesh.positions.len() as u32)
            .filter(|&v| {
                let p = mesh.positions[v as usize];
                (0..3).all(|i| i == index || (f64::from(p[i]) - center[i]).abs() < 1e-6)
            })
            .collect();
        assert_eq!(on_axis.len(), 2, "{name}: vertices on the axis {on_axis:?}");
        let mut ends: Vec<f64> = on_axis
            .iter()
            .map(|&v| f64::from(mesh.positions[v as usize][index]) - center[index])
            .collect();
        ends.sort_by(f64::total_cmp);
        assert_eq!(ends, [-2.0, 2.0], "{name}: the cap centers");
        for &v in &on_axis {
            assert_eq!(faces_at(&mesh, v), (10, 0), "{name}: a fan of 10 at {v}");
        }
        assert_eq!(stats(&mut s, &name)["triangles"], json!(20), "{name}");
    }
}

/// The cylinder and the uv sphere, at their defaults and their smallest, along each axis: closed
/// manifolds of genus 0, every face wound to face outward.
#[test]
fn cylinders_and_uv_spheres_are_closed_with_normals_out() {
    let mut s = agent();
    let center = [0.5, 1.0, -1.5];
    let shapes = [
        ("primitive.cylinder", json!({})),
        (
            "primitive.cylinder",
            json!({ "segments": 3, "divisions": 1, "cap_rings": 1, "axis": "z" }),
        ),
        ("primitive.cylinder", json!({ "segments": 7, "axis": "x" })),
        ("primitive.sphere", json!({ "form": "uv" })),
        (
            "primitive.sphere",
            json!({ "segments": 3, "rings": 2, "axis": "x" }),
        ),
        (
            "primitive.sphere",
            json!({ "segments": 5, "rings": 7, "axis": "z" }),
        ),
    ];
    for (i, (command, mut args)) in shapes.into_iter().enumerate() {
        let name = format!("shape-{i}");
        args["name"] = json!(name);
        args["center"] = json!(center);
        s.ok(command, args.clone());
        let mesh = s.mesh(&name);
        let what = format!("{command} {args}");
        assert_eq!(validate(&mesh, Expect::closed(2)), Ok(()), "{what}");
        assert_normals_out(&mesh, center, &what);
    }
}

/// Catalog `tool-misc.primitive-sphere`, for the uv form: radius 1, 64 by 32, every vertex at
/// distance 1, one vertex at each pole with a fan of triangles, quads everywhere else.
#[test]
fn the_uv_sphere_is_64_by_32_with_fans_at_the_poles() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "form": "uv", "name": "uv" }));
    assert_eq!(counts(&mut s, "uv"), json!([1986, 2048, 128, 1920, 0]));
    let mesh = s.mesh("uv");
    for p in &mesh.positions {
        let r = p
            .iter()
            .map(|&c| f64::from(c) * f64::from(c))
            .sum::<f64>()
            .sqrt();
        assert!((r - 1.0).abs() < 1e-6, "a vertex at {r} from the center");
    }
    for (pole, y) in [("north", 1.0), ("south", -1.0)] {
        let at: Vec<u32> = (0..mesh.positions.len() as u32)
            .filter(|&v| {
                let p = mesh.positions[v as usize].map(f64::from);
                (p[0].abs() + (p[1] - y).abs() + p[2].abs()) < 1e-12
            })
            .collect();
        assert_eq!(at.len(), 1, "one {pole} pole vertex: {at:?}");
        assert_eq!(faces_at(&mesh, at[0]), (64, 0), "the {pole} pole's fan");
    }
    s.ok(
        "primitive.sphere",
        json!({ "form": "uv", "axis": "x", "radius": 2, "center": [1, 0, 0], "name": "on-x" }),
    );
    let on_x = s.mesh("on-x");
    for pole in [[3.0, 0.0, 0.0], [-1.0, 0.0, 0.0]] {
        assert!(on_x.positions.contains(&pole), "a pole at {pole:?}");
    }
}

/// The positions of a mesh as bits, sorted, after `turn` is applied to each.
fn turned(mesh: &PlainMesh, turn: impl Fn([f32; 3]) -> [f32; 3]) -> Vec<[u32; 3]> {
    let mut bits: Vec<[u32; 3]> = mesh
        .positions
        .iter()
        .map(|&p| turn(p).map(f32::to_bits))
        .collect();
    bits.sort_unstable();
    bits
}

/// Making a primitive along X or Z gives exactly its Y-made points with the coordinates cycled,
/// and the cube is the same cube along any axis.
#[test]
fn the_axis_cycles_the_coordinates_of_every_primitive() {
    let mut s = agent();
    let shapes = [
        ("primitive.cube", json!({ "subdivisions": 3 })),
        (
            "primitive.sphere",
            json!({ "subdivisions": 3, "radius": 1.5 }),
        ),
        ("primitive.sphere", json!({ "segments": 9, "rings": 5 })),
        ("primitive.cylinder", json!({ "segments": 7, "height": 3 })),
    ];
    for (i, (command, args)) in shapes.into_iter().enumerate() {
        let mut made = Vec::new();
        for (axis, _) in AXES {
            let name = format!("shape-{i}-{axis}");
            let mut args = args.clone();
            args["axis"] = json!(axis);
            args["name"] = json!(name);
            s.ok(command, args);
            made.push(s.mesh(&name));
        }
        let what = format!("{command} {args}");
        assert_eq!(
            turned(&made[0], |p| p),
            turned(&made[1], |[x, y, z]| [y, z, x]),
            "{what} x"
        );
        assert_eq!(
            turned(&made[2], |p| p),
            turned(&made[1], |[x, y, z]| [z, x, y]),
            "{what} z"
        );
    }
    let cube = |s: &mut Session, axis: &str| {
        s.ok(
            "primitive.cube",
            json!({ "name": axis, "axis": axis, "subdivisions": 2 }),
        );
        turned(&s.mesh(axis), |p| p)
    };
    let along_y = cube(&mut s, "y");
    assert_eq!(cube(&mut s, "x"), along_y, "the cube along X");
    assert_eq!(cube(&mut s, "z"), along_y, "the cube along Z");
}

/// A sphere line with `subdivisions` and no `form`, as every log before decision 0047 has it,
/// makes the cube form whatever the default; `segments` or `rings` alone make the uv form; one
/// form's fields with the other's are refused.
#[test]
fn the_sphere_form_follows_its_fields() {
    let mut s = agent();
    let made = |s: &mut Session, name: &str, mut args: Value| {
        args["name"] = json!(name);
        s.ok("primitive.sphere", args);
        counts(s, name)
    };
    let old = json!({ "subdivisions": 8, "radius": 1.0, "center": [0.0, 0.0, 0.0] });
    let cube = json!([386, 384, 0, 384, 0]);
    assert_eq!(made(&mut s, "old", old), cube, "an old log line");
    assert_eq!(made(&mut s, "cube", json!({ "form": "cube" })), cube);
    assert_eq!(made(&mut s, "default", json!({})), cube, "the default form");
    assert_eq!(
        made(&mut s, "few", json!({ "subdivisions": 3 })),
        json!([56, 54, 0, 54, 0])
    );
    let around = made(&mut s, "around", json!({ "segments": 8 }));
    assert_eq!(
        around,
        json!([250, 256, 16, 240, 0]),
        "segments 8 by 32 rings"
    );
    let rings = made(&mut s, "rings", json!({ "rings": 4 }));
    assert_eq!(
        rings,
        json!([194, 256, 128, 128, 0]),
        "64 segments by 4 rings"
    );
    for (args, path) in [
        (json!({ "form": "uv", "subdivisions": 3 }), "/subdivisions"),
        (json!({ "form": "cube", "segments": 8 }), "/segments"),
        (json!({ "form": "cube", "rings": 8 }), "/rings"),
        (json!({ "subdivisions": 3, "rings": 4 }), "/rings"),
    ] {
        let err = s.err("primitive.sphere", args.clone());
        assert_eq!(err.code, ErrorCode::ArgsInvalid, "{args}: {err:?}");
        assert_eq!(err.path.as_deref(), Some(path), "{args}: {err:?}");
    }
}

/// Each call logs its form, its form's fields and its axis, and the log replays to the same
/// scene, bit for bit.
#[test]
fn primitives_log_their_form_and_axis_and_replay_bit_for_bit() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("primitives-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let log = dir.join("primitives.jsonl");
    let mut s = crate::determinism::seeded(3, Some(log.clone()));
    for (command, args) in [
        ("primitive.sphere", json!({})),
        (
            "primitive.sphere",
            json!({ "segments": 8, "axis": "z", "center": [2, 0, 0] }),
        ),
        ("primitive.cube", json!({ "axis": "x" })),
        (
            "primitive.cylinder",
            json!({ "axis": "x", "radius": 0.5, "height": 4 }),
        ),
        ("part.add", json!({ "shape": "sphere" })),
    ] {
        s.ok(command, args);
    }
    let (_, records) = crate::support::log_records(&log);
    let args: Vec<&Value> = records.iter().map(|r| &r.args).collect();
    for (i, (key, want)) in [
        ("form", "cube"),
        ("form", "uv"),
        ("axis", "x"),
        ("axis", "x"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(args[i][key], json!(want), "line {i}: {}", args[i]);
    }
    assert_eq!(args[0]["subdivisions"], json!(8), "{}", args[0]);
    assert_eq!(args[0]["axis"], json!("y"), "{}", args[0]);
    assert_eq!(
        (args[1]["segments"].clone(), args[1]["rings"].clone()),
        (json!(8), json!(32))
    );
    assert!(args[1].get("subdivisions").is_none(), "{}", args[1]);
    assert!(args[0].get("segments").is_none(), "{}", args[0]);
    assert_eq!(args[3]["divisions"], json!(8), "{}", args[3]);
    let mut replay = crate::determinism::seeded(4, None);
    crate::support::EngineSession::replay_log(&mut replay, &log);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(replay.hash(), s.hash());
}
