//! `boolean.apply` and `boolean.check` (decision 0057, sections 2, 5, 6 and 7), and
//! `part.remesh` folding its parts by their operations (section 3): the groups of the list cut
//! into new parts in one undo step, the ladder and its solvers, what each result carries from
//! its operands, the warnings, and the check behind Show Issues and Show Coplanar.

use std::path::PathBuf;

use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, NO_VERTEX, PlainMesh, arity};
use gs_testkit::Session;
use gs_testkit::claims::run::Harness;
use gs_testkit::fixtures::cube;
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::groups::ids;
use crate::parts::row;
use crate::support::{agent, log_records};

/// `mesh` scaled by `scale` about the origin, then moved by `by`.
fn placed(mut mesh: PlainMesh, scale: f32, by: [f32; 3]) -> PlainMesh {
    for p in &mut mesh.positions {
        *p = std::array::from_fn(|a| p[a] * scale + by[a]);
    }
    mesh
}

/// `mesh` with every face wound the other way.
fn inside_out(mut mesh: PlainMesh) -> PlainMesh {
    for face in &mut mesh.faces {
        let n = arity(*face);
        face[..n].reverse();
    }
    mesh
}

/// `mesh` with its last face taken off, and that face's values: open along the face's edges.
fn opened(mut mesh: PlainMesh) -> PlainMesh {
    mesh.faces.pop();
    let faces = mesh.attributes.values_mut();
    for column in faces.filter(|c| c.domain == AttrDomain::Face) {
        match &mut column.values {
            AttrValues::F32(v) => drop(v.pop()),
            AttrValues::U32(v) => drop(v.pop()),
        }
    }
    mesh
}

/// `mesh` with a column set.
fn with(mut mesh: PlainMesh, name: &str, domain: AttrDomain, values: AttrValues) -> PlainMesh {
    mesh.attributes
        .insert(name.into(), Attribute { domain, values });
    mesh
}

/// The body: a cube of 96 faces over [-1, 1], each face's group its index plus 1.
fn body() -> PlainMesh {
    let mesh = cube(4);
    let groups = (1..=mesh.faces.len() as u32).collect();
    with(mesh, "group", AttrDomain::Face, AttrValues::U32(groups))
}

/// The cutter: a cube 3 wide over x 0.1 to 3.1, each face's group its index plus 100.
fn cutter() -> PlainMesh {
    let mesh = placed(cube(2), 1.5, [1.6, 0.0, 0.0]);
    let groups = (100..100 + mesh.faces.len() as u32).collect();
    with(mesh, "group", AttrDomain::Face, AttrValues::U32(groups))
}

/// The body and the cutter as parts `body` and `cutter`, the cutter marked Difference.
fn cut(mut s: Session, body: &PlainMesh, cutter: &PlainMesh) -> Session {
    s.add_part("body", body);
    s.add_part("cutter", cutter);
    let difference = json!({ "part": "cutter", "operation": "difference" });
    s.ok("part.set_operation", difference);
    s
}

/// A cube and a bigger cube over its +x side, made with `primitive.cube`, the second marked
/// Difference: the result is the cube's x -1 to 0.1, volume 4.4.
pub(crate) fn primitives(mut s: Session) -> Session {
    s.ok(
        "primitive.cube",
        json!({ "name": "body", "subdivisions": 2 }),
    );
    let over = json!({ "name": "cutter", "size": 3, "center": [1.6, 0, 0] });
    s.ok("primitive.cube", over);
    s.ok(
        "part.set_operation",
        json!({ "part": "cutter", "operation": "difference" }),
    );
    s
}

/// The parts in list order.
fn list(s: &mut Session) -> Vec<String> {
    let list = s.query("part.list", json!({}));
    let rows = list["parts"].as_array().cloned().unwrap_or_default();
    (rows.iter())
        .map(|row| row["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn volume(s: &mut Session, part: &str) -> f64 {
    let stats = s.query("mesh.stats", json!({ "part": part }));
    stats["parts"][0]["volume"].as_f64().unwrap_or_default()
}

fn active(s: &mut Session) -> Value {
    s.query("session.state", json!({}))["parts.active"].clone()
}

fn f32s<'a>(mesh: &'a PlainMesh, name: &str) -> &'a [f32] {
    match mesh.attributes.get(name).map(|c| &c.values) {
        Some(AttrValues::F32(v)) => v,
        _ => panic!("no number column {name}"),
    }
}

/// Two parts, the second Difference: one new part at the end of the list, named `boolean`,
/// starting its group as a union, holding the first less the overlap, with both operands hidden
/// in the same step; one undo gives the scene back exactly, and redo the result.
#[test]
fn apply_cuts_the_second_part_from_the_first_into_a_new_part_in_one_undo_step() {
    let mut s = primitives(agent());
    let before = s.hash();
    let steps = s.history()["undoable"].as_u64().unwrap_or_default();
    let done = s.ok("boolean.apply", json!({}));
    let group = &done.result["groups"][0];
    assert_eq!(group["part"]["part"], json!("boolean"), "{}", done.result);
    assert_eq!(group["operands"], json!(["body", "cutter"]));
    assert_eq!(list(&mut s), ["body", "cutter", "boolean"], "at the end");
    let made = row(&mut s, "boolean");
    assert_eq!(
        (&made["start"], &made["operation"]),
        (&json!(true), &json!("union"))
    );
    let v = volume(&mut s, "boolean");
    assert!((v - 4.4).abs() < 1e-4, "the body less the overlap: {v}");
    for name in ["body", "cutter"] {
        assert_eq!(row(&mut s, name)["visible"], json!(false), "{name} hidden");
    }
    let after = s.hash();
    assert_eq!(
        s.history()["undoable"].as_u64(),
        Some(steps + 1),
        "one step"
    );
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "undo");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), after, "redo");
}

/// The active part stays when it is no operand, and is the first result when it was one.
#[test]
fn the_active_part_stays_unless_it_was_an_operand() {
    let mut s = primitives(agent());
    s.ok(
        "primitive.sphere",
        json!({ "name": "aside", "center": [6, 0, 0] }),
    );
    let pair = json!([[["body", "union"], ["cutter", "difference"]]]);
    s.ok("boolean.apply", json!({ "groups": pair }));
    assert_eq!(active(&mut s), json!("aside"), "no operand");
    s.ok("history.undo", json!({}));
    s.ok("part.select", json!({ "part": "cutter" }));
    s.ok("boolean.apply", json!({ "groups": pair }));
    assert_eq!(
        active(&mut s),
        json!("boolean"),
        "an operand: the first result"
    );
}

/// Left out, `groups` is every group of the list with a part that shows (a group that only
/// holds hidden parts makes nothing, and one part alone makes a copy of it), and the log holds
/// it resolved, so the call replays as it ran.
#[test]
fn groups_left_out_resolve_to_the_groups_that_show_and_are_logged_resolved() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("boolean-groups-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let log = dir.join("boolean.jsonl");
    let mut s = primitives(seeded(1, Some(log.clone())));
    let at = |x: f32| json!({ "subdivisions": 2, "center": [x, 0, 0] });
    s.ok("primitive.sphere", with_name(at(6.0), "lone"));
    s.ok("primitive.sphere", with_name(at(9.0), "gone"));
    for (part, start) in [("lone", true), ("gone", true)] {
        s.ok(
            "part.set_operation",
            json!({ "part": part, "start": start }),
        );
    }
    s.ok(
        "part.set_visible",
        json!({ "parts": ["gone"], "visible": false }),
    );
    s.ok("part.select", json!({ "part": "body" }));
    let done = s.ok("boolean.apply", json!({}));
    let made: Vec<&Value> = (done.result["groups"].as_array().into_iter().flatten())
        .map(|g| &g["part"]["part"])
        .collect();
    assert_eq!(
        made,
        [&json!("boolean"), &json!("boolean-2")],
        "{}",
        done.result
    );
    assert_eq!(
        row(&mut s, "lone")["visible"],
        json!(false),
        "the copy's operand"
    );
    let (_, records) = log_records(&log);
    std::fs::remove_dir_all(&dir).ok();
    let logged = records.iter().rfind(|r| r.command == "boolean.apply");
    let args = logged.map(|r| r.args.clone()).unwrap_or_default();
    let resolved = json!([
        [["body", "union"], ["cutter", "difference"]],
        [["lone", "union"]]
    ]);
    assert_eq!(args["groups"], resolved, "{args}");
}

fn with_name(mut args: Value, name: &str) -> Value {
    args["name"] = json!(name);
    args
}

/// The list order is the only precedence (catalog `live-boolean.evaluation-order`): below the
/// cutter, a union part refills the part of the hole it overlaps; moved above the cutter, it is
/// cut with the body.
#[test]
fn an_adder_below_a_subtractor_refills_its_hole_and_above_it_does_not() {
    for (above, want) in [(false, 5.3), (true, 4.4)] {
        let mut s = primitives(agent());
        let plug = json!({ "name": "plug", "size": 1, "center": [0.5, 0, 0] });
        s.ok("primitive.cube", plug);
        if above {
            s.ok(
                "part.reorder",
                json!({ "part": "plug", "to": { "place": 1 } }),
            );
        }
        s.ok("boolean.apply", json!({}));
        let v = volume(&mut s, "boolean");
        assert!(
            (v - want).abs() < 1e-4,
            "the plug above the cutter {above}: {v}"
        );
    }
}

/// `manifold` and `voxel` each run their own path and say so; `auto` takes the exact engine
/// for clean operands and reports it.
#[test]
fn each_solver_runs_its_path_and_auto_reports_which_ran() {
    for (solver, ran) in [
        ("manifold", "manifold"),
        ("voxel", "voxel"),
        ("auto", "manifold"),
    ] {
        let mut s = primitives(agent());
        let args = json!({ "solver": solver, "resolution": 48 });
        let done = s.ok("boolean.apply", args);
        assert_eq!(done.result["groups"][0]["solver"], json!(ran), "{solver}");
        let v = volume(&mut s, "boolean");
        assert!((v - 4.4).abs() < 0.1, "{solver}: {v}");
        let mesh = s.mesh("boolean");
        let quads = mesh.faces.iter().all(|f| f[3] != NO_VERTEX);
        assert_eq!(
            quads,
            ran == "voxel",
            "{solver}: the voxel fold gives quads only"
        );
    }
}

/// `resolution` is pinned in the log when a voxel step can run, and left out when none can.
#[test]
fn resolution_is_pinned_in_the_log_when_a_voxel_step_runs() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("boolean-resolution-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let log = dir.join("boolean.jsonl");
    let mut s = primitives(seeded(1, Some(log.clone())));
    s.ok("boolean.apply", json!({ "solver": "voxel" }));
    s.ok("history.undo", json!({}));
    s.ok("boolean.apply", json!({ "solver": "manifold" }));
    let (_, records) = log_records(&log);
    std::fs::remove_dir_all(&dir).ok();
    let calls: Vec<&Value> = (records.iter())
        .filter(|r| r.command == "boolean.apply")
        .map(|r| &r.args)
        .collect();
    assert_eq!(calls.len(), 2, "{records:?}");
    assert_eq!(calls[0]["resolution"], json!(128), "voxel: {}", calls[0]);
    assert!(
        calls[1].get("resolution").is_none(),
        "manifold: {}",
        calls[1]
    );
}

/// Refused, leaving the scene as it was: no group with two parts that show, an operand with no
/// faces, a voxel step over its cell limit, and with `manifold` an operand that needs a repair,
/// by name.
#[test]
fn apply_refuses_what_it_cannot_cut() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let e = s.err("boolean.apply", json!({}));
    assert_eq!(e.code, ErrorCode::Precondition, "{e:?}");
    assert!(e.message.contains("two parts"), "{}", e.message);
    let mut s = primitives(agent());
    s.add_part("empty", &PlainMesh::default());
    let groups = json!([[["body", "union"], ["empty", "difference"]]]);
    let e = s.err("boolean.apply", json!({ "groups": groups }));
    assert!(
        e.message.contains("'empty'") && e.message.contains("no faces"),
        "{e:?}"
    );
    let groups = json!([[["body", "union"], ["cutter", "difference"]]]);
    let fine = json!({ "groups": groups, "solver": "voxel", "resolution": 4096 });
    let e = s.err("boolean.apply", fine);
    assert_eq!(e.code, ErrorCode::Limit, "{e:?}");
    let mut s = cut(agent(), &body(), &opened(cutter()));
    let e = s.err("boolean.apply", json!({ "solver": "manifold" }));
    assert_eq!(e.code, ErrorCode::Precondition, "{e:?}");
    assert!(
        e.message.contains("'cutter'") && e.message.contains("open"),
        "{e:?}"
    );
}

/// The carry on the exact path: face groups from each face's source, the mask and the paint
/// exact at kept points and blended at cut ones, the earlier operand's values on the seam, and
/// the hidden faces made surface with the `hide` column dropped and reported.
#[test]
fn groups_paint_and_mask_carry_and_the_seam_takes_the_earlier_operand() {
    let ramp = |p: [f32; 3]| (p[0] + 1.0) / 4.0;
    let red = 0xff00_00ff_u32;
    let b = body();
    let mask: Vec<f32> = b.positions.iter().map(|&p| ramp(p)).collect();
    let b = with(b, "mask", AttrDomain::Vertex, AttrValues::F32(mask));
    let n = b.positions.len();
    let b = with(
        b,
        "color",
        AttrDomain::Vertex,
        AttrValues::U32(vec![red; n]),
    );
    let mut hide = vec![0; b.faces.len()];
    hide[0] = 1;
    let b = with(b, "hide", AttrDomain::Face, AttrValues::U32(hide));
    let c = cutter();
    let ones = vec![1.0; c.positions.len()];
    let c = with(c, "mask", AttrDomain::Vertex, AttrValues::F32(ones));
    let mut s = cut(agent(), &b, &c);
    let done = s.ok("boolean.apply", json!({}));
    assert_eq!(
        done.result["groups"][0]["dropped"],
        json!(["hide"]),
        "{}",
        done.result
    );
    let out = s.mesh("boolean");
    assert!(
        !out.attributes.contains_key("hide"),
        "hidden faces are surface"
    );
    let (groups, mask, color) = (ids(&out, "group"), f32s(&out, "mask"), ids(&out, "color"));
    let from_body = |f: usize| (1..=96).contains(&groups[f]);
    assert!(
        (0..out.faces.len()).all(|f| from_body(f) || groups[f] >= 100),
        "{groups:?}"
    );
    let mut on_body = vec![false; out.positions.len()];
    for (f, face) in out.faces.iter().enumerate() {
        for &v in face.iter().filter(|&&v| v != NO_VERTEX) {
            on_body[v as usize] |= from_body(f);
        }
    }
    let bits = |p: &[f32; 3]| p.map(f32::to_bits);
    let kept = |p: &[f32; 3]| b.positions.iter().position(|q| bits(q) == bits(p));
    for (v, p) in out.positions.iter().enumerate() {
        if !on_body[v] {
            assert_eq!(
                (mask[v], color[v]),
                (1.0, u32::MAX),
                "the cutter's own, at {p:?}"
            );
            continue;
        }
        let near =
            |a: u32, b: u32| (0..4).all(|k| (a >> (8 * k) & 255).abs_diff(b >> (8 * k) & 255) <= 1);
        assert!(
            near(color[v], red),
            "the body's paint at {p:?}: {:x}",
            color[v]
        );
        match kept(p) {
            Some(i) => assert_eq!(
                mask[v].to_bits(),
                ramp(b.positions[i]).to_bits(),
                "kept {p:?}"
            ),
            None => assert!((mask[v] - ramp(*p)).abs() < 1e-5, "cut {p:?}: {}", mask[v]),
        }
    }
}

/// UVs on the exact path: each output corner interpolated in its own source face (the body's
/// UVs are its points' x and z plus the face's index, so a corner taken from any other face
/// shows), the cutter's faces with none; and on an operand step 5 rebuilt, its UVs are dropped
/// with a warning while the body keeps its own.
#[test]
fn uvs_carry_from_each_corner_s_own_face_and_drop_on_a_rebuilt_operand() {
    let b = body();
    let mut columns: Vec<Vec<f32>> = vec![Vec::new(); 8];
    for (s, face) in b.faces.iter().enumerate() {
        for k in 0..4 {
            let p = b.positions[face[k] as usize];
            columns[2 * k].push(p[0] + s as f32);
            columns[2 * k + 1].push(p[2] + s as f32);
        }
    }
    let names = [
        "uv0.u", "uv0.v", "uv1.u", "uv1.v", "uv2.u", "uv2.v", "uv3.u", "uv3.v",
    ];
    let b = (names.iter().zip(columns)).fold(b, |m, (name, v)| {
        with(m, name, AttrDomain::Face, AttrValues::F32(v))
    });
    let open = (names.iter()).fold(opened(cutter()), |m, name| {
        let ones = vec![1.0; m.faces.len()];
        with(m, name, AttrDomain::Face, AttrValues::F32(ones))
    });
    for (cutter, rebuilt) in [(cutter(), false), (open, true)] {
        let mut s = cut(agent(), &b, &cutter);
        let done = s.ok("boolean.apply", json!({}));
        let warned = done
            .warnings
            .iter()
            .any(|w| w.contains("'cutter'") && w.contains("UVs"));
        assert_eq!(warned, rebuilt, "{:?}", done.warnings);
        let out = s.mesh("boolean");
        let groups = ids(&out, "group");
        let uv: Vec<&[f32]> = names.iter().map(|n| f32s(&out, n)).collect();
        for (f, face) in out.faces.iter().enumerate() {
            for k in 0..arity(*face) {
                let (u, v) = (uv[2 * k][f], uv[2 * k + 1][f]);
                if !(1..=96).contains(&groups[f]) {
                    assert_eq!((u, v), (0.0, 0.0), "rebuilt {rebuilt}: a cutter face {f}");
                    continue;
                }
                let s = (groups[f] - 1) as f32;
                let p = out.positions[face[k] as usize];
                let off = ((u - p[0] - s).abs(), (v - p[2] - s).abs());
                assert!(off.0 < 1e-3 && off.1 < 1e-3, "face {f} corner {k}: {off:?}");
            }
        }
    }
}

/// Each repair, and a base whose operation is not a union, add a sentence to the warnings.
#[test]
fn each_repair_and_an_ignored_base_operation_warn() {
    let mut s = cut(agent(), &body(), &inside_out(cutter()));
    s.ok(
        "part.set_operation",
        json!({ "part": "body", "operation": "intersect" }),
    );
    let done = s.ok("boolean.apply", json!({}));
    let warnings = done.warnings.join(" | ");
    assert!(
        warnings.contains("'body'") && warnings.contains("intersect"),
        "{warnings}"
    );
    assert!(
        warnings.contains("'cutter'") && warnings.contains("step 3"),
        "{warnings}"
    );
    let repairs = &done.result["groups"][0]["repairs"];
    assert_eq!(repairs[0]["part"], json!("cutter"), "{repairs}");
    assert_eq!(repairs[0]["step"], json!(3), "{repairs}");
}

/// Part `name` read from an OBJ file of `mesh`, which `part.create` would refuse when an edge
/// has three faces (a file is taken as it is).
fn import(s: &mut Session, name: &str, mesh: &PlainMesh) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("boolean-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let points = (mesh.positions.iter()).map(|p| format!("v {} {} {}\n", p[0], p[1], p[2]));
    let faces = mesh.faces.iter().map(|f| {
        let corners: Vec<String> = f[..arity(*f)].iter().map(|v| (v + 1).to_string()).collect();
        format!("f {}\n", corners.join(" "))
    });
    let text: String = points.chain(faces).collect();
    let path = dir.join("part.obj");
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{e}"));
    s.ok("file.import", json!({ "path": path, "name": name }));
    std::fs::remove_dir_all(&dir).ok();
}

/// The issues per part, each a count and the faces (capped): an open box, an edge three faces
/// share, an inside-out cube, two cubes crossing in one part, a cube with a cavity (not inside
/// out) and a clean cube, which reports nothing.
#[test]
fn check_reports_open_non_manifold_inside_out_and_crossing_parts() {
    let mut s = agent();
    let mut open = cube(2);
    open.faces.pop();
    let mut fin = cube(2);
    let [a, b, ..] = fin.faces[0];
    fin.positions.push([3.0, 3.0, 3.0]);
    fin.faces
        .push([a, b, fin.positions.len() as u32 - 1, NO_VERTEX]);
    import(&mut s, "fin", &fin);
    let parts = [
        ("open", open),
        ("turned", inside_out(cube(2))),
        // Off the lattice: a half step would meet only along edges and diagonals.
        (
            "crossed",
            joined(&cube(2), &placed(cube(2), 1.0, [0.3, 0.2, 0.1])),
        ),
        (
            "hollow",
            joined(&cube(2), &inside_out(placed(cube(2), 0.5, [0.0; 3]))),
        ),
        ("clean", cube(2)),
    ];
    for (name, mesh) in &parts {
        s.add_part(name, mesh);
    }
    for name in ["fin", "open", "turned", "crossed", "hollow", "clean"] {
        s.ok("part.set_operation", json!({ "part": name, "start": true }));
    }
    let report = s.query("boolean.check", json!({}));
    let issue = |part: &str, what: &str| -> (u64, usize) {
        let rows = report["parts"].as_array().cloned().unwrap_or_default();
        let row = rows
            .into_iter()
            .find(|r| r["part"] == json!(part))
            .unwrap_or_default();
        let count = row[what]["count"].as_u64().unwrap_or(u64::MAX);
        (count, row[what]["faces"].as_array().map_or(0, Vec::len))
    };
    assert_eq!(issue("open", "open_edges"), (4, 4), "{report}");
    assert_eq!(issue("fin", "non_manifold_edges").0, 1, "{report}");
    assert_eq!(issue("turned", "inside_out_shells"), (1, 24), "{report}");
    assert!(issue("crossed", "self_crossings").0 > 0, "{report}");
    assert_eq!(
        issue("hollow", "inside_out_shells").0,
        0,
        "a cavity: {report}"
    );
    let closed = |p: &str| {
        report["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| r["part"] == json!(p) && r["closed"] == json!(true))
    };
    assert!(
        !closed("open") && !closed("fin") && closed("turned"),
        "{report}"
    );
    for what in [
        "open_edges",
        "non_manifold_edges",
        "inside_out_shells",
        "self_crossings",
    ] {
        assert_eq!(issue("clean", what), (0, 0), "clean: {what}: {report}");
    }
    assert_eq!(report["coplanar"], json!([]), "each in a group of its own");
}

/// `a` and `b` as one mesh.
fn joined(a: &PlainMesh, b: &PlainMesh) -> PlainMesh {
    let n = a.positions.len() as u32;
    let mut out = a.clone();
    out.positions.extend_from_slice(&b.positions);
    let moved = b
        .faces
        .iter()
        .map(|f| f.map(|v| if v == NO_VERTEX { v } else { v + n }));
    out.faces.extend(moved);
    out
}

/// Two cubes face to face in one group have coplanar overlapping faces, named by both parts;
/// half the threshold apart they still do, twice the threshold apart they do not, and in
/// groups of their own they are not compared.
#[test]
fn check_reports_coplanar_faces_between_operands_of_one_group() {
    let threshold = 1e-4_f32;
    // The two cubes' bounding box together: x -1 to 3, so its diagonal is the square root of 24.
    let depth = threshold * 24_f32.sqrt();
    for (gap, start, found) in [
        (0.0, false, true),
        (depth / 2.0, false, true),
        (depth * 2.0, false, false),
        (0.0, true, false),
    ] {
        let mut s = agent();
        s.add_part("left", &cube(2));
        s.add_part("right", &placed(cube(2), 1.0, [2.0 + gap, 0.0, 0.0]));
        s.ok(
            "part.set_operation",
            json!({ "part": "right", "start": start }),
        );
        let report = s.query("boolean.check", json!({ "coplanar_threshold": threshold }));
        let pairs = report["coplanar"].as_array().cloned().unwrap_or_default();
        assert_eq!(
            !pairs.is_empty(),
            found,
            "gap {gap}, start {start}: {report}"
        );
        if found {
            assert_eq!(pairs[0]["parts"], json!(["left", "right"]), "{report}");
            assert_eq!(
                pairs[0]["count"],
                json!(4),
                "four quads meet four: {report}"
            );
            let sides = pairs[0]["faces"]
                .as_array()
                .map(|f| (f[0].as_array().map(Vec::len), f[1].as_array().map(Vec::len)));
            assert_eq!(sides, Some((Some(4), Some(4))), "{report}");
        }
    }
}

/// `part.remesh` folds its parts by their operations: a part marked Difference cuts the parts
/// above it instead of joining them.
#[test]
fn part_remesh_folds_parts_by_their_operations() {
    let mut s = primitives(agent());
    s.ok("part.remesh", json!({ "resolution": 48 }));
    let v = volume(&mut s, "remeshed");
    assert!((v - 4.4).abs() < 0.1, "the body less the cutter: {v}");
}

/// With every part a union, `part.remesh` gives the bits `remesh_parts_holes` gives, as before
/// it read operations: every point and every face, compared by their points.
#[test]
fn part_remesh_of_unions_gives_today_s_result_bit_for_bit() {
    use gs_geo::parts::merge;
    use gs_geo::voxel::{CellSize, CloseHoles, Settings, remesh_parts_holes};
    let (a, b) = (cube(3), placed(cube(2), 1.5, [1.6, 0.0, 0.0]));
    let mut s = agent();
    s.add_part("a", &a);
    s.add_part("b", &b);
    s.ok("part.remesh", json!({ "resolution": 40, "groups": false }));
    let got = s.mesh("remeshed");
    let merged = merge(&[&a, &b], |_, _| 0).unwrap_or_else(|e| panic!("{e}"));
    let settings = Settings {
        cell: CellSize::Resolution(40),
        project: false,
        blur: 0.0,
    };
    let parts = [a.faces.len(), b.faces.len()];
    let want = remesh_parts_holes(&merged, &parts, &settings, CloseHoles::Large)
        .unwrap_or_else(|e| panic!("{e}"))
        .mesh;
    assert_eq!(canonical(&got), canonical(&want));
}

/// A mesh as its points' bits, sorted, and its faces as the bits of their corners, each face
/// turned to start at its least corner, sorted: the same for any numbering of the same mesh.
fn canonical(mesh: &PlainMesh) -> (Vec<[u32; 3]>, Vec<Vec<[u32; 3]>>) {
    let bits = |v: u32| mesh.positions[v as usize].map(f32::to_bits);
    let mut points: Vec<[u32; 3]> = mesh.positions.iter().map(|p| p.map(f32::to_bits)).collect();
    points.sort_unstable();
    let mut faces: Vec<Vec<[u32; 3]>> = (mesh.faces.iter())
        .map(|f| {
            let mut c: Vec<[u32; 3]> = f[..arity(*f)].iter().map(|&v| bits(v)).collect();
            let least = (0..c.len()).min_by_key(|&k| c[k]).unwrap_or(0);
            c.rotate_left(least);
            c
        })
        .collect();
    faces.sort_unstable();
    (points, faces)
}

/// The harness reaches `boolean.apply`'s `closed` claim, and the three every edit keeps, with no
/// setup, at the gate's seed and cases: every part starts as a Union, so a plan with two fixtures
/// cuts them, and `part.set_operation` beside it marks some Difference or Intersect, or starts a
/// second group, first. `run_until_checked` fails unless each claim was checked.
#[test]
fn boolean_apply_keeps_its_claims_beside_part_set_operation() {
    let catalog = Session::new().engine.catalog().clone();
    let harness = Harness::new(&catalog).unwrap_or_else(|e| panic!("{e}"));
    (harness.only(&["boolean.apply", "part.set_operation"]))
        .run_until_checked()
        .unwrap_or_else(|e| panic!("{e}"));
}
