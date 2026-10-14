//! Session views (decision 0017), through commands: no View command touches the document or
//! its undo, a view's camera history gives back the exact camera, a gesture is one history step
//! and one log line, a replayed log gives back every camera bit for bit, an agent cannot move
//! `main` while a window shows it unless the human allows it, a locked view refuses every move,
//! a new view turns as a turntable about what it looks at (upside down too, the way the pointer
//! goes), Level takes a lean away, orbiting about the last edit point keeps it on its pixel, and
//! framing fits what it frames for any part size. Registry properties and metamorphic relations (ARCHITECTURE 10.1).

use std::path::PathBuf;

use gs_engine::{AGENT_VIEWS, EngineConfig};
use gs_protocol::{ErrorCode, Role};
use gs_render::Camera;
use gs_testkit::SplitMix64;
use gs_testkit::rng::signed_unit;
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::support::{EngineSession, agent, camera, document, exact, human, log_records, windowed};

/// A random camera command with its arguments, as a client would send it.
fn random_move(rng: &mut SplitMix64) -> (&'static str, Value) {
    let pick = rng.below(9);
    let mut u = || signed_unit(rng.next_u64());
    match pick {
        0 => (
            "view.orbit",
            json!({ "right_deg": 90.0 * u(), "up_deg": 60.0 * u() }),
        ),
        1 => ("view.roll", json!({ "deg": 90.0 * u() })),
        2 => ("view.pan", json!({ "dx": 200.0 * u(), "dy": 200.0 * u() })),
        3 => ("view.dolly", json!({ "factor": 1.0 + 0.5 * u() })),
        4 => ("view.frame", json!({})),
        5 => ("view.align", json!({ "to": "top", "toggle": true })),
        6 => (
            "view.lens",
            json!({ "projection": "toggle", "focal_mm": 30.0 + 20.0 * u().abs() }),
        ),
        7 => (
            "view.orbit",
            json!({ "right_deg": 30.0 * u(), "mode": "y", "snap": true }),
        ),
        _ => ("view.set", json!({ "camera": "iso" })),
    }
}

#[test]
fn no_view_command_changes_the_document_or_its_undo() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    s.ok("primitive.cube", json!({ "center": [2, 0, 0] }));
    let before = document(&mut s);
    let mut rng = SplitMix64::new(31);
    for k in 0..200 {
        let (command, args) = random_move(&mut rng);
        s.ok(command, args);
        if k % 7 == 0 {
            s.run("view.undo", json!({})).ok();
        }
        if k % 11 == 0 {
            s.run("view.redo", json!({})).ok();
        }
    }
    s.ok(
        "view.settings",
        json!({ "orbit": "z", "pivot_rule": "part" }),
    );
    s.ok("view.lock", json!({}));
    s.ok("view.lock", json!({}));
    assert_eq!(
        document(&mut s),
        before,
        "the document, its revision and its undo stayed"
    );
    let catalog = s.engine.catalog().clone();
    for info in catalog
        .commands
        .iter()
        .filter(|c| c.name.starts_with("view."))
    {
        let expect = match info.name.as_str() {
            "view.get" | "view.snapshot" | "view.contact_sheet" | "view.floor_save" => "query",
            _ => "view",
        };
        assert_eq!(
            json!(info.kind),
            json!(expect),
            "{} is a {expect} command",
            info.name
        );
    }
}

#[test]
fn camera_undo_gives_back_the_exact_camera_and_redo_the_moves() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    let start = exact(&mut s, "agent:test");
    let mut rng = SplitMix64::new(32);
    let mut after = Vec::new();
    for _ in 0..20 {
        let (command, args) = random_move(&mut rng);
        s.ok(command, args);
        after.push(exact(&mut s, "agent:test"));
    }
    for k in (0..20).rev() {
        s.ok("view.undo", json!({}));
        let want = if k == 0 { &start } else { &after[k - 1] };
        assert_eq!(
            &exact(&mut s, "agent:test"),
            want,
            "undo {} steps back",
            20 - k
        );
    }
    let error = s.err("view.undo", json!({}));
    assert_eq!(
        json!(error.code),
        json!(ErrorCode::Precondition),
        "nothing older"
    );
    for want in &after {
        s.ok("view.redo", json!({}));
        assert_eq!(&exact(&mut s, "agent:test"), want, "redo");
    }
    s.ok("view.undo", json!({}));
    s.ok("view.orbit", json!({ "right_deg": 5 }));
    assert!(
        s.run("view.redo", json!({})).is_err(),
        "a new move clears what could be redone"
    );
}

#[test]
fn the_history_keeps_fifty_steps() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    for _ in 0..60 {
        s.ok("view.orbit", json!({ "right_deg": 3 }));
    }
    let undo = s.query("view.get", json!({}))["undo"].clone();
    assert_eq!(undo, json!(50));
}

/// A fresh, unused log path.
pub(crate) fn log_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("views-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir.join("log.jsonl")
}

#[test]
fn a_gesture_is_one_step_and_one_log_line_and_a_replay_gives_every_camera_back() {
    let log = log_path("gesture");
    let mut s = Session::with(
        EngineConfig {
            log_path: Some(log.clone()),
            ..EngineConfig::default()
        },
        Role::Human,
    );
    s.ok("primitive.sphere", json!({}));
    s.ok("view.frame", json!({}));
    let undo_before = s.query("view.get", json!({}))["undo"].clone();
    for k in 0..100 {
        let args = json!({ "right_deg": 0.7, "up_deg": -0.3, "gesture": 7, "end": k == 99 });
        s.ok("view.orbit", args);
    }
    let undo = s.query("view.get", json!({}))["undo"].as_u64();
    assert_eq!(
        undo,
        undo_before.as_u64().map(|u| u + 1),
        "one history step"
    );
    // Single moves of every kind, and an agent's own view, after the gesture.
    s.ok("view.pan", json!({ "dx": 40, "dy": -25 }));
    s.ok("view.dolly", json!({ "factor": 0.7, "toward": [300, 600] }));
    s.ok("view.align", json!({ "to": "left", "toggle": true }));
    s.ok("view.roll", json!({ "deg": 17 }));
    s.ok("view.level", json!({}));
    s.ok("view.roll", json!({ "deg": -25 }));
    s.ok("view.settings", json!({ "orbit": "free" }));
    s.ok("view.settings", json!({ "orbit": "y" }));
    s.ok(
        "view.lens",
        json!({ "projection": "orthographic", "focal_mm": 50 }),
    );
    s.ok("view.orbit", json!({ "right_deg": 33, "snap": true }));
    s.by_agent("view.orbit", json!({ "right_deg": 12 }));
    let main = exact(&mut s, "main");
    let theirs = exact(&mut s, "agent:claude");
    let (_, records) = log_records(&log);
    let gesture_lines = (records.iter())
        .filter(|r| r.command == "view.orbit" || r.command == "view.set")
        .filter(|r| r.args["view"] == "main")
        .count();
    assert_eq!(
        gesture_lines, 2,
        "the gesture is one view.set line, the snap one orbit line"
    );
    let mut replay = human();
    replay.replay_log(&log);
    std::fs::remove_dir_all(log.parent().unwrap_or(&log)).ok();
    assert_eq!(exact(&mut replay, "main"), main, "main, bit for bit");
    assert_eq!(
        exact(&mut replay, "agent:claude"),
        theirs,
        "the agent's view, bit for bit"
    );
}

#[test]
fn an_agent_view_is_made_at_its_first_call_and_a_replay_makes_it_the_same() {
    let log = log_path("made");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    // Made at its first read, so main moving afterwards does not move it.
    let read = s.by("reader", "view.get", json!({}))["camera"].to_string();
    s.ok("view.orbit", json!({ "right_deg": 90 }));
    assert_eq!(exact(&mut s, "agent:reader"), read, "the view it read");
    // Made while main has a gesture open: the copy holds the gesture's turn so far.
    s.ok("view.orbit", json!({ "gesture": 1, "right_deg": 40 }));
    s.by_agent("view.orbit", json!({ "right_deg": 10 }));
    s.ok(
        "view.orbit",
        json!({ "gesture": 1, "end": true, "right_deg": 0 }),
    );
    let theirs = exact(&mut s, "agent:claude");
    let mut replay = human();
    replay.replay_log(&log);
    std::fs::remove_dir_all(log.parent().unwrap_or(&log)).ok();
    assert_eq!(
        exact(&mut replay, "agent:claude"),
        theirs,
        "made mid-gesture"
    );
    assert_eq!(exact(&mut replay, "agent:reader"), read, "made by a read");
}

#[test]
fn a_session_keeps_the_agent_views_used_last_and_drops_the_one_used_longest_ago() {
    let mut s = human();
    s.ok("primitive.sphere", json!({}));
    s.by("steady", "view.orbit", json!({ "right_deg": 30 }));
    for i in 0..AGENT_VIEWS + 5 {
        s.by(&format!("once-{i}"), "view.get", json!({}));
        s.by("steady", "view.get", json!({}));
    }
    let views = s.engine.views();
    let agents = views.keys().filter(|n| n.starts_with("agent:")).count();
    assert_eq!(agents, AGENT_VIEWS, "{:?}", views.keys());
    assert_eq!(
        views["agent:steady"].undo, 1,
        "the view in use keeps its history"
    );
    assert!(
        !views.contains_key("agent:once-0"),
        "the view used longest ago goes"
    );
    let last = format!("agent:once-{}", AGENT_VIEWS + 4);
    assert!(views.contains_key("main") && views.contains_key(&last));
}

/// Review 0003 M5: a refused camera command changes nothing: not the view's history, not its
/// gesture (opened or closed), not the log.
#[test]
fn a_refused_camera_command_leaves_the_view_and_the_log_as_they_were() {
    let log = log_path("refused");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("view.orbit", json!({ "right_deg": 10 }));
    s.ok("view.undo", json!({}));
    let before = s.query("view.get", json!({}));
    let refused = s.err("view.dolly", json!({ "factor": 3e38, "gesture": 7 }));
    assert_eq!(json!(refused.code), json!(ErrorCode::ArgsInvalid));
    assert_eq!(
        s.query("view.get", json!({})),
        before,
        "no step, no gesture"
    );
    s.ok("view.redo", json!({}));
    s.ok("view.orbit", json!({ "gesture": 2, "right_deg": 5 }));
    let open = s.query("view.get", json!({}));
    s.err("view.dolly", json!({ "factor": 3e38 }));
    assert_eq!(
        s.query("view.get", json!({})),
        open,
        "the gesture stays open"
    );
    s.ok(
        "view.orbit",
        json!({ "gesture": 2, "end": true, "right_deg": 5 }),
    );
    let (_, records) = log_records(&log);
    std::fs::remove_dir_all(log.parent().unwrap_or(&log)).ok();
    let commands: Vec<&str> = records.iter().map(|r| r.command.as_str()).collect();
    assert_eq!(
        commands,
        [
            "primitive.sphere",
            "view.orbit",
            "view.undo",
            "view.redo",
            "view.set"
        ],
        "the gesture's one line, nothing for the refused moves"
    );
}

/// Runs a command as an agent on a human's session: `claude`, or the client named.
trait ByAgent {
    fn by(&mut self, client: &str, command: &str, args: Value) -> Value;
    fn by_agent(&mut self, command: &str, args: Value) -> Value {
        self.by("claude", command, args)
    }
}

impl ByAgent for Session {
    fn by(&mut self, client: &str, command: &str, args: Value) -> Value {
        let me = self.origin.clone();
        self.origin = gs_engine::Origin {
            client_id: 9,
            client: client.into(),
            role: Role::Agent,
        };
        let out = self.ok(command, args).result;
        self.origin = me;
        out
    }
}

#[test]
fn an_agent_cannot_move_main_while_a_window_shows_it_unless_the_human_allows() {
    let mut s = windowed();
    s.ok("primitive.sphere", json!({}));
    let main = exact(&mut s, "main");
    let moves = [
        ("view.orbit", json!({ "view": "main", "right_deg": 20 })),
        (
            "view.frame",
            json!({ "view": "main", "target": "point", "point": [1, 0, 0] }),
        ),
        ("view.lock", json!({ "view": "main" })),
        ("view.undo", json!({ "view": "main" })),
    ];
    for (command, args) in &moves {
        let e = s.err(command, args.clone());
        assert_eq!(json!(e.code), json!(ErrorCode::Precondition), "{command}");
        assert!(
            e.hint.is_some_and(|h| h.contains("agents_move_view")),
            "says how"
        );
    }
    s.ok("view.orbit", json!({ "right_deg": 20 }));
    assert_eq!(
        exact(&mut s, "main"),
        main,
        "its own view moved, main did not"
    );
    assert_ne!(exact(&mut s, "agent:test"), main, "its own view did");
    let set = json!({ "camera": "main" });
    s.ok("view.set", set);
    assert_eq!(
        exact(&mut s, "agent:test"),
        main,
        "it can look from where main looks"
    );
    s.as_human("session.control", json!({ "agents_move_view": true }));
    s.ok("view.orbit", json!({ "view": "main", "right_deg": 20 }));
    assert_ne!(exact(&mut s, "main"), main, "allowed, it moves main");
    // Without a window no one looks through main, so replays and scripts may move it.
    let mut headless = agent();
    headless.ok("primitive.sphere", json!({}));
    headless.ok("view.orbit", json!({ "view": "main", "right_deg": 20 }));
}

#[test]
fn a_locked_view_refuses_every_move_and_stays_bit_identical() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("view.orbit", json!({ "right_deg": 10 }));
    s.ok("view.lock", json!({ "locked": true }));
    let locked = exact(&mut s, "agent:test");
    let mut rng = SplitMix64::new(33);
    for _ in 0..30 {
        let (command, args) = random_move(&mut rng);
        let e = s.err(command, args);
        assert_eq!(
            json!(e.code),
            json!(ErrorCode::Precondition),
            "{command}: {}",
            e.message
        );
        assert!(e.message.contains("locked"), "says why: {}", e.message);
    }
    s.err("view.undo", json!({}));
    assert_eq!(exact(&mut s, "agent:test"), locked);
    s.ok("view.lock", json!({ "locked": false }));
    s.ok("view.undo", json!({}));
}

#[test]
fn orbiting_about_the_last_edit_keeps_the_stroke_point_on_its_pixel() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    s.ok("view.settings", json!({ "pivot_rule": "last-edit" }));
    s.ok(
        "view.set",
        json!({ "camera": { "view": "front", "azimuth_deg": 25, "elevation_deg": 10 } }),
    );
    let stroke =
        json!({ "brush": "Draw", "size": 0.1, "samples": [{ "position": [0.6, 0.6, 0.53] }] });
    s.ok("brush.stroke", stroke);
    let last = s.query("view.get", json!({}))["last_edit"].clone();
    let point: [f32; 3] =
        serde_json::from_value(last).unwrap_or_else(|e| panic!("no last edit: {e}"));
    let size = [1024.0, 1024.0];
    let before = camera(&mut s, "agent:test").project(point, size);
    for (right, up) in [(30.0, 20.0), (-75.0, 5.0), (140.0, -60.0)] {
        s.ok("view.orbit", json!({ "right_deg": right, "up_deg": up }));
        let after = camera(&mut s, "agent:test").project(point, size);
        let off = before.zip(after).map_or(f32::INFINITY, |(a, b)| {
            let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
            (dx * dx + dy * dy).sqrt()
        });
        assert!(off < 0.05, "the edit point moved {off} px");
    }
    s.ok("view.frame", json!({}));
    let rule = s.query("view.get", json!({}));
    assert_eq!(rule["last_edit"], Value::Null, "framing forgets it");
    assert_eq!(
        rule["pivot"], rule["camera"]["target"],
        "and turns about what it framed"
    );
}

/// How far `after`'s eye moved to the right of `before`'s picture.
fn rightward(before: &Camera, after: &Camera) -> f32 {
    let (right, _, _) = before.axes();
    let [a, b] = [after.eye(), before.eye()];
    (0..3).map(|k| (a[k] - b[k]) * right.to_array()[k]).sum()
}

/// Research 21, 1.1 and 1.3: free turns leaned the horizon and the last edit pivot swung the
/// model away, so a new view turns as a turntable about the middle of its picture. The stroke's
/// point is still kept, for Frame last edit.
#[test]
fn a_new_view_turns_as_a_turntable_about_what_it_looks_at() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let stroke =
        json!({ "brush": "Draw", "size": 0.1, "samples": [{ "position": [0.6, 0.6, 0.53] }] });
    s.ok("brush.stroke", stroke);
    let view = s.query("view.get", json!({}));
    assert_eq!(
        (&view["orbit"], &view["pivot_rule"]),
        (&json!("y"), &json!("target"))
    );
    assert!(
        view["last_edit"].is_array(),
        "the stroke's point is kept: {view}"
    );
    assert_eq!(view["pivot"], view["camera"]["target"], "{view}");
    let mut rng = SplitMix64::new(41);
    for _ in 0..40 {
        let (right, up) = (
            90.0 * signed_unit(rng.next_u64()),
            40.0 * signed_unit(rng.next_u64()),
        );
        let before = camera(&mut s, "agent:test");
        s.ok("view.orbit", json!({ "right_deg": right, "up_deg": up }));
        let after = camera(&mut s, "agent:test");
        let (r, _, _) = after.axes();
        assert!(r.y.abs() < 1e-4, "the horizon stays level: right is {r:?}");
        assert_eq!(
            after.target, before.target,
            "it turns about what it looks at"
        );
    }
    s.ok("view.set", json!({ "camera": "front" }));
    let start = camera(&mut s, "agent:test");
    for _ in 0..8 {
        s.ok("view.orbit", json!({ "right_deg": 45 }));
    }
    let round = camera(&mut s, "agent:test");
    let (a, b) = (start.orientation.to_array(), round.orientation.to_array());
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    assert!(
        dot.abs() > 1.0 - 1e-6,
        "eight eighths come back: {a:?} {b:?}"
    );
}

/// Research 21, 1.7 item 2 (Blender's rule): upside down, a sideways turn still moves the camera
/// the way the pointer goes; a gesture keeps the way it started with, even once it has turned
/// over the top.
#[test]
fn upside_down_a_turn_follows_the_pointer_and_a_gesture_keeps_its_way() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("view.set", json!({ "camera": "front" }));
    s.ok("view.roll", json!({ "deg": 180 }));
    let before = camera(&mut s, "agent:test");
    s.ok("view.orbit", json!({ "right_deg": 20 }));
    let after = camera(&mut s, "agent:test");
    assert!(
        rightward(&before, &after) > 0.0,
        "it went right on the picture"
    );

    s.ok("view.set", json!({ "camera": "front" }));
    let turn = |right: f32, up: f32, end: bool| json!({ "right_deg": right, "up_deg": up, "gesture": 3, "end": end });
    s.ok("view.orbit", turn(0.0, 0.0, false));
    s.ok("view.orbit", turn(0.0, 150.0, false));
    let over = camera(&mut s, "agent:test");
    assert!(over.axes().1.y < 0.0, "over the top it is upside down");
    s.ok("view.orbit", turn(20.0, 0.0, true));
    let kept = camera(&mut s, "agent:test");
    assert!(
        rightward(&over, &kept) < 0.0,
        "the gesture started upright, so it turned as it began"
    );
    s.ok("view.orbit", json!({ "right_deg": 20 }));
    let lone = camera(&mut s, "agent:test");
    assert!(
        rightward(&kept, &lone) > 0.0,
        "a new turn starts upside down"
    );
}

/// Research 21, 1.4: Level takes the lean away and keeps where the view looks, upside down
/// included; a level view stays, with no history step; a locked view refuses it.
#[test]
fn level_takes_the_lean_away_and_keeps_where_the_view_looks() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    let tilted = json!({ "camera": { "view": "front", "azimuth_deg": 30, "elevation_deg": 20 } });
    for roll in [35.0, 180.0, -120.0] {
        s.ok("view.set", tilted.clone());
        s.ok("view.roll", json!({ "deg": roll }));
        let leaned = camera(&mut s, "agent:test");
        let leaned_exact = exact(&mut s, "agent:test");
        s.ok("view.level", json!({}));
        let level = camera(&mut s, "agent:test");
        let (r, u, f) = level.axes();
        assert!(
            r.y.abs() < 1e-5 && u.y > 0.0,
            "level and upright: {r:?} {u:?}"
        );
        assert!(f.dot(leaned.axes().2) > 1.0 - 1e-6, "looking the same way");
        let off = (0..3).map(|k| (level.target[k] - leaned.target[k]).abs());
        assert!(
            off.fold(0.0, f32::max) < 1e-5,
            "about the point it looks at"
        );
        assert_eq!(level.distance, leaned.distance);
        let undo = s.query("view.get", json!({}))["undo"].clone();
        s.ok("view.level", json!({}));
        assert_eq!(
            s.query("view.get", json!({}))["undo"],
            undo,
            "level stays, no step"
        );
        s.ok("view.undo", json!({}));
        assert_eq!(exact(&mut s, "agent:test"), leaned_exact, "one step back");
    }
    s.ok("view.lock", json!({ "locked": true }));
    let e = s.err("view.level", json!({}));
    assert!(e.message.contains("locked"), "{}", e.message);
}

/// Research 21, 1.7 item 6: turning the turntable on levels a view leaned in free mode, once
/// and as one history step; a locked view keeps its camera and still takes the setting.
#[test]
fn turning_the_turntable_on_levels_a_view_leaned_in_free_mode() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("view.settings", json!({ "orbit": "free" }));
    s.ok("view.set", json!({ "camera": "front" }));
    for (right, up) in [(40.0, 30.0), (-70.0, 25.0), (60.0, -50.0)] {
        s.ok("view.orbit", json!({ "right_deg": right, "up_deg": up }));
    }
    let leaned = exact(&mut s, "agent:test");
    assert!(
        camera(&mut s, "agent:test").axes().0.y.abs() > 0.05,
        "free turns leaned it"
    );
    s.ok("view.settings", json!({ "orbit": "y" }));
    let (r, u, _) = camera(&mut s, "agent:test").axes();
    assert!(r.y.abs() < 1e-5 && u.y > 0.0, "levelled: {r:?} {u:?}");
    let level = exact(&mut s, "agent:test");
    s.ok("view.settings", json!({ "orbit": "y" }));
    assert_eq!(
        exact(&mut s, "agent:test"),
        level,
        "the same mode again does nothing"
    );
    s.ok("view.undo", json!({}));
    assert_eq!(exact(&mut s, "agent:test"), leaned, "one step back");
    s.ok("view.settings", json!({ "orbit": "free" }));
    s.ok("view.lock", json!({ "locked": true }));
    s.ok("view.settings", json!({ "orbit": "y" }));
    assert_eq!(
        exact(&mut s, "agent:test"),
        leaned,
        "a locked view does not turn"
    );
    assert_eq!(s.query("view.get", json!({}))["orbit"], "y");
}

/// Review 0003 m1: a Select Rect outlines a region on the camera plane and touches no surface,
/// so it leaves the last edit point alone; the next orbit does not turn about empty space.
#[test]
fn a_select_rect_leaves_no_last_edit_point() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    let corners = [[300, 300], [700, 700]];
    let samples: Vec<Value> = corners.iter().map(|p| json!({ "position": p })).collect();
    let rect = json!({ "brush": "Select Rect", "camera": "front", "samples": samples });
    let hid = s.ok("brush.stroke", rect);
    let hid = hid.effect.and_then(|e| e.noop_reason);
    assert_eq!(hid, None, "the rectangle hid faces");
    let view = s.query("view.get", json!({}));
    assert_eq!(view["last_edit"], Value::Null, "{view}");
    assert_eq!(view["pivot"], view["camera"]["target"], "{view}");
}

/// Whether every corner of the box `lo..hi` is drawn by `c` inside the picture, with the 5%
/// border framing leaves (and a little rounding).
fn inside(c: &Camera, lo: [f32; 3], hi: [f32; 3], size: [f32; 2]) -> bool {
    (0..8).all(|k| {
        let p = [0, 1, 2].map(|i| if k & (1 << i) == 0 { lo[i] } else { hi[i] });
        c.project(p, size).is_some_and(|[x, y]| {
            let (nx, ny) = (2.0 * x / size[0] - 1.0, 1.0 - 2.0 * y / size[1]);
            nx.abs() <= 0.9 + 1e-3 && ny.abs() <= 0.9 + 1e-3
        })
    })
}

#[test]
fn framing_fits_what_it_frames_for_any_part_size() {
    let mut rng = SplitMix64::new(34);
    for k in 0..24 {
        let mut s = agent();
        let scale = [1e-3, 1e-2, 0.1, 1.0, 10.0, 100.0, 1e3, 1e4][k % 8];
        let mut u = || signed_unit(rng.next_u64());
        let center = [u() * scale * 5.0, u() * scale * 5.0, u() * scale * 5.0];
        s.ok(
            "primitive.cube",
            json!({ "size": scale, "center": center, "name": "a" }),
        );
        let far = [center[0] + 40.0 * scale, center[1], center[2]];
        s.ok(
            "primitive.sphere",
            json!({ "radius": scale * 0.3, "center": far, "name": "b" }),
        );
        let viewport = [1200, 700];
        let size = [1200.0, 700.0];
        s.ok(
            "view.orbit",
            json!({ "right_deg": 100.0 * u(), "up_deg": 60.0 * u(), "viewport": viewport }),
        );
        s.ok(
            "view.frame",
            json!({ "target": "part", "part": "a", "viewport": viewport }),
        );
        let h = scale * 0.5;
        let (lo, hi) = (center.map(|c| c - h), center.map(|c| c + h));
        let c = camera(&mut s, "agent:test");
        assert!(inside(&c, lo, hi, size), "scale {scale}: part a framed");
        s.ok("view.frame", json!({ "viewport": viewport }));
        let c = camera(&mut s, "agent:test");
        let r = scale * 0.3;
        let (lo2, hi2) = (far.map(|c| c - r), far.map(|c| c + r));
        assert!(
            inside(&c, lo, hi, size) && inside(&c, lo2, hi2, size),
            "scale {scale}: all"
        );
    }
}

#[test]
fn framing_the_mask_fits_the_masked_points() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let mask = json!({ "brush": "Mask Pen", "size": 0.3, "strength": 1, "samples": [{ "position": [0, 0, 1] }] });
    s.ok("brush.stroke", mask);
    s.ok(
        "view.frame",
        json!({ "target": "mask", "viewport": [800, 800] }),
    );
    let c = camera(&mut s, "agent:test");
    let mesh = s.mesh("sphere");
    let masked: Vec<[f32; 3]> = match mesh.attributes.get("mask").map(|a| &a.values) {
        Some(gs_protocol::AttrValues::F32(m)) => (mesh.positions.iter().zip(m))
            .filter(|(_, m)| **m > 0.0)
            .map(|(p, _)| *p)
            .collect(),
        _ => panic!("no mask"),
    };
    assert!(
        masked.len() > 10,
        "the stroke masked a patch: {}",
        masked.len()
    );
    let size = [800.0, 800.0];
    let reach = (masked.iter())
        .filter_map(|p| c.project(*p, size))
        .map(|[x, y]| ((2.0 * x / size[0] - 1.0).abs()).max((1.0 - 2.0 * y / size[1]).abs()))
        .fold(0.0f32, f32::max);
    assert!(
        reach <= 0.9 + 1e-3,
        "every masked point is in the frame: {reach}"
    );
    assert!(
        reach > 0.3,
        "and the patch fills a good part of it: {reach}"
    );
    success(
        s.call("view.frame", json!({ "target": "unmasked" })),
        "unmasked",
    );
}

/// The look commands take what a picture draws beside their other arguments, each documented
/// with its default, and a misspelled one is refused with the right name.
#[test]
fn look_switches_are_flat_documented_arguments() {
    let catalog = agent().engine.catalog().clone();
    let looks = [
        ("view.snapshot", json!({ "wirefame": true })),
        (
            "view.contact_sheet",
            json!({ "views": ["front"], "wirefame": true }),
        ),
    ];
    for (command, misspelled) in looks {
        let info = catalog.commands.iter().find(|c| c.name == command);
        let properties = info.map(|c| c.args_schema["properties"].clone());
        let properties = properties.unwrap_or_default();
        for arg in [
            "wireframe",
            "antialias",
            "grid",
            "active_outline",
            "face_groups",
        ] {
            let documented = properties[arg]["description"].as_str().unwrap_or_default();
            assert!(!documented.is_empty(), "{command} documents {arg}");
        }
        assert_eq!(properties["antialias"]["default"], json!(true), "{command}");
        assert_eq!(properties["grid"]["default"], json!(false), "{command}");
        let err = agent().err(command, misspelled);
        assert_eq!(err.code, ErrorCode::ArgsInvalid, "{command}");
        assert_eq!(
            err.suggestions.first().map(String::as_str),
            Some("wireframe"),
            "{command}"
        );
    }
}
