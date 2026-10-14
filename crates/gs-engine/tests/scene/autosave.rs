//! Autosave with a clock moved by hand: Max Duration and Rest Duration fire as the catalog's
//! `interface.autosave` says, a manual save restarts Max Duration only, `file.autosave`'s
//! settings last across sessions and Max files keeps that many copies of each scene, a clean
//! stop removes the copies a saved scene no longer needs, a recovery replays what several
//! clients did after the last copy, and a recovered session autosaves at once.

use std::path::Path;
use std::time::Duration;

use gs_engine::{EngineConfig, ManualClock, Origin, StateFolder};
use gs_protocol::{ErrorCode, Outcome, Role};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::{at, folder, save, session, stroke};

const MINUTE: Duration = Duration::from_secs(60);

/// A session autosaving into `state` with the settings kept there, on a clock moved by hand.
pub fn clocked(state: &Path) -> (Session, ManualClock) {
    let config = EngineConfig {
        autosave: Some(StateFolder::in_folder(state.to_path_buf())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    let clock = ManualClock::default();
    s.engine.set_clock(Box::new(clock.clone()));
    (s, clock)
}

/// The autosave that fires now, if one does: the copy's path.
pub fn tick(s: &mut Session) -> Option<String> {
    let response = s.engine.tick()?;
    match response.outcome {
        Outcome::Ok(done) => done.result["path"].as_str().map(str::to_string),
        Outcome::Error(e) => panic!("the autosave failed: {}", e.message),
    }
}

/// Some input: a small edit.
fn poke(s: &mut Session) {
    s.ok(
        "mesh.translate",
        json!({ "part": "sphere", "direction": [1, 0, 0], "distance": 0.001 }),
    );
}

#[test]
fn rest_and_max_duration_fire_as_the_catalog_says() {
    let state = folder("autosave-triggers");
    let (mut s, clock) = clocked(&state);
    assert_eq!(tick(&mut s), None, "an empty scene is not autosaved");
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    clock.advance(Duration::from_secs(119));
    assert_eq!(tick(&mut s), None, "not before two minutes without input");
    clock.advance(Duration::from_secs(1));
    let copy = tick(&mut s).expect("Rest Duration fired");
    assert!(
        copy.starts_with(state.to_str().unwrap()) && copy.ends_with("-001.gss"),
        "{copy}"
    );
    clock.advance(3 * MINUTE);
    assert_eq!(tick(&mut s), None, "nothing changed since the autosave");
    // Input every minute keeps Rest Duration from firing; Max Duration fires anyway, ten
    // minutes after the autosave.
    for minute in 1..=7 {
        poke(&mut s);
        clock.advance(MINUTE);
        let fired = tick(&mut s);
        assert_eq!(fired.is_some(), minute == 7, "minute {minute}: {fired:?}");
    }
}

/// Catalog `interface.autosave`: "with Rest Duration at 1 minute, a manual save followed by a
/// minute of no input still writes an autosave" (with the default 2 minutes, and then with
/// `file.autosave`'s 1), since a manual save restarts Max Duration only.
#[test]
fn a_manual_save_restarts_max_duration_only() {
    let state = folder("autosave-manual");
    let dir = folder("autosave-manual-files");
    let (mut s, clock) = clocked(&state);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    for _ in 0..9 {
        poke(&mut s);
        clock.advance(MINUTE);
        assert_eq!(tick(&mut s), None);
    }
    save(&mut s, json!({ "path": at(&dir, "scene.gss") }));
    for minute in 1..=9 {
        poke(&mut s);
        clock.advance(MINUTE);
        assert_eq!(tick(&mut s), None, "minute {minute} after the save");
    }
    save(&mut s, json!({}));
    clock.advance(2 * MINUTE);
    assert!(
        tick(&mut s).is_some(),
        "a save then two idle minutes still autosaves"
    );
    s.ok("file.autosave", json!({ "rest_minutes": 1 }));
    poke(&mut s);
    save(&mut s, json!({}));
    clock.advance(MINUTE);
    assert!(
        tick(&mut s).is_some(),
        "Rest Duration at 1: a save then one idle minute still autosaves"
    );
}

/// Catalog `interface.autosave`: "setting Max files to 3 and quick-saving scene A four times
/// and scene B four times leaves three copies of each". `file.autosave` sets it in one session,
/// and the sessions after it start with it, as it is kept in the state folder.
#[test]
fn max_files_3_keeps_the_newest_three_copies_of_each_scene() {
    let state = folder("autosave-keep");
    let dir = folder("autosave-keep-files");
    let (mut first, _) = clocked(&state);
    let set = first.ok("file.autosave", json!({ "keep": 3 })).result;
    let kept = json!({ "max_minutes": 10.0, "rest_minutes": 2.0, "keep": 3 });
    assert_eq!(set, kept, "the answer is every setting");
    drop(first);
    let mut folders = Vec::new();
    for name in ["a", "b"] {
        let (mut s, _) = clocked(&state);
        assert_eq!(s.query("file.autosave", json!({})), kept, "kept");
        s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        save(&mut s, json!({ "path": at(&dir, &format!("{name}.gss")) }));
        let mut last = String::new();
        for _ in 0..4 {
            last = s.ok("file.quick_save", json!({})).result["path"]
                .as_str()
                .unwrap()
                .into();
        }
        let copy = Path::new(&last);
        assert_eq!(
            copy.file_name().unwrap().to_str().unwrap(),
            format!("{name}-004.gss")
        );
        folders.push(copy.parent().unwrap().to_path_buf());
    }
    for dir in &folders {
        let mut names: Vec<String> = (std::fs::read_dir(dir).unwrap())
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let scene = &names[0][..1];
        let want: Vec<String> = (2..=4).map(|n| format!("{scene}-{n:03}.gss")).collect();
        assert_eq!(names, want, "{}", dir.display());
    }
}

/// `file.autosave` (decision 0016, section 17): `session.state` shows the settings, a duration
/// at the top of its range turns its own trigger off, a value out of range is refused naming
/// it, and a session with autosave off is refused and shows none.
#[test]
fn the_top_of_a_durations_range_turns_its_trigger_off() {
    let state = folder("autosave-settings");
    let (mut s, clock) = clocked(&state);
    let shown = |s: &mut Session| {
        let values = s.query("session.state", json!({}));
        ["max_minutes", "rest_minutes", "keep"].map(|k| values[format!("autosave.{k}")].clone())
    };
    let defaults = [json!(10.0), json!(2.0), json!(10)];
    assert_eq!(shown(&mut s), defaults, "the defaults");
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok(
        "file.autosave",
        json!({ "max_minutes": 60, "rest_minutes": 30 }),
    );
    let off = [json!(60.0), json!(30.0), json!(10)];
    assert_eq!(shown(&mut s), off, "both at the top");
    clock.advance(24 * 60 * MINUTE);
    assert_eq!(tick(&mut s), None, "a day later neither has fired");
    s.ok("file.autosave", json!({ "max_minutes": 10 }));
    assert!(tick(&mut s).is_some(), "Max Duration on again fires");
    let e = s.err("file.autosave", json!({ "keep": 101 }));
    let refused = (e.code, e.path.as_deref());
    assert_eq!(
        refused,
        (ErrorCode::ArgsInvalid, Some("/keep")),
        "{}",
        e.message
    );
    let mut none = Session::new();
    let e = none.err("file.autosave", json!({}));
    assert_eq!(
        e.code,
        ErrorCode::Precondition,
        "autosave off: {}",
        e.message
    );
    assert_eq!(
        shown(&mut none),
        [Value::Null, Value::Null, Value::Null],
        "none shown"
    );
}

#[test]
fn a_clean_stop_of_a_saved_scene_removes_its_autosaves_and_marker() {
    let state = folder("autosave-stop");
    let dir = folder("autosave-stop-files");
    let (mut s, _) = clocked(&state);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    save(&mut s, json!({ "path": at(&dir, "scene.gss") }));
    let copy = s.ok("file.quick_save", json!({})).result["path"]
        .as_str()
        .unwrap()
        .to_string();
    let marker = |state: &Path| std::fs::read_dir(state.join("sessions")).unwrap().count();
    assert_eq!(marker(&state), 1, "a running session is marked");
    s.engine.close();
    assert_eq!(marker(&state), 0, "a clean stop removes the marker");
    assert!(
        !Path::new(&copy).exists(),
        "and the copies the saved file has overtaken"
    );
}

/// Makes every session marked in `state` look crashed: its marker names a process that is gone.
pub fn mark_dead(state: &Path) {
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    for entry in std::fs::read_dir(state.join("sessions")).unwrap() {
        let path = entry.unwrap().path();
        let mut marker: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        marker["pid"] = pid.into();
        std::fs::write(&path, marker.to_string()).unwrap();
    }
}

/// Review 0003 M4 and s1: a recovery replays as its own client, so a `view.frame` on another
/// client's last edit point must carry the point, or the recovery stops at that line.
#[test]
fn a_recovery_replays_a_frame_on_another_clients_last_edit_point() {
    let dir = folder("recover-frame");
    let state = dir.join("state");
    let config = EngineConfig {
        session: "crashed".into(),
        log_path: Some(dir.join("crashed.jsonl")),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s.ok("file.quick_save", json!({}));
    s.origin = Origin {
        client_id: 9,
        client: "agent-a".into(),
        role: Role::Agent,
    };
    s.ok(
        "brush.stroke",
        stroke("Draw", [0.0, 0.0, 1.0], [0.1, 0.0, 1.0]),
    );
    s.ok("view.frame", json!({ "target": "last-edit" }));
    s.ok(
        "mesh.translate",
        json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 0.5 }),
    );
    let (hash, framed) = (s.hash(), s.query("view.get", json!({}))["camera"].clone());
    mark_dead(&state);
    let mut r = session(0, None, Some(&state));
    let id = r.query("file.recovery", json!({}))["candidates"][0]["id"].clone();
    let recovered = r.ok("file.recover", json!({ "session": id })).result;
    assert_eq!(recovered["replay_stopped"], Value::Null, "{recovered}");
    assert_eq!(r.hash(), hash, "the scene as it was at the last command");
    let view = r.query("view.get", json!({ "view": "agent:agent-a" }));
    assert_eq!(view["camera"], framed, "framed where the session framed it");
}

/// A recovery stops at the first log line that fails, keeps the scene as the lines before it
/// left it, and names that line.
#[test]
fn a_recovery_stops_at_the_first_line_that_fails() {
    let dir = folder("recover-stop");
    let (state, log) = (dir.join("state"), dir.join("crashed.jsonl"));
    let config = EngineConfig {
        session: "crashed".into(),
        log_path: Some(log.clone()),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({}));
    s.ok("file.quick_save", json!({}));
    let moved = json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 0.5 });
    s.ok("mesh.translate", moved);
    let hash = s.hash();
    // The same move again for a part the scene does not have, as an edited log could hold.
    let text = std::fs::read_to_string(&log).unwrap();
    let gone = text
        .lines()
        .last()
        .unwrap()
        .replace("\"sphere\"", "\"gone\"");
    std::fs::write(&log, format!("{text}{gone}\n")).unwrap();
    mark_dead(&state);
    let mut r = session(0, None, Some(&state));
    let candidate = r.query("file.recovery", json!({}))["candidates"][0].clone();
    let lines = candidate["lines_after"].as_u64().unwrap_or_default();
    let recovered = r
        .ok("file.recover", json!({ "session": candidate["id"] }))
        .result;
    assert_eq!(recovered["replayed"], json!(lines - 1), "{recovered}");
    let stopped = recovered["replay_stopped"].as_str().unwrap_or_default();
    let line = format!("line {lines} (mesh.translate): ");
    assert!(
        stopped.starts_with(&line) && stopped.contains("gone"),
        "{recovered}"
    );
    assert_eq!(r.hash(), hash, "the scene as the lines before it left it");
}

/// Review 0005, m6: a recovery removes the crashed session's marker, so the recovered session
/// autosaves at once, before either timer, and a second crash right after it still leaves the
/// whole scene to recover.
#[test]
fn a_second_crash_right_after_a_recovery_still_recovers() {
    let state = folder("recover-twice");
    let named = |session: &str| EngineConfig {
        session: session.into(),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut first = Session::with(named("first"), Role::Human);
    first.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    first.ok("file.quick_save", json!({}));
    let hash = first.hash();
    mark_dead(&state);

    let (mut second, _) = clocked(&state);
    let id = second.query("file.recovery", json!({}))["candidates"][0]["id"].clone();
    second.ok("file.recover", json!({ "session": id }));
    let copy = tick(&mut second).expect("an autosave with the clock not moved");
    assert!(copy.ends_with("-001.gss"), "{copy}");
    mark_dead(&state);

    let mut third = Session::with(named("third"), Role::Human);
    let found = third.query("file.recovery", json!({}));
    let [candidate] = <[Value; 1]>::try_from(found["candidates"].as_array().unwrap().clone())
        .expect("the recovered session, crashed");
    assert_eq!(candidate["autosave"], json!(copy), "{candidate}");
    third.ok("file.recover", json!({ "session": candidate["id"] }));
    assert_eq!(third.hash(), hash, "the scene the first session had");
}

/// Decision 0029, section 9: Isolate is saved in the session section and comes back the same
/// through `file.open` and through a recovery, which open a file the same way. So does the
/// active part (decision 0033): the cube, where a file without one would make the sphere
/// active, and `part.list` marks it.
#[test]
fn isolate_comes_back_through_open_and_recovery() {
    let state = folder("recover-isolate");
    let config = EngineConfig {
        session: "isolated".into(),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("primitive.cube", json!({}));
    s.ok("part.isolate", json!({ "parts": ["cube"] }));
    let copy = s.ok("file.quick_save", json!({})).result["path"].clone();
    let list = s.query("part.list", json!({}));
    mark_dead(&state);
    let mut opened = Session::new();
    opened.ok("file.open", json!({ "path": copy }));
    let mut recovered = session(0, None, Some(&state));
    let id = recovered.query("file.recovery", json!({}))["candidates"][0]["id"].clone();
    recovered.ok("file.recover", json!({ "session": id }));
    for (how, s) in [("opened", &mut opened), ("recovered", &mut recovered)] {
        assert_eq!(
            s.query("part.list", json!({})),
            list,
            "{how}: the cube alone shows"
        );
        let isolated = &s.query("session.state", json!({}))["parts.isolated"];
        assert_eq!(*isolated, json!(true), "{how}");
    }
}
