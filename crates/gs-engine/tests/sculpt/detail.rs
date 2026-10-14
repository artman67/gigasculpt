//! Live Detail through `brush.stroke` (decision 0021): a stroke that carries `live_detail` adds
//! and removes triangles under the brush and keeps the part a valid closed surface, says what
//! it did, or why it did not refine; undo and redo give back each version bit for bit, a part
//! numbered as it was given included; and pinning fills every setting into the logged call, so
//! the log replays in a fresh session to the same hashes at any thread count. `mesh.reorder`
//! gives the same surface a new numbering, as one undo step.

use gs_engine::EngineConfig;
use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh, Role, ScriptLine};
use gs_testkit::{Expect, Session, success, text, validate};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke, stroke_lines, vertex_bits, with};

/// A stroke across the front of the sphere.
fn across() -> Vec<[f32; 3]> {
    line([-0.4, 0.1, 0.92], [0.4, 0.2, 0.9], 12)
}

/// `brush.stroke` arguments for `brush` across the front, refining with `live_detail`.
fn refining(brush: &str, live_detail: Value, extra: &Value) -> Value {
    let mut ask = json!({});
    ask["live_detail"] = live_detail;
    stroke(brush, &across(), &with(ask, extra))
}

/// The part as a closed manifold sphere, or a panic listing what is wrong.
fn valid_sphere(s: &mut Session, part: &str) -> usize {
    let mesh = s.mesh(part);
    if let Err(defects) = validate(&mesh, Expect::closed(2)) {
        panic!("{part} is not a valid sphere: {defects:?}");
    }
    mesh.faces.len()
}

#[test]
fn a_refining_stroke_adds_triangles_and_says_what_it_did() {
    let mut s = sphere_session(None);
    let faces = valid_sphere(&mut s, "sphere");
    let call = s.ok(
        "brush.stroke",
        refining("Draw", json!({ "size": 0.1 }), &json!({})),
    );
    let effect = call.effect.unwrap_or_else(|| panic!("no effect"));
    assert!(effect.topology_changed, "{effect:?}");
    let done = call.result;
    let ld = &done["live_detail"];
    assert_eq!(ld["refined"], json!(true), "{done}");
    assert!(ld.get("why_not").is_none(), "{done}");
    let count = |k: &str| ld[k].as_u64().unwrap_or_default();
    assert!(count("splits") > 0 && count("points_added") > 0, "{done}");
    let refined = valid_sphere(&mut s, "sphere");
    assert!(refined > faces, "{faces} faces became {refined}");
    // Without `live_detail` the topology never changes, and nothing is reported.
    let plain = s.query("brush.stroke", stroke("Draw", &across(), &json!({})));
    assert!(plain.get("live_detail").is_none(), "{plain}");
    assert_eq!(valid_sphere(&mut s, "sphere"), refined);
}

/// A refining stroke over a long box of quads returns with the box valid: its equally long
/// sides once kept the split step going forever (review 0006, C1).
#[test]
fn refining_a_long_box_ends() {
    let mut s = Session::new();
    let mut mesh = gs_testkit::fixtures::cube(4);
    for p in &mut mesh.positions {
        p[0] *= 2.0;
    }
    s.ok("part.create", json!({ "name": "box", "mesh": mesh }));
    let mut args = stroke(
        "Draw",
        &line([-1.0, -0.3, 1.0], [1.0, 0.4, 1.0], 12),
        &json!({ "live_detail": { "size": 0.05 } }),
    );
    args["part"] = json!("box");
    let done = s.ok("brush.stroke", args).result;
    assert_eq!(done["live_detail"]["refined"], json!(true), "{done}");
    if let Err(defects) = validate(&s.mesh("box"), Expect::closed(2)) {
        panic!("the box is not valid: {defects:?}");
    }
}

/// A stroke inside one big face refines it, since the candidates are the faces the brush
/// touches, not only the edges passing inside it; a stroke that changes nothing says so
/// (review 0006, m1).
#[test]
fn a_stroke_inside_one_big_face_refines_it() {
    let mut s = Session::new();
    s.ok("primitive.cube", json!({}));
    let draw = |ld: Value| {
        let mut args = stroke(
            "Draw",
            &line([-0.6, -0.3, 1.0], [0.6, 0.4, 1.0], 12),
            &json!({ "live_detail": ld }),
        );
        args["part"] = json!("cube");
        args
    };
    let mut faces = s.mesh("cube").faces.len();
    for size in [0.2, 0.05] {
        // Plain midpoints keep the face flat, so the second stroke's samples still lie on it;
        // the Hermite midpoint, following the corner normals, would lift its middle by a third
        // of its half-width.
        let ld = json!({ "size": size, "smooth_split": false });
        let done = s.ok("brush.stroke", draw(ld)).result;
        let ld = &done["live_detail"];
        assert_eq!(ld["refined"], json!(true), "size {size}: {done}");
        assert!(ld["splits"].as_u64() > Some(0), "size {size}: {done}");
        let mesh = s.mesh("cube");
        assert!(
            mesh.faces.len() > faces,
            "size {size}: {} faces",
            mesh.faces.len()
        );
        faces = mesh.faces.len();
        if let Err(defects) = validate(&mesh, Expect::closed(2)) {
            panic!("size {size}: the cube is not valid: {defects:?}");
        }
    }
    let done = s.ok(
        "brush.stroke",
        draw(json!({ "size": 50.0, "combined": false })),
    );
    let ld = &done.result["live_detail"];
    assert_eq!(ld["refined"], json!(false), "{}", done.result);
    assert!(text(&ld["why_not"]).contains("smaller"), "{}", done.result);
}

/// Lengths in percent keep working after refining strokes: node upkeep leaves a node empty,
/// and its empty box once made the part's size infinite (review 0006, M1).
#[test]
fn percent_lengths_stay_finite_after_refining() {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    for (k, size) in [0.05f32, 0.03, 0.02, 0.05].iter().enumerate() {
        let y = 0.1 * k as f32 - 0.2;
        let mut args = stroke(
            "Draw",
            &line([-0.3, y, 0.95], [0.3, y + 0.05, 0.95], 12),
            &json!({ "live_detail": { "size": size } }),
        );
        args["part"] = json!("sphere");
        s.ok("brush.stroke", args);
    }
    s.ok(
        "mesh.translate",
        json!({ "part": "sphere", "distance": "1%", "direction": [1, 0, 0] }),
    );
    s.ok(
        "brush.stroke",
        stroke(
            "Draw",
            &line([-0.3, 0.2, 0.95], [0.3, 0.25, 0.95], 12),
            &json!({}),
        ),
    );
}

#[test]
fn strokes_that_cannot_refine_sculpt_as_usual_and_say_why() {
    let mut s = sphere_session(None);
    let faces = valid_sphere(&mut s, "sphere");
    let ask = json!({});
    for (brush, extra, why) in [
        ("Move", json!({}), "drags"),
        ("Mask Pen", json!({}), "paints the mask"),
        (
            "Draw",
            json!({ "settings": { "live_detail": { "enable": false } } }),
            "opts out",
        ),
    ] {
        let done = s.query("brush.stroke", refining(brush, ask.clone(), &extra));
        let ld = &done["live_detail"];
        assert_eq!(ld["refined"], json!(false), "{brush}: {done}");
        assert!(text(&ld["why_not"]).contains(why), "{brush}: {done}");
        assert_eq!(
            valid_sphere(&mut s, "sphere"),
            faces,
            "{brush} kept the topology"
        );
    }
    // A part with UV columns: refining would tear them.
    let mut mesh = s.mesh("sphere");
    let uv = |k: usize| {
        (0..mesh.faces.len())
            .map(|f| (f + k) as f32 * 0.001)
            .collect()
    };
    let columns = [("uv0.u", uv(0)), ("uv0.v", uv(1))];
    for (name, values) in columns {
        let column = Attribute {
            domain: AttrDomain::Face,
            values: AttrValues::F32(values),
        };
        mesh.attributes.insert(name.into(), column);
    }
    s.add_part("mapped", &mesh);
    let args = with(
        refining("Draw", ask.clone(), &json!({})),
        &json!({ "part": "mapped" }),
    );
    let done = s.query("brush.stroke", args);
    assert!(
        text(&done["live_detail"]["why_not"]).contains("uvs off"),
        "{done}"
    );
    // A part with detail levels keeps its topology.
    let mut s = sphere_session(None);
    s.ok("level.subdivide", json!({}));
    let done = s.query("brush.stroke", refining("Draw", ask, &json!({})));
    assert!(
        text(&done["live_detail"]["why_not"]).contains("detail levels"),
        "{done}"
    );
}

#[test]
fn undo_and_redo_give_back_each_version_of_a_part_numbered_as_given() {
    let mut s = sphere_session(None);
    let given = s.mesh("sphere");
    s.add_part("given", &given);
    let start = s.hash();
    let mut hashes = Vec::new();
    for size in [0.12, 0.3] {
        let args = refining("Clay", json!({ "size": size }), &json!({ "part": "given" }));
        let done = s.query("brush.stroke", args);
        assert_eq!(done["live_detail"]["refined"], json!(true), "{done}");
        valid_sphere(&mut s, "given");
        hashes.push(s.hash());
    }
    for want in hashes.iter().rev().skip(1).chain([&start]) {
        s.ok("history.undo", json!({}));
        assert_eq!(&s.hash(), want, "undo gives back the version before");
    }
    let back = s.mesh("given");
    assert_eq!(back.faces, given.faces, "the faces in the order given");
    let (a, b) = vertex_bits(&back, &given);
    assert!(a == b, "and every vertex as given, bit for bit");
    for want in &hashes {
        s.ok("history.redo", json!({}));
        assert_eq!(&s.hash(), want, "redo gives back the refined version");
    }
}

/// Each face as its corners' positions in winding order from the lowest, sorted: one surface
/// reads the same under any numbering, and whichever corner a store puts first.
fn surface(mesh: &PlainMesh) -> Vec<Vec<[u32; 3]>> {
    let corner = |&v: &u32| mesh.positions[v as usize].map(f32::to_bits);
    let mut faces: Vec<Vec<[u32; 3]>> = (mesh.faces.iter())
        .map(|f| {
            let mut c: Vec<[u32; 3]> = f.iter().filter(|&&v| v != NO_VERTEX).map(corner).collect();
            let first = (0..c.len()).min_by_key(|&i| c[i]).unwrap_or_default();
            c.rotate_left(first);
            c
        })
        .collect();
    faces.sort_unstable();
    faces
}

#[test]
fn mesh_reorder_keeps_the_mesh_in_a_new_numbering_and_undoes_exactly() {
    let mut s = sphere_session(None);
    s.ok(
        "brush.stroke",
        refining("Clay", json!({ "size": 0.12 }), &json!({})),
    );
    let (before, start) = (s.mesh("sphere"), s.hash());
    let effect = s.ok("mesh.reorder", json!({})).effect;
    let effect = effect.unwrap_or_else(|| panic!("no effect"));
    assert!(
        effect.topology_changed && effect.noop_reason.is_none(),
        "{effect:?}"
    );
    let after = s.mesh("sphere");
    assert_ne!(after.faces, before.faces, "a new numbering");
    assert_eq!(after.positions.len(), before.positions.len());
    assert!(surface(&after) == surface(&before), "the same surface");
    let reordered = s.hash();
    let again = s.ok("mesh.reorder", json!({})).effect;
    assert!(
        again.is_some_and(|e| e.noop_reason.is_some()),
        "a sorted part stays as it is"
    );
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), start, "undo gives back the old numbering");
    assert!(s.mesh("sphere").faces == before.faces, "and its faces");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), reordered, "redo the new one");
}

#[test]
fn pinning_fills_every_setting_into_the_logged_call() {
    let log = log_path("live-detail-pin");
    let mut s = sphere_session(Some(log.clone()));
    let state = s.query(
        "live_detail.set",
        json!({ "mode": "constant", "size": 0.03 }),
    );
    assert_eq!(
        state["on"],
        json!(false),
        "settings leave the pen's switch off: {state}"
    );
    s.ok(
        "brush.stroke",
        refining("Draw", json!({ "merge": 0.2 }), &json!({})),
    );
    // The brush's own settings come before the session's.
    let own = json!({ "settings": { "live_detail": { "mode": "brush", "size": 0.5 } } });
    s.ok("brush.stroke", refining("Clay", json!({}), &own));
    let logged: Vec<Value> = stroke_lines(&log).into_iter().map(|r| r.args).collect();
    // Settings are f32, so the log holds them as the f64 of an f32.
    let want = [
        json!({ "mode": "constant", "size": 0.03_f32, "merge": 0.2_f32, "combined": true,
                "budget": 4096, "smooth_split": true, "relax": 0.5 }),
        json!({ "mode": "brush", "size": 0.5, "merge": 0.4_f32, "combined": true,
                "budget": 4096, "smooth_split": true, "relax": 0.5 }),
    ];
    for (args, want) in logged.iter().zip(&want) {
        assert_eq!(&args["live_detail"], want, "{args}");
    }
    assert_eq!(logged.len(), 2);
    std::fs::remove_file(&log).ok();
}

/// A dab's target is at least 0.005 of its radius in every mode (review 0006, S2): brush mode
/// refuses a smaller size, and a constant size below it refines as brush mode's least does.
#[test]
fn a_size_far_below_the_brush_is_held_at_its_floor() {
    let mut s = sphere_session(None);
    let err = s.err(
        "brush.stroke",
        refining("Draw", json!({ "size": 0.001 }), &json!({})),
    );
    assert_eq!(err.path.as_deref(), Some("/live_detail/size"), "{err:?}");
    let err = s.err("live_detail.set", json!({ "mode": "brush", "size": 0.001 }));
    assert_eq!(err.path.as_deref(), Some("/size"), "{err:?}");
    // A brush wider than the sphere, so that its least target is near the sphere's edges.
    let run = |ld: Value| {
        let mut s = sphere_session(None);
        let points = line([-0.2, 0.0, 0.98], [0.2, 0.0, 0.98], 3);
        let args = stroke("Draw", &points, &json!({ "size": 12.0, "strength": 0.0 }));
        let done = s.query("brush.stroke", with(args, &json!({ "live_detail": ld })));
        (done["live_detail"]["splits"].clone(), s.hash())
    };
    let least = run(json!({ "size": 0.005 }));
    assert!(least.0.as_u64() > Some(0), "{least:?}");
    assert_eq!(run(json!({ "mode": "constant", "size": 0.0001 })), least);
}

#[test]
fn live_detail_set_keeps_the_session_settings() {
    let mut s = Session::new();
    let state = |s: &mut Session| s.query("session.state", json!({}));
    let st = state(&mut s);
    assert_eq!(st["live_detail.on"], json!(false), "off to start: {st}");
    assert_eq!(st["live_detail.mode"], json!("brush"));
    let now = s.query("live_detail.set", json!({ "on": true, "mode": "screen" }));
    assert_eq!(
        now["settings"]["size"],
        json!(30.0),
        "screen mode's own size: {now}"
    );
    let now = s.query("live_detail.set", json!({ "toggle": true, "size": 12 }));
    assert_eq!(now["on"], json!(false), "{now}");
    let st = state(&mut s);
    assert_eq!(st["live_detail.mode"], json!("screen"), "{st}");
    assert_eq!(st["live_detail.size"], json!(12.0), "{st}");
    for (args, path) in [
        (json!({ "on": true, "toggle": true }), "/toggle"),
        (json!({ "size": -1 }), "/size"),
        (json!({ "mode": "constant" }), "/size"),
        (json!({ "relax": 2 }), "/relax"),
    ] {
        let err = s.err("live_detail.set", args.clone());
        assert_eq!(err.path.as_deref(), Some(path), "{args}: {}", err.message);
    }
    assert_eq!(state(&mut s), st, "refused changes change nothing");
}

/// What the window's cursor shows: whether Live Detail can refine the active part under the
/// session's brush, and if not, what stands in the way (in decision 0021's order, section 8)
/// and what to do about it.
#[test]
fn session_state_says_whether_the_active_part_refines_under_the_brush() {
    let state = |s: &mut Session| {
        let st = s.query("session.state", json!({}));
        let get = |k: &str| st[format!("live_detail.{k}")].clone();
        let why = get("why_not").as_str().unwrap_or_default().to_string();
        (get("eligible"), get("blocked"), why)
    };
    let mut s = Session::new();
    let empty = s.query("session.state", json!({}));
    for k in ["eligible", "blocked", "why_not"] {
        let name = format!("live_detail.{k}");
        assert_eq!(
            empty.get(&name),
            Some(&Value::Null),
            "{name} in an empty scene"
        );
    }
    let mut s = sphere_session(None);
    assert_eq!(state(&mut s), (json!(true), Value::Null, String::new()));
    let opt_out = json!({ "brush": "Draw", "settings": { "live_detail": { "enable": false } } });
    for (set, blocked, why) in [
        (json!({ "brush": "Move" }), "brush", "drags"),
        (json!({ "brush": "Mask Pen" }), "brush", "paints the mask"),
        (opt_out, "opt_out", "enable\": true"),
    ] {
        s.ok("brush.set", set.clone());
        let (eligible, by, text) = state(&mut s);
        assert_eq!((eligible, by), (json!(false), json!(blocked)), "{set}");
        assert!(text.contains(why), "{set}: {text}");
    }
    let back = json!({ "settings": { "live_detail": { "enable": null } } });
    s.ok("brush.set", back);
    assert_eq!(
        state(&mut s).0,
        json!(true),
        "the brush's own setting again"
    );
    // A part with UV columns becomes active as it enters.
    let mut mesh = s.mesh("sphere");
    for name in ["uv0.u", "uv0.v"] {
        let values = (0..mesh.faces.len()).map(|f| f as f32 * 0.001).collect();
        let column = Attribute {
            domain: AttrDomain::Face,
            values: AttrValues::F32(values),
        };
        mesh.attributes.insert(name.into(), column);
    }
    s.add_part("mapped", &mesh);
    let (eligible, by, text) = state(&mut s);
    assert_eq!((eligible, by), (json!(false), json!("uvs")));
    assert!(text.contains("uvs off"), "{text}");
    s.ok("part.select", json!({ "part": "sphere" }));
    assert_eq!(state(&mut s).0, json!(true));
    // Detail levels come first, whatever the brush.
    s.ok("level.subdivide", json!({}));
    s.ok("brush.set", json!({ "brush": "Move" }));
    let (eligible, by, text) = state(&mut s);
    assert_eq!((eligible, by), (json!(false), json!("levels")));
    assert!(text.contains("level.delete_higher"), "{text}");
}

/// Refining strokes on a sphere and on a part numbered as given, logged with hashes; the hash
/// after each call.
fn session(log: Option<std::path::PathBuf>) -> Vec<String> {
    let config = EngineConfig {
        log_path: log,
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let given = s.mesh("sphere");
    s.add_part("given", &given);
    let sphere = json!({ "part": "sphere" });
    let mirror = json!({ "part": "sphere", "symmetry": { "mirror": { "x": true } }, "size": "8%" });
    let screen: Vec<Value> = [[420.0, 500.0], [520.0, 470.0], [610.0, 520.0]]
        .iter()
        .map(|p| json!({ "position": p, "pressure": 0.7 }))
        .collect();
    let calls = [
        refining("Draw", json!({ "size": 0.15 }), &sphere),
        refining("Clay Build", json!({}), &mirror),
        refining(
            "Inflate",
            json!({ "combined": false }),
            &json!({ "part": "given" }),
        ),
        json!({ "part": "sphere", "brush": "Clay", "camera": "front", "size_px": 48, "samples": screen,
               "live_detail": { "mode": "screen", "size": 10 } }),
        refining("Smooth", json!({ "size": 0.6 }), &sphere),
    ];
    let mut hashes = vec![s.hash()];
    for args in calls {
        s.ok("brush.stroke", args);
        hashes.push(s.hash());
    }
    hashes
}

#[test]
fn the_log_replays_refining_strokes_at_any_thread_count() {
    let log = log_path("live-detail-replay");
    let hashes = gs_par::with_threads(24, || session(Some(log.clone())));
    assert_eq!(
        gs_par::with_threads(1, || session(None)),
        hashes,
        "1 and 24 threads give the same hashes"
    );
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("no log: {e}"));
    let mut replay = Session::with(EngineConfig::default(), Role::Agent);
    for line in text.lines() {
        let Ok(ScriptLine::Record(record)) = ScriptLine::parse(line) else {
            continue;
        };
        let request = record.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), &record.command);
        assert_eq!(
            Some(replay.hash()),
            record.hash,
            "{} replays exactly",
            record.command
        );
    }
    assert_eq!(
        Some(&replay.hash()),
        hashes.last(),
        "the replay ends where the session did"
    );
    std::fs::remove_file(&log).ok();
}

/// Smooth at strength 0 changes only the density (decision 0021, section 3): its kernel moves
/// nothing, so it leaves the part exactly as Draw at strength 0 with the same dabs does, which
/// only the pass changes, splitting at a fine size and merging at a coarse one; and over a part
/// the pass has nothing to do on, it changes nothing at all.
#[test]
fn smooth_at_strength_0_changes_only_the_density() {
    let zero = json!({ "strength": 0.0, "settings": { "stroke": { "spacing": 0.025 } } });
    // At Smooth's own merge (0.45, STATUS item 122), for Draw and the refine as for Smooth.
    let constant = |size: f32| json!({ "mode": "constant", "size": size, "merge": 0.45 });
    let run = |brush: &str| {
        let mut s = sphere_session(None);
        for size in [0.03, 0.4] {
            let done = s.query("brush.stroke", refining(brush, constant(size), &zero));
            assert_eq!(
                done["live_detail"]["refined"],
                json!(true),
                "{brush}: {done}"
            );
        }
        valid_sphere(&mut s, "sphere");
        (s.mesh("sphere"), s.hash())
    };
    let ((smooth, a), (draw, b)) = (run("Smooth"), run("Draw"));
    assert!(
        smooth.faces == draw.faces,
        "Smooth and Draw made other faces"
    );
    assert_eq!(
        a, b,
        "Smooth at strength 0 moved a point Draw at strength 0 did not"
    );
    let mut s = sphere_session(None);
    s.ok("mesh.refine", json!({ "size": 0.05, "merge": 0.45 }));
    let rested = s.hash();
    let done = s.query("brush.stroke", refining("Smooth", constant(0.05), &zero));
    assert_eq!(done["live_detail"]["refined"], json!(false), "{done}");
    assert_eq!(
        s.hash(),
        rested,
        "a stroke with nothing to refine moved a point"
    );
}
