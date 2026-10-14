//! Observation commands against slow references (stats, bounds), primitives against geometry,
//! percentage lengths, part names, and the session summaries.

use gs_engine::EngineConfig;
use gs_protocol::Role;
use serde_json::{Value, json};

use gs_testkit::fixtures::tetra;
use gs_testkit::measure;
use gs_testkit::{Session, code, success, text};

use crate::support::{EngineSession, agent};

const TRI: u32 = u32::MAX;

fn assert_close(got: f64, want: f64, what: &str) {
    let scale = want.abs().max(1.0);
    assert!(
        (got - want).abs() <= 1e-9 * scale,
        "{what}: got {got}, want {want}"
    );
}

/// Builds parts of every kind of shape the engine can make.
fn assorted(s: &mut Session) {
    s.ok(
        "primitive.sphere",
        json!({ "name": "s1", "subdivisions": 1 }),
    );
    s.ok(
        "primitive.sphere",
        json!({ "name": "s5", "subdivisions": 5, "radius": 0.3, "center": [1, 2, 3] }),
    );
    s.ok(
        "mesh.jitter",
        json!({ "part": "s5", "amount": "3%", "seed": 9 }),
    );
    s.ok(
        "primitive.cube",
        json!({ "name": "c3", "subdivisions": 3, "size": 4 }),
    );
    s.ok(
        "mesh.translate",
        json!({ "part": "c3", "direction": [1, -1, 0.5], "distance": 2.5 }),
    );
    s.ok("part.create", json!({ "name": "t", "mesh": tetra() }));
    let open = json!({ "positions": [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [2, 0, 0.5]],
                       "faces": [[0, 1, 2, 3], [1, 4, 2, TRI]] });
    s.ok("part.create", json!({ "name": "open", "mesh": open }));
}

#[test]
fn stats_and_bounds_match_slow_references() {
    let mut s = agent();
    assorted(&mut s);
    let all = s.query("mesh.stats", json!({}))["parts"].clone();
    let names: Vec<String> = all
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| text(&p["part"]))
        .collect();
    assert_eq!(names, ["c3", "open", "s1", "s5", "t"], "every part, sorted");
    let mut everything = Vec::new();
    for (name, measured) in names.iter().zip(all.as_array().into_iter().flatten()) {
        let mesh = s.mesh(name);
        let want = measure::stats(&mesh);
        let count = |key: &str| measured[key].as_u64().unwrap_or(u64::MAX);
        let got = (
            count("vertices"),
            count("faces"),
            count("triangles"),
            count("quads"),
            count("edges"),
            count("boundary_edges"),
        );
        let expected = (
            want.vertices,
            want.faces,
            want.triangles,
            want.quads,
            want.edges,
            want.boundary_edges,
        );
        assert_eq!(got, expected, "{name} counts");
        assert_close(
            measured["area"].as_f64().unwrap_or_default(),
            want.area,
            &format!("{name} area"),
        );
        assert_close(
            measured["volume"].as_f64().unwrap_or_default(),
            want.volume,
            &format!("{name} volume"),
        );
        let attributes: Vec<&String> = mesh.attributes.keys().collect();
        assert_eq!(
            measured["attributes"],
            json!(attributes),
            "{name} attributes"
        );
        check_bounds(
            &s.query("mesh.bounds", json!({ "part": name })),
            &mesh.positions,
            name,
        );
        everything.extend(mesh.positions);
    }
    check_bounds(
        &s.query("mesh.bounds", json!({})),
        &everything,
        "the whole scene",
    );
    assert_eq!(
        s.query("mesh.stats", json!({ "part": "t" }))["parts"][0],
        all[4],
        "one part"
    );
}

fn check_bounds(measured: &Value, points: &[[f32; 3]], what: &str) {
    let [lo, hi] = measure::bounds(points).unwrap_or_default();
    let read = |key: &str| -> [f32; 3] {
        serde_json::from_value(measured[key].clone()).unwrap_or([f32::NAN; 3])
    };
    assert_eq!(
        read("min").map(f32::to_bits),
        lo.map(f32::to_bits),
        "{what} min"
    );
    assert_eq!(
        read("max").map(f32::to_bits),
        hi.map(f32::to_bits),
        "{what} max"
    );
    let center: [f32; 3] =
        std::array::from_fn(|i| f64::midpoint(f64::from(lo[i]), f64::from(hi[i])) as f32);
    let size: [f32; 3] = std::array::from_fn(|i| hi[i] - lo[i]);
    assert_eq!(
        read("center").map(f32::to_bits),
        center.map(f32::to_bits),
        "{what} center"
    );
    assert_eq!(
        read("size").map(f32::to_bits),
        size.map(f32::to_bits),
        "{what} size"
    );
    let diagonal = size
        .iter()
        .map(|&d| f64::from(d) * f64::from(d))
        .sum::<f64>()
        .sqrt();
    let got = measured["diagonal"].as_f64().unwrap_or_default();
    assert!(
        (got - diagonal).abs() <= 1e-6 * diagonal.max(1.0),
        "{what} diagonal {got} vs {diagonal}"
    );
}

#[test]
fn spheres_and_cubes_have_the_documented_shape() {
    let mut s = agent();
    for n in [1u64, 2, 3, 7] {
        let name = format!("s{n}");
        s.ok(
            "primitive.sphere",
            json!({ "name": name, "subdivisions": n, "radius": 2.5, "center": [1, -2, 3] }),
        );
        let stats = s.query("mesh.stats", json!({ "part": name }))["parts"][0].clone();
        let expected = json!([6 * n * n + 2, 6 * n * n, 6 * n * n, 12 * n * n, 0]);
        let got = json!([
            stats["vertices"],
            stats["faces"],
            stats["quads"],
            stats["edges"],
            stats["boundary_edges"]
        ]);
        assert_eq!(
            got, expected,
            "sphere {n}: 6n²+2 vertices, 6n² quads, closed"
        );
        for p in s.mesh(&name).positions {
            let (dx, dy, dz) = (p[0] - 1.0, p[1] + 2.0, p[2] - 3.0);
            let r = (dx * dx + dy * dy + dz * dz).sqrt();
            assert!(
                (r - 2.5).abs() < 1e-5,
                "sphere {n}: a vertex {r} from the center"
            );
        }
        let volume = stats["volume"].as_f64().unwrap_or_default();
        assert!(
            volume > 0.0 && volume < 4.0 / 3.0 * std::f64::consts::PI * 2.5 * 2.5 * 2.5,
            "sphere {n} volume {volume}"
        );
    }
    s.ok(
        "primitive.sphere",
        json!({ "name": "fine", "subdivisions": 32 }),
    );
    let volume = s.query("mesh.stats", json!({ "part": "fine" }))["parts"][0]["volume"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (volume / (4.0 / 3.0 * std::f64::consts::PI) - 1.0).abs() < 0.01,
        "a fine sphere is round: {volume}"
    );
    for n in [1u64, 3] {
        let name = format!("c{n}");
        s.ok(
            "primitive.cube",
            json!({ "name": name, "subdivisions": n, "size": 3, "center": [1, 2, -1] }),
        );
        let stats = s.query("mesh.stats", json!({ "part": name }))["parts"][0].clone();
        assert_eq!(stats["vertices"], json!(6 * n * n + 2), "cube {n} vertices");
        assert_close(
            stats["volume"].as_f64().unwrap_or_default(),
            27.0,
            "cube volume",
        );
        assert_close(
            stats["area"].as_f64().unwrap_or_default(),
            54.0,
            "cube area",
        );
        let bounds = s.query("mesh.bounds", json!({ "part": name }));
        assert_eq!(
            (bounds["min"].clone(), bounds["max"].clone()),
            (json!([-0.5, 0.5, -2.5]), json!([2.5, 3.5, 0.5])),
            "cube {n}"
        );
    }
}

#[test]
fn lengths_given_as_percentages_follow_the_part_size() {
    let mut s = agent();
    s.ok("primitive.cube", json!({ "size": 2 }));
    let diagonal = s.query("mesh.bounds", json!({}))["diagonal"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (diagonal - 2.0 * 3f64.sqrt()).abs() < 1e-6,
        "cube diagonal {diagonal}"
    );
    let moved = s.query(
        "mesh.translate",
        json!({ "direction": [0, 0, 2], "distance": "50%" }),
    );
    let dz = moved["offset"][2].as_f64().unwrap_or_default();
    assert!(
        (dz - diagonal / 2.0).abs() < 1e-6,
        "50% of the diagonal, along unit z: {dz}"
    );
    assert_eq!(
        (moved["offset"][0].clone(), moved["offset"][1].clone()),
        (json!(0.0), json!(0.0)),
        "only z"
    );
    let before = s.mesh("cube");
    let moved = s.query(
        "mesh.translate",
        json!({ "direction": [3, 4, 0], "distance": 10 }),
    );
    assert_eq!(
        moved["offset"],
        json!([6.0, 8.0, 0.0]),
        "plain units along the normalized direction"
    );
    let shifted: Vec<[f32; 3]> = before
        .positions
        .iter()
        .map(|p| [p[0] + 6.0, p[1] + 8.0, p[2]])
        .collect();
    assert_eq!(
        s.mesh("cube").positions,
        shifted,
        "every vertex moved by the offset"
    );
    let before = s.mesh("cube");
    let jittered = s.query("mesh.jitter", json!({ "amount": "1%" }));
    let amount = jittered["amount"].as_f64().unwrap_or_default();
    assert!(
        (amount - diagonal / 100.0).abs() < 1e-6,
        "1% of the diagonal: {amount}"
    );
    let after = s.mesh("cube");
    let moves = before
        .positions
        .iter()
        .flatten()
        .zip(after.positions.iter().flatten());
    // NaN counts as infinitely far, so a NaN offset fails the check instead of vanishing.
    let largest = moves
        .map(|(a, b)| f64::from((b - a).abs()))
        .map(|d| if d.is_nan() { f64::INFINITY } else { d })
        .fold(0.0, f64::max);
    assert!(
        largest > amount / 4.0 && largest <= amount * 1.0001,
        "offsets up to the amount: {largest}"
    );
}

#[test]
fn new_parts_get_free_names_and_parts_are_pinned() {
    let mut s = agent();
    let err = s.err(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 1 }),
    );
    assert_eq!(code(err.code), "E_NOT_FOUND", "no part to move");
    assert!(err.hint.is_some(), "says how to make one");
    let named: Vec<String> = (0..3)
        .map(|_| text(&s.query("primitive.sphere", json!({}))["part"]))
        .collect();
    assert_eq!(named, ["sphere", "sphere-2", "sphere-3"], "free names");
    assert_eq!(
        text(&s.query("primitive.cube", json!({ "name": "head" }))["part"]),
        "head",
        "given names"
    );
    let err = s.err("primitive.sphere", json!({ "name": "head" }));
    assert_eq!(code(err.code), "E_PRECONDITION", "taken names are refused");
    assert!(err.hint.is_some(), "with a fix");
    let active = s.query("part.get", json!({}));
    assert_eq!(
        active["positions"].as_array().map(Vec::len),
        Some(8),
        "several parts: the active one, the cube added last (decision 0033)"
    );
    let err = s.err("part.get", json!({ "part": "sphre" }));
    assert_eq!(
        (code(err.code), err.suggestions[0].as_str()),
        ("E_NOT_FOUND".into(), "sphere"),
        "did you mean"
    );
    let logged = s.ok("mesh.jitter", json!({ "part": "head", "amount": 0.1 }));
    assert!(
        logged.result["seed"].is_u64(),
        "random commands report the seed they used"
    );
}

#[test]
fn empty_scenes_have_no_bounds() {
    let mut s = agent();
    assert_eq!(
        code(s.err("mesh.bounds", json!({})).code),
        "E_PRECONDITION",
        "nothing to measure"
    );
    s.ok(
        "part.create",
        json!({ "name": "nothing", "mesh": { "positions": [], "faces": [] } }),
    );
    let err = s.err("mesh.bounds", json!({ "part": "nothing" }));
    assert!(
        err.hint.is_some() && code(err.code) == "E_PRECONDITION",
        "an empty part has no bounds"
    );
}

#[test]
fn describe_and_info_report_the_session() {
    let log = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("info-{}.jsonl", std::process::id()));
    let config = EngineConfig {
        session: "studio".into(),
        seed: 31,
        history_budget_bytes: 5000,
        log_path: Some(log.clone()),
        verify: false,
        autosave: None,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    assorted(&mut s);
    s.ok("history.checkpoint", json!({ "name": "keep" }));
    s.ok("history.undo", json!({}));
    success(
        s.as_human("session.control", json!({ "gesture": true })),
        "gesture",
    );
    let text_out = text(&s.query("scene.describe", json!({}))["text"]);
    let stats = s.query("mesh.stats", json!({}))["parts"].clone();
    for part in stats.as_array().into_iter().flatten() {
        let line = format!(
            "{}: {} vertices, {} faces",
            text(&part["part"]),
            part["vertices"],
            part["faces"]
        );
        assert!(
            text_out.contains(&line),
            "describe lists {line}:\n{text_out}"
        );
    }
    let history = s.history();
    for fact in [
        "studio",
        "seed 31",
        "keep",
        "gesture: true",
        "paused: false",
    ] {
        assert!(
            text_out.contains(fact),
            "describe mentions {fact}:\n{text_out}"
        );
    }
    for key in ["undoable", "redoable", "bytes", "budget_bytes"] {
        assert!(
            text_out.contains(&history[key].to_string()),
            "describe mentions {key}:\n{text_out}"
        );
    }
    let info = s.query("session.info", json!({}));
    assert_eq!(info["session"], json!("studio"), "name");
    assert_eq!(info["seed"], json!(31), "seed");
    assert_eq!(info["history_budget_bytes"], json!(5000), "budget");
    assert_eq!(info["log_path"], json!(log.display().to_string()), "log");
    let state = s.query("session.state", json!({}));
    assert_eq!(
        (
            state["session.paused"].clone(),
            state["session.gesture"].clone()
        ),
        (json!(false), json!(true)),
        "switches"
    );
    let rev = s.call("mesh.stats", json!({})).rev;
    assert_eq!(info["rev"], json!(rev), "revision");
    std::fs::remove_file(log).ok();
}

/// The defaults the schemas document are the ones commands use, and the default primitives
/// come out the same size, so a new part matches the ones already there.
#[test]
fn documented_defaults_are_the_ones_used() {
    let catalog = agent().engine.catalog().clone();
    let default = |command: &str, arg: &str| -> Value {
        let info = catalog.commands.iter().find(|c| c.name == command);
        info.map(|c| c.args_schema["properties"][arg]["default"].clone())
            .unwrap_or_default()
    };
    let mut s = agent();
    // A sphere's counts depend on its form, so they have no schema default (decision 0047);
    // the default sphere is the cube form with 8 subdivisions.
    let made = s.query("primitive.sphere", json!({}));
    assert_eq!(
        made["vertices"],
        json!(6 * 8 * 8 + 2),
        "default subdivisions 8"
    );
    let sphere = s.query("mesh.bounds", json!({ "part": "sphere" }));
    assert_eq!(
        sphere["max"][1],
        default("primitive.sphere", "radius"),
        "default radius"
    );
    s.ok("primitive.cube", json!({}));
    let cube = s.query("mesh.bounds", json!({ "part": "cube" }));
    let size = default("primitive.cube", "size");
    assert_eq!(cube["size"], json!([size, size, size]), "default size");
    assert_eq!(
        (cube["min"].clone(), cube["max"].clone()),
        (sphere["min"].clone(), sphere["max"].clone()),
        "same box"
    );
}

/// m3: a direction is measured in f64, so huge and tiny directions are fine and only a zero
/// one is refused.
#[test]
fn directions_of_any_size_point_the_same_way() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    for direction in [
        json!([1e20, 0, 0]),
        json!([1e-30, 0, 0]),
        json!([3e-45, 0, 0]),
    ] {
        let moved = s.query(
            "mesh.translate",
            json!({ "direction": direction, "distance": 1 }),
        );
        assert_eq!(moved["offset"], json!([1.0, 0.0, 0.0]), "along {direction}");
    }
    let err = s.err(
        "mesh.translate",
        json!({ "direction": [0, 0, 0], "distance": 1 }),
    );
    assert_eq!(
        err.path.as_deref(),
        Some("/direction"),
        "a zero direction is refused"
    );
}

/// Review 0005, m5: `light.set` takes a direction of any length but 0, as `mesh.translate`
/// does above, and keeps it at length 1, the direction the renderer lights from.
#[test]
fn light_directions_of_any_size_point_the_same_way() {
    let mut s = agent();
    let third = 1.0 / 3.0_f64.sqrt();
    for (direction, want) in [
        (json!([1e-30, 0, 0]), [1.0, 0.0, 0.0]),
        (json!([1e-45, 0, 0]), [1.0, 0.0, 0.0]),
        (json!([3e38, 3e38, 3e38]), [third; 3]),
        (json!([0, 3, -4]), [0.0, 0.6, -0.8]),
    ] {
        s.ok("light.set", json!({ "light": 2, "direction": direction }));
        let state = s.query("session.state", json!({}));
        let got = &state["light.2"]["direction"];
        for (k, want) in want.into_iter().enumerate() {
            let got = got[k].as_f64().unwrap_or(f64::NAN);
            assert!((got - want).abs() < 1e-7, "{direction}: {got} for {want}");
        }
    }
    let err = s.err("light.set", json!({ "light": 2, "direction": [0, 0, 0] }));
    assert_eq!(
        err.path.as_deref(),
        Some("/direction"),
        "a zero direction is refused"
    );
}
