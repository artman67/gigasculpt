//! `mesh.project_detail` (decision 0037, section 4): detail from a sculpted part lands on a
//! smoother part of other topology within decision 0030's stated bound, the parts' transforms
//! are honored, UVs are never carried, each refusal leaves the scene as it was, and the same
//! call gives the same bytes.

use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh};
use gs_testkit::{Session, measure};
use serde_json::{Value, json};

use crate::support::{agent, worst};

/// A world-space stroke of `brush` and `size` over part "source" near `at`.
fn stroke(brush: &str, at: [f32; 3], size: f32) -> Value {
    let samples: Vec<Value> = (0..4)
        .map(|i| json!({ "position": [at[0] + 0.05 * i as f32, at[1], at[2]] }))
        .collect();
    json!({ "brush": brush, "part": "source", "samples": samples, "size": size })
}

/// A session with "source", a unit sphere of 3,456 quads with three Draw strokes and a Mask Pen
/// spot on it, and "target", a smooth unit sphere made by `target`.
fn scene(target: &[(&str, Value)]) -> Session {
    let mut s = agent();
    s.ok(
        "primitive.sphere",
        json!({ "name": "source", "subdivisions": 24 }),
    );
    for at in [[0.0, 0.0, 1.0], [0.6, 0.0, 0.8], [0.0, -1.0, 0.0]] {
        s.ok("brush.stroke", stroke("Draw", at, 0.35));
    }
    s.ok("brush.stroke", stroke("Mask Pen", [0.0, 0.0, -1.0], 0.8));
    for (command, args) in target {
        s.ok(command, args.clone());
    }
    s
}

/// A smooth sphere of 1,350 quads, a grid other than the source's.
fn sphere() -> Vec<(&'static str, Value)> {
    vec![(
        "primitive.sphere",
        json!({ "name": "target", "subdivisions": 15 }),
    )]
}

fn diagonal(mesh: &PlainMesh) -> f64 {
    let b = measure::bounds(&mesh.positions).unwrap_or_default();
    (0..3)
        .map(|k| f64::from(b[1][k] - b[0][k]) * f64::from(b[1][k] - b[0][k]))
        .sum::<f64>()
        .sqrt()
}

fn length(p: [f32; 3], q: [f32; 3]) -> f64 {
    (0..3)
        .map(|k| f64::from(p[k] - q[k]) * f64::from(p[k] - q[k]))
        .sum::<f64>()
        .sqrt()
}

/// Section 4's stated tolerance, as `level.unfreeze` is held to it: every point of the new top
/// lies on the source surface (within 1e-5 of its diagonal, every point being in reach here),
/// and the source points lie on average within one source edge length (the mean) of a target
/// point. The target is a voxel remesh of a coarse sphere, so no grid lines up with the source's.
#[test]
fn sculpted_detail_lands_on_a_smoother_target_of_other_topology_within_the_bound() {
    let mut target = sphere();
    target[0].1["subdivisions"] = json!(6);
    target.push((
        "mesh.voxel_remesh",
        json!({ "part": "target", "resolution": 16 }),
    ));
    let mut s = scene(&target);
    let args = json!({ "part": "target", "source": "source", "divide": true, "distance": "5%" });
    let done = s.ok("mesh.project_detail", args).result;
    let (source, new) = (s.mesh("source"), s.mesh("target"));
    assert!(
        done["levels_added"].as_u64().is_some_and(|n| n >= 1),
        "{done}"
    );
    assert_eq!(done["out_of_reach"], json!(0), "every point was in reach");
    assert_eq!(done["moved"].as_u64(), Some(new.positions.len() as u64));
    let diag = diagonal(&source);
    let points = PlainMesh {
        positions: new.positions.clone(),
        ..PlainMesh::default()
    };
    let on = measure::hausdorff_from(&points, &source);
    assert!(on <= 1e-5 * diag, "a target point is {on} off the source");
    let edges: Vec<f64> = (source.faces.iter())
        .flat_map(|f| (0..4).map(move |k| (f[k], f[(k + 1) % 4])))
        .map(|(a, b)| length(source.positions[a as usize], source.positions[b as usize]))
        .collect();
    let mean_edge = edges.iter().sum::<f64>() / edges.len() as f64;
    let total: f64 = (source.positions.iter())
        .map(|&p| {
            (new.positions.iter())
                .map(|&q| length(p, q))
                .fold(f64::INFINITY, f64::min)
        })
        .sum();
    let mean = total / source.positions.len() as f64;
    assert!(
        mean <= mean_edge,
        "source points are {mean} from the target on average"
    );
}

/// Decision 0029: the parts are matched where they stand in the scene. Turned and scaled
/// differently (whole parts, the source's mask not holding it back), the parts project as they
/// do with their transforms baked into their points.
#[test]
fn the_parts_transforms_are_honored() {
    let place = |s: &mut Session, part: &str, axis: [f32; 3], degrees: f32| {
        let frame = json!({ "origin": [0.0, 0.0, 0.0], "axes": [0.0, 0.0, 0.0, 1.0] });
        let ops = [
            json!({ "rotate": { "axis": axis, "degrees": degrees } }),
            json!({ "scale": [1.5, 1.5, 1.5] }),
        ];
        for op in ops {
            let args = json!({ "part": part, "frame": frame, "op": op, "mask": false });
            s.ok("transform.apply", args);
            let bytes = s.history()["steps"][0]["bytes"].as_u64();
            assert!(bytes < Some(1024), "{op} moved the points, not the part");
        }
    };
    let call = json!({ "part": "target", "source": "source", "carry": ["mask"] });
    let run = |bake: bool| {
        let mut s = scene(&sphere());
        place(&mut s, "source", [1.0, 2.0, 3.0], 35.0);
        place(&mut s, "target", [0.0, 0.0, 1.0], 20.0);
        if bake {
            for part in ["source", "target"] {
                s.ok("part.apply_transform", json!({ "part": part }));
            }
        }
        let done = s.ok("mesh.project_detail", call.clone()).result;
        (s.mesh("target"), done)
    };
    let ((placed, done), (baked, want)) = (run(false), run(true));
    assert!(done["moved"].as_u64().is_some_and(|n| n > 1000), "{done}");
    assert_eq!(done["out_of_reach"], want["out_of_reach"]);
    let gap = worst(&placed.positions, &baked.positions);
    assert!(gap < 1e-5 * diagonal(&baked) as f32, "{gap} apart");
    let mask = |m: &PlainMesh| match m.attributes.get("mask").map(|a| &a.values) {
        Some(AttrValues::F32(v)) => v.clone(),
        _ => panic!("the mask was not carried"),
    };
    let (a, b) = (mask(&placed), mask(&baked));
    assert!(a.iter().any(|&m| m > 0.5), "the masked spot came across");
    let off = (a.iter().zip(&b))
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max);
    assert!(off < 1e-3, "the carried masks differ by {off}");
}

#[test]
fn carry_refuses_uv_names() {
    let mut s = scene(&sphere());
    for name in ["uv0.u", "uv0.v", "uv3.u", "uv3.v"] {
        let args = json!({ "part": "target", "source": "source", "carry": ["mask", name] });
        let err = s.err("mesh.project_detail", args);
        assert_eq!(err.code, ErrorCode::ArgsInvalid);
        assert_eq!(err.path.as_deref(), Some("/carry/1"));
        assert!(err.message.contains("UV"), "{}", err.message);
    }
}

/// Decision 0037, section 4's refusals, each leaving the scene exactly as it was.
#[test]
fn each_refusal_leaves_the_scene_as_it_was() {
    let mut s = scene(&sphere());
    let hidden = PlainMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        faces: vec![[0, 1, 2, u32::MAX]],
        attributes: [(
            "hide".to_string(),
            Attribute {
                domain: AttrDomain::Face,
                values: AttrValues::U32(vec![1]),
            },
        )]
        .into(),
    };
    s.add_part("hidden", &hidden);
    let call = |extra: Value| {
        let mut args = json!({ "part": "target", "source": "source" });
        (extra.as_object().into_iter().flatten()).for_each(|(k, v)| args[k] = v.clone());
        args
    };
    let refused = [
        (call(json!({ "source": "target" })), ErrorCode::ArgsInvalid),
        (call(json!({ "source": "nowhere" })), ErrorCode::NotFound),
        (call(json!({ "distance": 0 })), ErrorCode::ArgsInvalid),
        (call(json!({ "distance": "-1%" })), ErrorCode::ArgsInvalid),
        (call(json!({ "carry": ["color"] })), ErrorCode::ArgsInvalid),
        (call(json!({ "source": "hidden" })), ErrorCode::Precondition),
    ];
    for (args, code) in refused {
        assert_eq!(
            s.err("mesh.project_detail", args.clone()).code,
            code,
            "{args}"
        );
    }
    s.ok("level.subdivide", json!({ "part": "target" }));
    let divide = call(json!({ "divide": true }));
    assert_eq!(
        s.err("mesh.project_detail", divide).code,
        ErrorCode::Precondition
    );
    s.ok("level.freeze", json!({ "part": "target" }));
    assert_eq!(
        s.err("mesh.project_detail", call(json!({}))).code,
        ErrorCode::Precondition
    );
}

/// The same scene and call give the same bytes in a second session, projected in place and
/// divided.
#[test]
fn the_same_call_gives_the_same_bytes_twice() {
    for extra in [
        json!({ "carry": ["mask"], "blur": 30 }),
        json!({ "divide": true }),
    ] {
        let mut args = json!({ "part": "target", "source": "source" });
        (extra.as_object().into_iter().flatten()).for_each(|(k, v)| args[k] = v.clone());
        let run = || {
            let mut s = scene(&sphere());
            let reply = s.ok("mesh.project_detail", args.clone()).result;
            let mesh = serde_json::to_string(&s.mesh("target")).unwrap_or_default();
            (s.hash(), mesh, reply)
        };
        let (first, second) = (run(), run());
        assert!(first.1.len() > 1000, "the target was read");
        assert_eq!(first, second, "{args}");
    }
}
