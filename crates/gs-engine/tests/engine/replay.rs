//! The command log: what it records, that replaying it rebuilds the scene, that a restart never
//! erases it, that the standard log folder stays bounded, and that verify mode lets a replay
//! name the first call that differs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use gs_engine::EngineConfig;
use gs_protocol::Role;
use serde_json::json;

use crate::determinism::{determinism_script, seeded};
use gs_testkit::{Session, success};

use crate::support::{EngineSession, log_records};

/// A fresh, unused log path.
fn log_path(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("replay-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
    dir.join(format!("{name}.jsonl"))
}

fn remove(log: &Path) {
    if let Some(dir) = log.parent() {
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn replaying_the_log_rebuilds_the_scene() {
    let log = log_path("replay");
    let mut s = seeded(3, Some(log.clone()));
    let hashes = determinism_script(&mut s);
    s.ok("mesh.stats", json!({}));
    success(
        s.batch(&[
            ("primitive.cube", json!({ "name": "b" })),
            ("mesh.stats", json!({})),
        ]),
        "batch",
    );
    let target = s.hash();
    let (header, records) = log_records(&log);
    assert_eq!(
        (header.session.as_str(), header.seed, header.verify),
        ("main", 3, false),
        "the header names the session"
    );
    assert_eq!(
        header.catalog_hash,
        s.engine.catalog().hash,
        "and the catalog"
    );
    assert_eq!(
        records.len(),
        hashes.len() + 1,
        "one line per call that may change the session, a batch as one line"
    );
    let seqs: Vec<u64> = records.iter().filter_map(|r| r.seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "sequence numbers increase: {seqs:?}"
    );
    let batch = records.last().map(|r| {
        (
            r.command.clone(),
            r.args["commands"][0]["args"]["name"].clone(),
        )
    });
    assert_eq!(
        batch,
        Some(("cmd.batch".into(), json!("b"))),
        "the batch, resolved"
    );
    assert!(
        records
            .iter()
            .all(|r| r.client.as_deref() == Some("test#1")),
        "who ran it"
    );
    assert!(
        records.iter().all(|r| r.hash.is_none()),
        "no hashes without verify"
    );
    let mut replay = seeded(99, None);
    replay.replay_log(&log);
    remove(&log);
    assert_eq!(
        replay.hash(),
        target,
        "replay with another session seed gives the same scene"
    );
}

/// C1: a batch is one undo step, so its log line must be one batch, or a later undo replays
/// differently.
#[test]
fn a_batch_then_an_undo_replays_to_the_same_scene() {
    let log = log_path("batch-undo");
    let mut s = seeded(0, Some(log.clone()));
    success(
        s.batch(&[
            ("primitive.sphere", json!({ "subdivisions": 2 })),
            (
                "mesh.translate",
                json!({ "direction": [1, 0, 0], "distance": 1 }),
            ),
        ]),
        "batch",
    );
    s.ok("history.undo", json!({}));
    let mut replay = seeded(0, None);
    replay.replay_log(&log);
    remove(&log);
    assert_eq!(replay.hash(), s.hash(), "the replay is empty too");
}

/// C2: step ids count steps, so an undo that names its step replays even when queries and
/// failed commands used up sequence numbers in the original session.
#[test]
fn an_undo_that_names_its_step_replays() {
    let log = log_path("expect");
    let mut s = seeded(0, Some(log.clone()));
    s.ok("mesh.stats", json!({}));
    s.err(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 1 }),
    );
    let made = s.ok("primitive.cube", json!({}));
    assert_eq!(made.history_id, Some(1), "the first step is step 1");
    s.ok("mesh.stats", json!({}));
    s.ok("history.undo", json!({ "expect": made.history_id }));
    let mut replay = seeded(0, None);
    replay.replay_log(&log);
    remove(&log);
    assert_eq!(replay.hash(), s.hash(), "the replay undid the same step");
}

/// M4: a session that restarts under the same name keeps its earlier log, and each log starts
/// with its own header, so each replays on its own.
#[test]
fn a_restart_keeps_the_earlier_log() {
    let log = log_path("restart");
    let first_run = {
        let mut s = seeded(0, Some(log.clone()));
        s.ok("primitive.sphere", json!({}));
        s.ok(
            "mesh.translate",
            json!({ "direction": [1, 0, 0], "distance": 1 }),
        );
        s.hash()
    };
    let mut second = seeded(0, Some(log.clone()));
    second.ok("primitive.cube", json!({}));
    let kept = log.with_file_name("restart-1.jsonl");
    let (_, earlier) = log_records(&kept);
    let (_, latest) = log_records(&log);
    assert_eq!((earlier.len(), latest.len()), (2, 1), "both runs are kept");
    let mut third = seeded(0, Some(log.clone()));
    third.ok("primitive.cube", json!({}));
    assert!(
        log.with_file_name("restart-2.jsonl").exists(),
        "and the next one"
    );
    let mut replay = seeded(0, None);
    replay.replay_log(&kept);
    remove(&log);
    assert_eq!(
        replay.hash(),
        first_run,
        "the kept log replays the first run"
    );
}

/// The standard log folder stays bounded (decision 0016, section 14): a session start deletes
/// the oldest closed logs beyond the limit, whole, with the lock file a crash left beside one,
/// and keeps a log another session is still writing (it holds the lock beside it) however old
/// it is. Its own log lands in the folder under its session name and pid.
#[test]
fn a_session_start_prunes_the_oldest_closed_logs() {
    let state = log_path("unused").with_file_name("state");
    let logs = state.join("logs");
    std::fs::create_dir_all(&logs).unwrap_or_else(|e| panic!("{e}"));
    let made = |name: &str, age: u64| {
        let path = logs.join(name);
        let file = std::fs::File::create(&path).unwrap_or_else(|e| panic!("{e}"));
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 - age);
        file.set_modified(when).unwrap_or_else(|e| panic!("{e}"));
        path
    };
    let newest = [
        made("a-1.jsonl", 1),
        made("b-2.jsonl", 2),
        made("c-3.jsonl", 3),
    ];
    let oldest = [made("d-4.jsonl", 4), made("e-5.jsonl", 5)];
    let left = made("d-4.jsonl.lock", 4);
    let live = made("live-6.jsonl", 6);
    let writer = std::fs::File::open(made("live-6.jsonl.lock", 6));
    let writer = writer.unwrap_or_else(|e| panic!("{e}"));
    writer.try_lock().unwrap_or_else(|e| panic!("{e}"));
    let mut config = EngineConfig::default().with_standard_log(state.clone());
    if let Some(standard) = config.standard_log.as_mut() {
        standard.files = 3;
    }
    let own = config.log_path.clone().unwrap_or_default();
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.cube", json!({}));
    let kept = |p: &PathBuf| p.exists();
    assert!(newest.iter().all(kept), "the newest three stay");
    assert!(!oldest.iter().any(kept), "the oldest closed ones go");
    assert!(!left.exists(), "with the lock file a crash left beside one");
    assert!(live.exists(), "a log being written stays");
    let name = format!("main-{}.jsonl", std::process::id());
    assert_eq!(own, logs.join(name), "the session's own log");
    assert_eq!(log_records(&own).1.len(), 1, "and it writes there");
    drop((s, writer));
    remove(&state);
}

/// Decision 0063: the lock that tells pruning a log is still being written is on a file beside
/// it, `<log>.lock`, not on the log. Locks on Windows are mandatory, so one on the log stopped
/// every other reader there: a recovery in another session, a bug bundle, `gs run` replaying
/// it. Here another handle can even lock the log while it is written. A clean stop removes the
/// lock file.
#[test]
fn a_log_being_written_is_locked_beside_it_not_on_it() {
    let log = log_path("beside");
    let mut s = seeded(0, Some(log.clone()));
    s.ok("primitive.cube", json!({}));
    let reader = std::fs::File::open(&log).unwrap_or_else(|e| panic!("{e}"));
    assert!(reader.try_lock().is_ok(), "nothing locks the log itself");
    drop(reader);
    let beside = log.with_file_name("beside.jsonl.lock");
    let lock = std::fs::File::open(&beside).unwrap_or_else(|e| panic!("{e}"));
    assert!(lock.try_lock().is_err(), "the lock beside it is held");
    drop((lock, s));
    assert!(!beside.exists(), "and goes with a clean stop");
    remove(&log);
}

/// M5: pausing changes who may edit, not the scene, so it is not logged and a log from a
/// session the human paused replays.
#[test]
fn pausing_is_not_logged() {
    let log = log_path("paused");
    let mut s = seeded(0, Some(log.clone()));
    success(
        s.as_human("session.control", json!({ "paused": true })),
        "pause",
    );
    success(s.as_human("primitive.sphere", json!({})), "the human edits");
    success(
        s.as_human("session.control", json!({ "paused": false })),
        "go on",
    );
    s.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 1 }),
    );
    let (_, records) = log_records(&log);
    let commands: Vec<&str> = records.iter().map(|r| r.command.as_str()).collect();
    // The view.set is the agent's view, made at its first call as a copy of main's camera.
    assert_eq!(
        commands,
        ["primitive.sphere", "view.set", "mesh.translate"],
        "no session.control"
    );
    let mut replay = seeded(0, None);
    replay.replay_log(&log);
    remove(&log);
    assert_eq!(replay.hash(), s.hash(), "an agent replays it");
}

/// The display (decision 0023) is logged, so a replay draws the same pictures, and it comes
/// back as `session.state` names it, occlusion's strength and radius too (decision 0019 as
/// amended for Q8). A probe file is logged with its hash, as every file read.
#[test]
fn the_display_is_logged_and_replays() {
    let log = log_path("display");
    let mut s = seeded(0, Some(log.clone()));
    let display = json!({
        "shading": "lit", "exposure": -0.5, "transform": "agx",
        "subsurface": { "mode": "on", "radius": 2 },
        "occlusion_strength": 0.5, "occlusion_radius": 0.25,
        "floor": true, "mask": false, "mask_opacity": 0.5, "active_outline": false,
        "face_groups": true
    });
    s.ok("view.display", display);
    let occlusion = |s: &mut Session| {
        let state = s.query("session.state", json!({}));
        (
            state["view.occlusion_strength"].clone(),
            state["view.occlusion_radius"].clone(),
        )
    };
    assert_eq!(occlusion(&mut s), (json!(0.5), json!(0.25)));
    s.err("view.display", json!({ "occlusion_strength": 2 }));
    s.ok("view.display", json!({ "occlusion_radius": "auto" }));
    assert_eq!(
        occlusion(&mut s),
        (json!(0.5), json!("auto")),
        "the strength left out keeps its value"
    );
    s.err("view.display", json!({ "mask_opacity": -0.5 }));
    s.ok("view.display", json!({ "mask": true }));
    s.ok(
        "light.set",
        json!({ "light": 3, "on": true, "color": [1, 0.5, 0.25] }),
    );
    s.ok("light.set", json!({ "ambient": 0.25 }));
    let probe = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/environments/studio_small_09_1k.hdr"
    );
    s.ok("view.environment", json!({ "path": probe, "turn": 90 }));
    let (_, records) = log_records(&log);
    let read = records.iter().find(|r| r.command == "view.environment");
    let hash = read.map(|r| r.args["hash"].clone()).unwrap_or_default();
    assert!(hash.as_str().is_some_and(|h| h.len() == 64), "{hash}");

    let shown = |s: &mut Session| -> Vec<(String, serde_json::Value)> {
        let state = s.query("session.state", json!({}));
        let values = state.as_object().cloned().unwrap_or_default();
        let display = |k: &String| ["view.", "light."].iter().any(|p| k.starts_with(p));
        values.into_iter().filter(|(k, _)| display(k)).collect()
    };
    let mut replay = seeded(0, None);
    replay.replay_log(&log);
    remove(&log);
    let after = shown(&mut s);
    assert_eq!(shown(&mut replay), after, "the replay displays the same");
    let value = |name: &str| after.iter().find(|p| p.0 == name).map(|p| p.1.clone());
    assert_eq!(value("view.shading"), Some(json!("lit")));
    assert_eq!(value("view.subsurface.radius"), Some(json!(2.0)));
    assert_eq!(value("view.environment"), Some(json!(probe)));
    assert_eq!(value("light.ambient"), Some(json!(0.25)));
    assert_eq!(
        [
            value("view.floor"),
            value("view.mask"),
            value("view.mask_opacity"),
            value("view.active_outline"),
            value("view.face_groups")
        ],
        [
            Some(json!(true)),
            Some(json!(true)),
            Some(json!(0.5)),
            Some(json!(false)),
            Some(json!(true))
        ],
        "the floor, the opacity, the outline and the face groups left out keep their values"
    );
    assert_eq!(
        value("light.3").map(|l| l["color"].clone()),
        Some(json!([1.0, 0.5, 0.25]))
    );
}

/// Review 0005, m1: the selected look is logged, so a replay draws the same pictures and not
/// only the same scene; a session that cannot render still replays the line.
#[test]
fn a_replay_draws_the_same_pictures() {
    let log = log_path("look");
    let drawing = |log: Option<PathBuf>| {
        let mut s = seeded(0, log);
        let renderer = gs_ui::Offscreen::host(None).unwrap_or_else(|e| panic!("{e}"));
        s.engine.attach_renderer(renderer);
        s
    };
    let picture = |s: &mut Session| {
        let shot = s.query("view.snapshot", json!({ "size": 64, "antialias": false }));
        shot["png_base64"].as_str().unwrap_or_default().to_string()
    };
    let mut s = drawing(Some(log.clone()));
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("view.display", json!({ "shading": "lit" }));
    let gray = picture(&mut s);
    s.ok("material.select", json!({ "name": "skin-4" }));
    let skin = picture(&mut s);
    assert_ne!(skin, gray, "the look changes the picture");

    let mut replay = drawing(None);
    replay.replay_log(&log);
    assert_eq!(
        picture(&mut replay),
        skin,
        "the replay draws the same picture"
    );
    seeded(0, None).replay_log(&log);
    remove(&log);
}

/// Decision 0029: a stroke on a part at the identity transform is what it was before part
/// transforms, bit for bit. `golden/strokes.jsonl` is a verify-mode log written by main at
/// c155bc1, before them: world, screen, mirrored, locked, Live Detail, Move, Mask Pen and Smooth
/// strokes on one sphere, then an undo. A change to a brush preset or to what a stroke does
/// fails it by design; write the log again with the same calls, on main, when that change is
/// meant (c155bc1 changed what Live Detail does, so its lines and those after it changed;
/// decision 0036 changed the Draw and Smooth presets, so their hashes and every scene hash from
/// the first stroke on were written again, the calls the same; Smooth's Live Detail merge of
/// 0.45 changed only its preset hash, since that stroke does not refine; decision 0036's
/// spacer, its first dab on the press, moved every scene hash from the first stroke on;
/// decision 0040's quad split moved where a screen stroke's pick lands, so every scene hash
/// from the first screen stroke on was written again).
#[test]
fn a_stroke_log_from_before_part_transforms_replays_bit_for_bit() {
    let log = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/engine/golden/strokes.jsonl");
    let (header, records) = log_records(&log);
    assert!(
        header.verify && records.iter().all(|r| r.hash.is_some()),
        "every line carries the scene after it"
    );
    seeded(header.seed, None).replay_log(&log);
}

/// G1: in verify mode every line carries the scene hash after it.
#[test]
fn verify_mode_records_the_scene_after_every_call() {
    let log = log_path("verify");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    let mut hashes = determinism_script(&mut s);
    s.ok("mesh.stats", json!({}));
    s.ok("history.undo", json!({}));
    hashes.push(s.hash());
    let (header, records) = log_records(&log);
    remove(&log);
    assert!(header.verify, "the header says so");
    let logged: Vec<String> = records.iter().filter_map(|r| r.hash.clone()).collect();
    assert_eq!(logged, hashes, "the hash after each logged call");
}

/// Decision 0015: part ids come from a counter only logged calls move, so a replay hands out
/// the same ids, after undos, failed calls and batches.
#[test]
fn a_replay_hands_out_the_same_ids() {
    let log = log_path("ids");
    let mut s = seeded(5, Some(log.clone()));
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({}));
    s.ok("history.undo", json!({}));
    s.err(
        "mesh.translate",
        json!({ "part": "#2", "direction": [1, 0, 0], "distance": 1 }),
    );
    s.ok("primitive.cube", json!({ "name": "box" }));
    success(
        s.batch(&[
            ("primitive.sphere", json!({ "name": "ball" })),
            (
                "mesh.translate",
                json!({ "part": "@1", "direction": [0, 1, 0], "distance": 1 }),
            ),
        ]),
        "batch",
    );
    let want = crate::parts::rows(&mut s);
    assert_eq!(
        want.iter().map(|r| r.0).collect::<Vec<_>>(),
        [1, 3, 4],
        "{want:?}"
    );
    let mut replay = seeded(77, None);
    replay.replay_log(&log);
    remove(&log);
    assert_eq!(
        crate::parts::rows(&mut replay),
        want,
        "the same ids in the same order"
    );
    assert_eq!(replay.hash(), s.hash(), "and the same scene");
}
