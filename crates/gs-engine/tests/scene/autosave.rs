//! Autosave with a clock moved by hand: Max Duration and Rest Duration fire as the catalog's
//! `interface.autosave` says, a manual save restarts Max Duration only, `file.autosave`'s
//! settings last across sessions and Max files keeps that many copies of each scene, a clean
//! stop removes the copies a saved scene no longer needs, a recovery replays what several
//! clients did after the last copy, and a recovered session autosaves at once.

use std::path::{Path, PathBuf};
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
    let files = |state: &Path| std::fs::read_dir(state.join("sessions")).unwrap().count();
    assert_eq!(
        files(&state),
        2,
        "a running session is marked, beside the lock it holds"
    );
    s.engine.close();
    assert_eq!(
        files(&state),
        0,
        "a clean stop removes the marker and its lock"
    );
    assert!(
        !Path::new(&copy).exists(),
        "and the copies the saved file has overtaken"
    );
}

/// Makes every session marked in `state` look crashed while it runs on: each marker moves to
/// the name of a process that is gone, beside which no running session holds a lock.
pub fn mark_dead(state: &Path) {
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    for entry in std::fs::read_dir(state.join("sessions")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let mut marker: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        marker["pid"] = pid.into();
        let moved = format!("{}-{pid}.json", marker["session"].as_str().unwrap());
        std::fs::write(path.with_file_name(moved), marker.to_string()).unwrap();
        std::fs::remove_file(&path).unwrap();
    }
}

/// The candidates `s` is offered, by id.
fn offered(s: &mut Session) -> Vec<String> {
    let found = s.query("file.recovery", json!({}));
    let all = found["candidates"].as_array().cloned().unwrap_or_default();
    all.iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

/// Session `name`'s settings, autosaving into `state`.
fn named(state: &Path, name: &str) -> EngineConfig {
    EngineConfig {
        session: name.into(),
        autosave: Some(StateFolder::in_folder(state.to_path_buf())),
        ..EngineConfig::default()
    }
}

/// Session `name`, autosaving into `state`, with a sphere it has autosaved.
fn autosaved(state: &Path, name: &str) -> Session {
    let mut s = Session::with(named(state, name), Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("file.quick_save", json!({}));
    s
}

/// A second session that still runs is not offered as crashed work, however its process looks
/// from outside: it holds the lock beside its marker until it stops. A clean stop takes the
/// marker away, so it is not offered then either.
#[test]
fn a_session_that_still_runs_is_not_offered_as_crashed() {
    let state = folder("recover-running");
    let mut first = autosaved(&state, "first");
    let mut second = Session::with(named(&state, "second"), Role::Human);
    assert_eq!(offered(&mut second), Vec::<String>::new(), "first runs");
    first.engine.close();
    assert_eq!(
        offered(&mut second),
        Vec::<String>::new(),
        "first stopped cleanly"
    );
}

/// A session that stopped without closing is offered, even when its process number names a
/// running program: here this one, as a program started after a reboot can be given the number
/// a crashed one had. The lock it held went with it; a process number can be handed on.
#[test]
fn a_session_that_stopped_is_offered_even_when_its_process_number_runs() {
    let state = folder("recover-reused");
    let mut first = autosaved(&state, "first");
    let hash = first.hash();
    drop(first);
    let mut second = Session::with(named(&state, "second"), Role::Human);
    let id = format!("first-{}", std::process::id());
    assert_eq!(offered(&mut second), [id.as_str()], "first stopped");
    second.ok("file.recover", json!({ "session": id }));
    assert_eq!(second.hash(), hash, "its scene comes back");
    assert_eq!(
        offered(&mut second),
        Vec::<String>::new(),
        "and is offered no more"
    );
}

/// A new session given a stopped one's name and process number (a reboot can hand both on)
/// takes neither its marker's name nor its standard log's: the stopped one stays offered, with
/// the line it logged after its autosave.
#[test]
fn a_new_session_with_a_stopped_ones_name_and_process_number_leaves_its_marker_and_log() {
    let state = folder("recover-same-name");
    let logged = || named(&state, "first").with_standard_log(state.clone());
    let mut first = Session::with(logged(), Role::Human);
    first.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    first.ok("file.quick_save", json!({}));
    let up = json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 0.5 });
    first.ok("mesh.translate", up);
    drop(first);
    let mut again = Session::with(logged(), Role::Human);
    let found = again.query("file.recovery", json!({}));
    let id = format!("first-{}", std::process::id());
    assert_eq!(
        found["candidates"]
            .as_array()
            .map(|c| c.iter().map(|c| &c["id"]).collect()),
        Some(vec![&json!(id)]),
        "the stopped one: {found}"
    );
    let lines = &found["candidates"][0]["lines_after"];
    assert_eq!(*lines, json!(1), "its own line after the autosave: {found}");
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

/// The command log is there but cannot be read (a folder stands at its path, which no system
/// reads as a file): the offer is the newest autosave alone and says why in a person's words,
/// naming the log (gs-engine `file::file_refused`; the system's reason, "Is a directory (os
/// error 21)" on Linux, by its kind), and a replay asked for anyway is refused with the same
/// reason.
#[test]
fn an_unreadable_log_is_offered_as_the_autosave_alone_saying_why() {
    let dir = folder("recover-unreadable");
    let (state, log) = (dir.join("state"), dir.join("crashed.jsonl"));
    let config = EngineConfig {
        session: "crashed".into(),
        log_path: Some(log.clone()),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("file.quick_save", json!({}));
    drop(s);
    std::fs::remove_file(&log).unwrap();
    std::fs::create_dir(&log).unwrap();
    let mut r = session(0, None, Some(&state));
    let candidate = r.query("file.recovery", json!({}))["candidates"][0].clone();
    let offer = candidate["offer"].as_str().unwrap_or_default();
    let alone = "the newest autosave alone: the command log after it cannot be read: ";
    let reason = offer.strip_prefix(alone).unwrap_or_default();
    // A folder on Linux and macOS; on Windows, a file it may not read.
    let worded = ["crashed.jsonl\" is a folder", "not allowed to read"];
    assert!(
        worded.iter().any(|w| reason.contains(w)) && !reason.contains("os error"),
        "the offer says why in a person's words: {candidate}"
    );
    assert_eq!(candidate["lines_after"], Value::Null, "{candidate}");
    let asked = json!({ "session": candidate["id"], "replay": true });
    let e = r.err("file.recover", asked);
    assert!(
        e.code == ErrorCode::Precondition && e.message.contains(reason),
        "the refusal gives {reason:?}: {}",
        e.message
    );
}

/// A session logging to `crashed.jsonl` in a new folder `name` that autosaved a sphere, moved
/// it twice and crashed: its state folder, its log, and the scene's hash after the first move.
fn moved_twice_and_crashed(name: &str) -> (PathBuf, PathBuf, String) {
    let dir = folder(name);
    let (state, log) = (dir.join("state"), dir.join("crashed.jsonl"));
    let config = EngineConfig {
        session: "crashed".into(),
        log_path: Some(log.clone()),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("file.quick_save", json!({}));
    let up = json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 0.5 });
    s.ok("mesh.translate", up.clone());
    let hash = s.hash();
    s.ok("mesh.translate", up);
    drop(s);
    (state, log, hash)
}

/// Cuts line `from_end` of `log` (0 is the last) to its first half, as a crash in the middle
/// of writing it would, and ends the file with a newline or without one.
fn halve_line(log: &Path, from_end: usize, newline: bool) {
    let text = std::fs::read_to_string(log).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    let i = lines.len() - 1 - from_end;
    let line = lines[i];
    lines[i] = &line[..line.len() / 2];
    let end = if newline { "\n" } else { "" };
    std::fs::write(log, lines.join("\n") + end).unwrap();
}

/// A crash or power loss in the middle of writing the last record leaves it cut off with no
/// newline. Recovery drops that line alone, as a write-ahead log drops a torn last record
/// (decision 0016, section 14): every whole record before it replays, and the offer says the
/// last command was cut off.
#[test]
fn a_log_whose_last_record_was_cut_off_recovers_every_whole_record_before_it() {
    let (state, log, hash) = moved_twice_and_crashed("recover-torn");
    halve_line(&log, 0, false);
    let mut r = session(0, None, Some(&state));
    let candidate = r.query("file.recovery", json!({}))["candidates"][0].clone();
    assert_eq!(candidate["lines_after"], json!(1), "{candidate}");
    assert_eq!(candidate["replayable"], json!(true), "{candidate}");
    let offer = candidate["offer"].as_str().unwrap_or_default();
    assert!(
        offer.starts_with("the newest autosave plus the 1 commands after it")
            && offer.contains("the last command was cut off"),
        "{candidate}"
    );
    let asked = json!({ "session": candidate["id"] });
    let recovered = r.ok("file.recover", asked).result;
    assert_eq!(recovered["replayed"], json!(1), "{recovered}");
    assert_eq!(recovered["replay_stopped"], Value::Null, "{recovered}");
    assert_eq!(r.hash(), hash, "the scene after the last whole command");
}

/// Only a last line without its newline counts as cut off by a crash: a line that will not
/// parse anywhere else, before the last or ending in its newline, still makes the log
/// unreadable, and the offer is the autosave alone, naming that line.
#[test]
fn a_line_that_will_not_parse_elsewhere_still_makes_the_log_unreadable() {
    for (case, from_end, newline) in [("middle", 1, true), ("last-ended", 0, true)] {
        let (state, log, _) = moved_twice_and_crashed(&format!("recover-bad-{case}"));
        halve_line(&log, from_end, newline);
        let number = std::fs::read_to_string(&log).unwrap().lines().count() - from_end;
        let mut r = session(0, None, Some(&state));
        let candidate = r.query("file.recovery", json!({}))["candidates"][0].clone();
        let offer = candidate["offer"].as_str().unwrap_or_default();
        let alone = "the newest autosave alone: the command log after it cannot be read";
        assert!(
            offer.starts_with(alone) && offer.contains(&format!("line {number}: ")),
            "{case}: {candidate}"
        );
        assert_eq!(candidate["lines_after"], Value::Null, "{case}: {candidate}");
    }
}

/// Review 0005, m6: a recovery removes the crashed session's marker, so the recovered session
/// autosaves at once, before either timer, and a second crash right after it still leaves the
/// whole scene to recover.
#[test]
fn a_second_crash_right_after_a_recovery_still_recovers() {
    let state = folder("recover-twice");
    let mut first = autosaved(&state, "first");
    let hash = first.hash();
    mark_dead(&state);

    let (mut second, _) = clocked(&state);
    let id = second.query("file.recovery", json!({}))["candidates"][0]["id"].clone();
    second.ok("file.recover", json!({ "session": id }));
    let copy = tick(&mut second).expect("an autosave with the clock not moved");
    assert!(copy.ends_with("-001.gss"), "{copy}");
    mark_dead(&state);

    let mut third = Session::with(named(&state, "third"), Role::Human);
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
