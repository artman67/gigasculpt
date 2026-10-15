//! New, Revert, Save Next and the recent files (decision 0016, section 16), through commands;
//! Save Next for a Part file and the low disk space warning (section 17, items 8 and 9).

use gs_engine::{EngineConfig, FreeSpace};
use gs_protocol::{CommandError, LogRecord, Outcome, Request, Role, RunParams};
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::{at, folder, save, session, stroke};

/// The error code `command` is refused with.
fn refused(s: &mut Session, command: &str, args: Value) -> String {
    code(s.err(command, args).code)
}

#[test]
fn new_refuses_unsaved_changes_and_with_discard_starts_an_empty_scene() {
    let dir = folder("new");
    let mut s = session(1, None, None);
    let empty = s.hash();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    save(&mut s, json!({ "path": at(&dir, "scene.gss") }));
    s.ok("primitive.cube", json!({}));
    assert_eq!(refused(&mut s, "file.new", json!({})), "E_PRECONDITION");
    let new = s.query("file.new", json!({ "discard_changes": true }));
    assert_eq!(new, json!({ "name": "untitled", "discarded": true }));
    assert_eq!(s.hash(), empty, "nothing is left");
    assert_eq!(s.history()["steps"], json!([]), "nor any history");
    assert_eq!(
        refused(&mut s, "file.save", json!({})),
        "E_PRECONDITION",
        "the new scene has no file"
    );
    let again = s.query("file.new", json!({}));
    assert_eq!(
        again["discarded"],
        json!(false),
        "an empty scene has nothing to lose"
    );
}

/// What a dry run of `command` says: the error, if it is refused.
fn dry_run(s: &mut Session, command: &str, args: Value) -> Option<CommandError> {
    let request = Request::Run(RunParams::new(command, args).dry());
    match s.request(request).outcome {
        Outcome::Ok(_) => None,
        Outcome::Error(e) => Some(e),
    }
}

/// Open and New on unsaved changes are refused naming the argument that lets them go, so the
/// window asks for it and a script knows what to pass (decision 0016, section 17; R6).
#[test]
fn open_and_new_on_unsaved_changes_name_the_argument_that_lets_them_go() {
    let dir = folder("unsaved-names");
    let other = at(&dir, "other.gss");
    let mut s = session(1, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    save(&mut s, json!({ "path": other }));
    s.ok("primitive.cube", json!({}));
    for (command, args) in [
        ("file.open", json!({ "path": other })),
        ("file.new", json!({})),
    ] {
        let e = s.err(command, args.clone());
        assert_eq!(code(e.code), "E_PRECONDITION", "{command}");
        assert_eq!(e.path.as_deref(), Some("/discard_changes"), "{command}");
        assert!(e.message.contains("unsaved changes"), "{}", e.message);
        let dry = dry_run(&mut s, command, args).map(|e| e.path);
        assert_eq!(dry, Some(Some("/discard_changes".into())), "{command}");
    }
    s.query(
        "file.open",
        json!({ "path": other, "discard_changes": true }),
    );
    assert!(dry_run(&mut s, "file.new", json!({})).is_none(), "saved");
}

/// A save into a folder GigaSculpt may not write, as a shared or a system folder is, says so in
/// a person's words naming the file; never the system's text with its error number.
#[test]
#[cfg(unix)]
fn a_save_it_may_not_write_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let dir = folder("save-not-allowed");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut s = session(1, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let e = s.err("file.save", json!({ "path": at(&dir, "shut.gss") }));
    assert_eq!(code(e.code), "E_PRECONDITION", "{}", e.message);
    for words in ["shut.gss", "not allowed to write"] {
        assert!(e.message.contains(words), "{}", e.message);
    }
    for system in ["os error", "Permission denied"] {
        assert!(!e.message.contains(system), "{}", e.message);
    }
}

/// Opening a scene file GigaSculpt may not read, as a shared one can be, says so in a person's
/// words naming the file, as a save does.
#[test]
#[cfg(unix)]
fn an_open_it_may_not_read_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let path = at(&folder("open-not-allowed"), "shut.gss");
    let mut s = session(1, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("file.save", json!({ "path": path }));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let e = s.err(
        "file.open",
        json!({ "path": path, "discard_changes": true }),
    );
    assert_eq!(code(e.code), "E_PRECONDITION", "{}", e.message);
    for words in ["shut.gss", "not allowed to read"] {
        assert!(e.message.contains(words), "{}", e.message);
    }
    for system in ["os error", "Permission denied"] {
        assert!(!e.message.contains(system), "{}", e.message);
    }
}

/// `file.save {}` on a scene that has no file is refused before it starts a job, naming
/// `path`, so a dry run says so and the window asks a name; the first save with a path and the
/// next without write the same file (decision 0016, section 17; R6).
#[test]
fn a_first_save_without_a_path_is_refused_before_it_runs_naming_path() {
    let dir = folder("first-save");
    let path = at(&dir, "first.gss");
    let mut s = session(1, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let dry = dry_run(&mut s, "file.save", json!({})).unwrap_or_else(|| {
        panic!("a dry run of a first save without a path is refused");
    });
    assert_eq!(code(dry.code), "E_PRECONDITION");
    assert_eq!(dry.path.as_deref(), Some("/path"));
    assert_eq!(dry.hint.as_deref(), Some("give a path ending in .gss"));
    let e = s.err("file.save", json!({}));
    assert_eq!(
        (e.message.as_str(), e.path),
        (dry.message.as_str(), dry.path)
    );
    assert!(!std::path::Path::new(&path).exists(), "nothing is written");
    let first = s.query("file.save", json!({ "path": path }));
    s.ok("primitive.cube", json!({}));
    assert!(dry_run(&mut s, "file.save", json!({})).is_none());
    let second = s.query("file.save", json!({}));
    assert_eq!(
        second["path"], first["path"],
        "the same file, with no path given"
    );
}

#[test]
fn revert_brings_back_the_saved_file_and_logs_as_its_open() {
    let dir = folder("revert");
    let (path, log) = (at(&dir, "scene.gss"), dir.join("session.jsonl"));
    let mut s = session(3, Some(log.clone()), None);
    s.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    assert_eq!(
        refused(&mut s, "file.revert", json!({})),
        "E_PRECONDITION",
        "no file yet"
    );
    save(&mut s, json!({ "path": path }));
    let (saved, bytes) = (s.hash(), std::fs::read(&path).unwrap());
    s.ok(
        "brush.stroke",
        stroke("Draw", [-0.3, 0.0, 1.0], [0.3, 0.2, 1.0]),
    );
    assert_ne!(s.hash(), saved);
    let back = s.query("file.revert", json!({}));
    let last = std::fs::read_to_string(&log).unwrap();
    let last: LogRecord = serde_json::from_str(last.lines().last().unwrap()).unwrap();
    assert_eq!(back["file"], json!(path));
    assert_eq!(s.hash(), saved, "the scene is the file's again");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "the file is untouched"
    );
    let hash = blake3::hash(&bytes).to_hex().to_string();
    assert_eq!(
        last.command, "file.open",
        "logged as the open it amounts to"
    );
    assert_eq!(
        last.args,
        json!({ "path": path, "discard_changes": true, "hash": hash })
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(refused(&mut s, "file.revert", json!({})), "E_NOT_FOUND");
}

#[test]
fn save_next_takes_the_next_free_number_and_never_overwrites() {
    let dir = folder("next");
    let mut s = session(4, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    save(&mut s, json!({ "path": at(&dir, "head.gss") }));
    let first = s.query("file.save_next", json!({}));
    assert_eq!(
        first["path"],
        json!(at(&dir, "head_001.gss")),
        "a name without a number"
    );
    let taken = at(&dir, "head_002.gss");
    std::fs::write(&taken, b"someone else's file").unwrap();
    s.ok("primitive.cube", json!({}));
    let second = s.query("file.save_next", json!({}));
    assert_eq!(
        second["path"],
        json!(at(&dir, "head_003.gss")),
        "a taken name is passed over"
    );
    assert_eq!(std::fs::read(&taken).unwrap(), b"someone else's file");
    assert_eq!(
        save(&mut s, json!({}))["path"],
        json!(at(&dir, "head_003.gss")),
        "the scene carries on in its new file"
    );
    save(&mut s, json!({ "path": at(&dir, "take9.gss") }));
    let wider = s.query("file.save_next", json!({}));
    assert_eq!(wider["path"], json!(at(&dir, "take10.gss")));
}

#[test]
fn recent_files_come_newest_first_bounded_and_kept_for_the_next_session() {
    let dir = folder("recent");
    let state = dir.join("state");
    let listed = |s: &mut Session| -> Vec<(String, bool)> {
        let files = s.query("file.recent", json!({}))["files"].clone();
        (files.as_array().into_iter().flatten())
            .map(|f| (f["name"].as_str().unwrap().to_string(), f["gone"] == true))
            .collect()
    };
    let names = |range: std::ops::RangeInclusive<u32>| -> Vec<(String, bool)> {
        range
            .rev()
            .map(|i| (format!("scene{i:02}"), false))
            .collect()
    };
    let mut off = session(5, None, None);
    assert_eq!(
        refused(&mut off, "file.recent", json!({})),
        "E_PRECONDITION"
    );
    let mut s = session(5, None, Some(&state));
    assert_eq!(listed(&mut s), Vec::<(String, bool)>::new(), "none yet");
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    for i in 0..12 {
        save(
            &mut s,
            json!({ "path": at(&dir, &format!("scene{i:02}.gss")) }),
        );
    }
    assert_eq!(listed(&mut s), names(2..=11), "the newest ten");
    // A Part file and a quick save are not the scene's file: the list stays.
    save(
        &mut s,
        json!({ "path": at(&dir, "piece.gss"), "parts": ["sphere"] }),
    );
    s.ok("file.quick_save", json!({}));
    assert_eq!(listed(&mut s), names(2..=11));
    let five = at(&dir, "scene05.gss");
    s.ok("file.open", json!({ "path": five }));
    let mut moved = names(2..=11);
    moved.retain(|(n, _)| n != "scene05");
    moved.insert(0, ("scene05".into(), false));
    assert_eq!(listed(&mut s), moved, "an open moves its file up");
    std::fs::remove_file(at(&dir, "scene11.gss")).unwrap();
    moved[1].1 = true;
    let mut next = session(6, None, Some(&state));
    assert_eq!(
        listed(&mut next),
        moved,
        "the next session sees the list, a gone file marked"
    );
    assert_eq!(
        refused(&mut next, "file.open", json!({ "recent": 2 })),
        "E_NOT_FOUND"
    );
    assert_eq!(
        refused(&mut next, "file.open", json!({ "recent": 11 })),
        "E_NOT_FOUND"
    );
    let opened = next.query("file.open", json!({ "recent": 1 }));
    assert_eq!(opened["path"], json!(five));
    assert_eq!(next.hash(), s.hash(), "and its scene came in");
}

/// Catalog `interface.scene-files`: "Save and Save Next work the same for Part files (selected
/// Part or all Parts)" (decision 0016, section 17, item 8). The first counts from the path
/// given, the next from the Part file saved last; a taken name is passed over; the scene keeps
/// its own file.
#[test]
fn save_next_of_a_part_file_takes_the_next_free_number_and_never_overwrites() {
    let dir = folder("next-part");
    let mut s = session(7, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("primitive.cube", json!({}));
    let scene = at(&dir, "scene.gss");
    save(&mut s, json!({ "path": scene }));
    let selected = s.query("session.state", json!({}))["parts.active"].clone();
    let active = json!({ "parts": ["@active"] });
    let e = s.err("file.save_next", active.clone());
    assert_eq!(
        (code(e.code).as_str(), e.path.as_deref()),
        ("E_PRECONDITION", Some("/path")),
        "no Part file yet: it asks where to count from ({})",
        e.message
    );
    let first = json!({ "parts": ["@active"], "path": at(&dir, "arm.gss") });
    let first = s.query("file.save_next", first);
    assert_eq!(first["path"], json!(at(&dir, "arm_001.gss")));
    assert_eq!(
        (&first["kind"], &first["parts"]),
        (&json!("parts"), &json!([selected])),
        "the selected Part, as a Part file"
    );
    let taken = at(&dir, "arm_002.gss");
    std::fs::write(&taken, b"someone else's file").unwrap();
    let second = s.query("file.save_next", active);
    assert_eq!(
        second["path"],
        json!(at(&dir, "arm_003.gss")),
        "from the Part file saved last, past a taken name"
    );
    assert_eq!(std::fs::read(&taken).unwrap(), b"someone else's file");
    let all = json!({ "parts": ["sphere", "cube"] });
    let all = s.query("file.save_next", all);
    assert_eq!(
        (&all["path"], &all["parts"]),
        (&json!(at(&dir, "arm_004.gss")), &json!(["sphere", "cube"])),
        "all the Parts"
    );
    let state = s.query("session.state", json!({}));
    assert_eq!(state["file.path"], json!(scene), "the scene keeps its file");
}

/// Catalog `interface.scene-files`: "low disk space warns clearly and keeps warning until
/// freed" (decision 0016, section 17, item 9). Under the threshold (1 GiB here) `file.low_disk`
/// names the scene's folder and a save still runs; with less than the file needs, a save is
/// refused saying so; once space returns the warning goes at the next call.
#[test]
fn low_disk_space_warns_under_the_threshold_until_space_returns() {
    let dir = folder("low-disk");
    let free = FreeSpace::fixed(64 << 30);
    let config = EngineConfig {
        free_space: free.clone(),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    let low = |s: &mut Session| s.query("session.state", json!({}))["file.low_disk"].clone();
    s.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    free.set(1 << 20);
    assert_eq!(
        low(&mut s),
        Value::Null,
        "no file and no autosave: no folder to watch"
    );
    free.set(64 << 30);
    let path = at(&dir, "scene.gss");
    save(&mut s, json!({ "path": path }));
    assert_eq!(low(&mut s), Value::Null, "room to spare");
    free.set(300 << 20);
    let warned = low(&mut s);
    let folder = dir.display().to_string();
    assert_eq!(
        (&warned["folder"], &warned["free"], &warned["threshold"]),
        (&json!(folder), &json!(300 << 20), &json!(1 << 30)),
        "under 1 GiB: {warned}"
    );
    s.ok("primitive.cube", json!({}));
    assert_eq!(
        save(&mut s, json!({}))["path"],
        json!(path),
        "a save still runs"
    );
    assert_eq!(
        low(&mut s)["free"],
        json!(300 << 20),
        "and the warning stays"
    );
    free.set(1000);
    let e = s.err("file.save", json!({}));
    assert_eq!(code(e.code), "E_PRECONDITION");
    assert!(e.message.contains("MB free"), "{}", e.message);
    free.set(64 << 30);
    assert_eq!(low(&mut s), Value::Null, "freed: the warning goes");
}
