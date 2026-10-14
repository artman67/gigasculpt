//! Voxel mode (decision 0002, amended 2026-10-12) through the engine: `mesh.voxel_mode` turns a
//! part's switch on, remeshing it once, or off, leaving the mesh; `mesh.voxel_update`, the
//! window's Ctrl drag on empty canvas, remeshes a part in Voxel mode at the cell size it keeps
//! and is refused for one that is not; `mesh.voxel_remesh`'s `close_holes` 0 leaves an open part
//! open where 1 closes it; the switch and the update take it too, and a call that leaves it out
//! takes the session's hole closing (`mesh.voxel_settings`, decision 0052 as amended), written
//! into its logged call; and the switch is saved with the scene.

use std::collections::BTreeMap;

use gs_protocol::{ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::socket::scratch;
use crate::support::log_records;

/// The active part's switch, as `session.state` reports it.
fn switch(s: &mut Session) -> Value {
    s.query("session.state", json!({}))["parts.voxel_mode"].clone()
}

/// Each edge, either way round, and how many faces use it.
fn edge_uses(mesh: &PlainMesh) -> BTreeMap<(u32, u32), u32> {
    let mut uses = BTreeMap::new();
    for f in &mesh.faces {
        let c: Vec<u32> = f.iter().copied().filter(|&v| v != NO_VERTEX).collect();
        for k in 0..c.len() {
            let (a, b) = (c[k], c[(k + 1) % c.len()]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    uses
}

/// The edges only one face uses: the mesh's open rims.
pub(crate) fn open_edges(mesh: &PlainMesh) -> Vec<(u32, u32)> {
    let uses = edge_uses(mesh).into_iter();
    uses.filter(|(_, n)| *n == 1).map(|(e, _)| e).collect()
}

/// The length of every edge.
fn edge_lengths(mesh: &PlainMesh) -> Vec<f64> {
    let at = |v: u32| mesh.positions[v as usize].map(f64::from);
    let edges = edge_uses(mesh).into_keys();
    edges
        .map(|(a, b)| {
            let (p, q) = (at(a), at(b));
            (0..3)
                .map(|i| (p[i] - q[i]) * (p[i] - q[i]))
                .sum::<f64>()
                .sqrt()
        })
        .collect()
}

/// The cell size a remesh's reply reports.
fn cell_of(reply: &Value) -> f64 {
    reply["voxel_size"].as_f64().unwrap_or(0.0)
}

#[test]
fn with_the_switch_on_the_update_rebuilds_even_quads_within_one_cell_of_a_stretched_sphere() {
    let mut s = Session::new();
    assert_eq!(switch(&mut s), Value::Null, "no part, no switch");
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    s.ok("mesh.voxel_remesh", json!({ "resolution": 32 }));
    assert_eq!(switch(&mut s), json!(false), "the button leaves it off");
    let on = s.ok("mesh.voxel_mode", json!({ "on": true }));
    assert_eq!(switch(&mut s), json!(true), "on");
    let cell = cell_of(&on.result["remesh"]);
    assert!(
        (cell - 2.0 / 32.0).abs() <= 1e-6,
        "turned on, it remeshed at the cell size the part keeps: {cell}"
    );
    // Pulled out to two and a half times its length along x, as sculpting stretches it.
    s.ok(
        "transform.apply",
        json!({ "frame": { "origin": [0, 0, 0], "axes": [0, 0, 0, 1] },
                "op": { "scale": [2.5, 1.0, 1.0] } }),
    );
    let stretched = s.mesh("sphere");
    // Surface Nets joins the vertices of two cells that share a side, so no edge it makes is
    // longer than the diagonal of two cells side by side.
    let most = 6.0_f64.sqrt() * cell * 1.0001;
    let longest = |m: &PlainMesh| edge_lengths(m).into_iter().fold(0.0, f64::max);
    assert!(
        longest(&stretched) > most,
        "stretched, some quads are long: {} cells",
        longest(&stretched) / cell
    );
    let done = s.ok("mesh.voxel_update", json!({}));
    assert!(
        (cell_of(&done.result) - cell).abs() <= 1e-6,
        "at the kept cell size: {}",
        cell_of(&done.result)
    );
    let mesh = s.mesh("sphere");
    assert!(mesh.faces.iter().all(|f| f[3] != NO_VERTEX), "all quads");
    assert!(open_edges(&mesh).is_empty(), "closed");
    let lengths = edge_lengths(&mesh);
    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
    assert!(
        longest(&mesh) <= most,
        "even again: {} cells at most",
        longest(&mesh) / cell
    );
    assert!(
        (0.6..=1.4).contains(&(mean / cell)),
        "edges about a cell long: {} cells",
        mean / cell
    );
    let off = gs_testkit::measure::hausdorff(&stretched, &mesh);
    assert!(
        off <= cell,
        "within one cell of the shape: {} cells",
        off / cell
    );
}

#[test]
fn with_the_switch_off_the_update_is_refused_and_off_then_on_remeshes_again() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    // Remeshed by the button: it keeps a cell size, but the switch is off.
    s.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    let refused = |s: &mut Session, what: &str| {
        let hash = s.hash();
        let e = s.err("mesh.voxel_update", json!({}));
        assert_eq!(e.code, ErrorCode::Precondition, "{what}: {e:?}");
        assert!(
            e.message.contains("'sphere' is not in Voxel mode"),
            "{what}: {}",
            e.message
        );
        let hint = e.hint.unwrap_or_default();
        assert!(hint.contains("mesh.voxel_mode"), "{what}: {hint}");
        assert_eq!(s.hash(), hash, "{what}: nothing changed");
    };
    refused(&mut s, "never on");
    let on = s.ok("mesh.voxel_mode", json!({ "on": true }));
    let size = on.result["remesh"]["voxel_size"].clone();
    let effect = on
        .effect
        .unwrap_or_else(|| panic!("an edit reports its effect"));
    assert!(effect.topology_changed, "turning it on remeshes");
    s.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 0.25 }),
    );
    let mesh = s.mesh("sphere");
    let off = s.ok("mesh.voxel_mode", json!({ "on": false }));
    assert_eq!(off.result["remesh"], Value::Null, "off remeshes nothing");
    let left = s.mesh("sphere");
    assert!(
        left.positions == mesh.positions && left.faces == mesh.faces,
        "off leaves the mesh as it is"
    );
    assert_eq!(switch(&mut s), json!(false), "off");
    refused(&mut s, "turned off");
    // Undo turns it back on; redo, off again.
    s.ok("history.undo", json!({}));
    assert_eq!(switch(&mut s), json!(true), "undone");
    s.ok("history.redo", json!({}));
    assert_eq!(switch(&mut s), json!(false), "redone");
    // Off and on again reruns the remesh, at the same cell size.
    let again = s.ok("mesh.voxel_mode", json!({ "on": true }));
    assert_eq!(again.result["remesh"]["voxel_size"], size, "the same size");
    let effect = again
        .effect
        .unwrap_or_else(|| panic!("an edit reports its effect"));
    assert!(effect.topology_changed, "a new surface");
    // Asking for the state it is in changes nothing, and says so.
    let hash = s.hash();
    let same = s.ok("mesh.voxel_mode", json!({ "on": true }));
    let said = (same.effect.as_ref()).and_then(|e| e.noop_reason.clone());
    assert!(said.is_some_and(|r| r.contains("already on")), "{same:?}");
    assert_eq!(s.hash(), hash, "already on");
}

#[test]
fn turning_the_switch_on_refuses_a_part_with_detail_levels_as_a_remesh_does() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("level.subdivide", json!({}));
    let hash = s.hash();
    let want = s.err("mesh.voxel_remesh", json!({}));
    let e = s.err("mesh.voxel_mode", json!({ "on": true }));
    assert_eq!(
        (e.code, &e.message, &e.hint),
        (want.code, &want.message, &want.hint),
        "the remesh's own refusal"
    );
    assert_eq!(switch(&mut s), json!(false), "still off");
    assert_eq!(s.hash(), hash, "nothing changed");
}

/// An open tube along y: `segments` quads around by 8 along, radius 0.5 and length 2, facing
/// out, its two ends open.
pub(crate) fn open_tube(segments: u32) -> Value {
    let positions: Vec<[f32; 3]> = (0..9)
        .flat_map(|j| {
            (0..segments).map(move |k| {
                let a = std::f32::consts::TAU * k as f32 / segments as f32;
                [
                    0.5 * libm::cosf(a),
                    j as f32 / 4.0 - 1.0,
                    0.5 * libm::sinf(a),
                ]
            })
        })
        .collect();
    // Face n joins ring n / segments to the next, around from point n % segments.
    let faces: Vec<[u32; 4]> = (0..8 * segments)
        .map(|n| {
            let (ring, next) = (n - n % segments, (n + 1) % segments);
            [n, n + segments, ring + segments + next, ring + next]
        })
        .collect();
    json!({ "positions": positions, "faces": faces })
}

#[test]
fn hole_closing_0_leaves_the_ends_of_an_open_cylinder_open_where_1_closes_them() {
    let remeshed = |close: Value| {
        let mut s = Session::new();
        s.ok(
            "part.create",
            json!({ "name": "tube", "mesh": open_tube(32) }),
        );
        let mut args = json!({ "resolution": 32 });
        if !close.is_null() {
            args["close_holes"] = close;
        }
        let done = s.ok("mesh.voxel_remesh", args);
        (cell_of(&done.result), s.mesh("tube"))
    };
    let (cell, closed) = remeshed(json!(1));
    assert!(open_edges(&closed).is_empty(), "1 closes both ends");
    let radius = |p: &[f32; 3]| f64::from(libm::hypotf(p[0], p[2]));
    assert!(
        closed
            .positions
            .iter()
            .any(|p| radius(p) < 0.5 - 2.0 * cell),
        "across the ends"
    );
    let (_, left_out) = remeshed(Value::Null);
    assert_eq!(left_out.faces, closed.faces, "left out, holes close");
    let (_, open) = remeshed(json!(0));
    let rims = open_edges(&open);
    assert!(!rims.is_empty(), "0 leaves them open");
    for (a, b) in rims {
        for v in [a, b] {
            let y = f64::from(open.positions[v as usize][1]).abs();
            assert!(y >= 1.0 - 2.0 * cell, "a rim at an end: y {y}");
        }
    }
    for p in &open.positions {
        let off = (radius(p) - 0.5).abs();
        assert!(off <= 1.5 * cell, "on the tube, not across it: {p:?}");
    }
    assert!(open.faces.iter().all(|f| f[3] != NO_VERTEX), "all quads");
    // 2 to 4 are later work.
    let mut s = Session::new();
    s.ok(
        "part.create",
        json!({ "name": "tube", "mesh": open_tube(32) }),
    );
    let e = s.err("mesh.voxel_remesh", json!({ "close_holes": 2 }));
    assert_eq!(e.code, ErrorCode::ArgsInvalid, "{e:?}");
    assert_eq!(e.path.as_deref(), Some("/close_holes"), "{e:?}");
}

/// Each voxel call in the log at `log`, with the hole closing it holds (null for none).
fn logged_closing(log: &std::path::Path) -> Vec<(String, Value)> {
    let (_, records) = log_records(log);
    (records.into_iter())
        .filter(|r| r.command.starts_with("mesh.voxel_") && r.command != "mesh.voxel_settings")
        .map(|r| (r.command, r.args["close_holes"].clone()))
        .collect()
}

#[test]
fn the_switch_and_the_update_take_hole_closing_and_the_log_holds_the_value_each_used() {
    let dir = scratch("voxel-closing");
    let log = dir.join("closing.jsonl");
    let mut s = seeded(0, Some(log.clone()));
    s.ok(
        "part.create",
        json!({ "name": "tube", "mesh": open_tube(32) }),
    );
    let open = |s: &mut Session| !open_edges(&s.mesh("tube")).is_empty();
    s.ok(
        "mesh.voxel_remesh",
        json!({ "resolution": 24, "close_holes": 0 }),
    );
    s.ok("mesh.voxel_mode", json!({ "on": true, "close_holes": 0 }));
    assert!(open(&mut s), "the switch with 0 leaves the ends open");
    s.ok("mesh.voxel_update", json!({ "close_holes": 1 }));
    assert!(!open(&mut s), "the update with 1 closes them");
    // Left out, the session's hole closing: 1 at first, then 0.
    s.ok("mesh.voxel_update", json!({}));
    s.ok("mesh.voxel_settings", json!({ "close_holes": 0 }));
    s.ok("mesh.voxel_update", json!({}));
    s.ok("mesh.voxel_mode", json!({ "on": false }));
    s.ok("mesh.voxel_mode", json!({ "on": true }));
    let e = s.err("mesh.voxel_update", json!({ "close_holes": 2 }));
    assert_eq!(e.path.as_deref(), Some("/close_holes"), "{e:?}");
    let call = |command: &str, close: Value| (command.to_string(), close);
    assert_eq!(
        logged_closing(&log),
        [
            call("mesh.voxel_remesh", json!(0)),
            call("mesh.voxel_mode", json!(0)),
            call("mesh.voxel_update", json!(1)),
            call("mesh.voxel_update", json!(1)),
            call("mesh.voxel_update", json!(0)),
            // Turned off, it remeshes nothing and needs none.
            call("mesh.voxel_mode", Value::Null),
            call("mesh.voxel_mode", json!(0)),
        ],
        "the log holds the hole closing each call used"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn with_hole_closing_0_a_remesh_leaving_it_out_stays_open_logs_0_and_replays_the_same_at_1() {
    let dir = scratch("voxel-preference");
    let log = dir.join("preference.jsonl");
    let mut s = seeded(0, Some(log.clone()));
    let closing =
        |s: &mut Session| s.query("session.state", json!({}))["voxel.close_holes"].clone();
    assert_eq!(closing(&mut s), json!(1), "large triangles at first");
    let e = s.err("mesh.voxel_settings", json!({ "close_holes": 2 }));
    assert_eq!(e.code, ErrorCode::ArgsInvalid, "{e:?}");
    assert_eq!(e.path.as_deref(), Some("/close_holes"), "{e:?}");
    s.ok("mesh.voxel_settings", json!({ "close_holes": 0 }));
    assert_eq!(closing(&mut s), json!(0), "none");
    s.ok(
        "part.create",
        json!({ "name": "tube", "mesh": open_tube(32) }),
    );
    s.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    assert!(
        !open_edges(&s.mesh("tube")).is_empty(),
        "the ends stay open"
    );
    assert_eq!(
        logged_closing(&log),
        [("mesh.voxel_remesh".to_string(), json!(0))],
        "the logged call says 0"
    );
    // Replayed where the hole closing is 1, without the call that set it to 0.
    let mut replay = seeded(0, None);
    assert_eq!(closing(&mut replay), json!(1));
    let (_, records) = log_records(&log);
    for r in records
        .iter()
        .filter(|r| r.command != "mesh.voxel_settings")
    {
        let request = r.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), &format!("{r:?}"));
    }
    assert_eq!(replay.hash(), s.hash(), "the same scene");
    assert!(!open_edges(&replay.mesh("tube")).is_empty(), "still open");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_switch_is_saved_with_the_scene_and_a_scene_saved_before_opens_with_it_off() {
    let dir = scratch("voxel-mode");
    let path = dir.join("on.gss").display().to_string();
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("mesh.voxel_remesh", json!({ "resolution": 16 }));
    s.ok("mesh.voxel_mode", json!({ "on": true }));
    let on = s.hash();
    s.ok("mesh.voxel_mode", json!({ "on": false }));
    assert_ne!(
        s.hash(),
        on,
        "the scene hash counts the switch while it is on"
    );
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), on, "undone, on again");
    s.ok("file.save", json!({ "path": path }));
    let mut opened = Session::new();
    opened.ok("file.open", json!({ "path": path }));
    assert_eq!(switch(&mut opened), json!(true), "saved on, opened on");
    assert_eq!(opened.hash(), s.hash(), "the same scene");
    opened.ok("mesh.voxel_update", json!({}));
    // Written by format 1 on 2026-10-10: a sphere that keeps a cell size from a voxel remesh.
    let golden = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/scene/golden/v1.gss");
    let mut old = Session::new();
    old.ok("file.open", json!({ "path": golden }));
    assert_eq!(
        switch(&mut old),
        json!(false),
        "an old scene opens with it off"
    );
    let e = old.err("mesh.voxel_update", json!({ "part": "sphere" }));
    assert_eq!(e.code, ErrorCode::Precondition, "{e:?}");
    std::fs::remove_dir_all(dir).ok();
}
