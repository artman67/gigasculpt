//! Save then open: the same scene hash, the same numbering and properties, the same result for
//! the next commands, random ones included; the same bytes from two sessions that saved the
//! same scene; Part files; and what a save or an open refuses. Parts with detail levels are in
//! `levels.rs`.

use gs_protocol::LogRecord;
use gs_testkit::code;
use serde_json::json;

use crate::{at, build, folder, next, save, session};

#[test]
fn a_saved_scene_reopens_with_the_same_hash_and_the_same_next_commands() {
    let dir = folder("roundtrip");
    let path = at(&dir, "scene.gss");
    let mut a = session(7, None, None);
    build(&mut a, &dir);
    let saved = save(&mut a, json!({ "path": path }));
    assert_eq!(
        saved["parts"],
        json!(["sphere", "painted"]),
        "in the part list's order"
    );
    assert_eq!(saved["kind"], json!("scene"));
    // Every call takes a command number, and random commands draw from it, so both sessions
    // make the same calls between the save or open and the next commands.
    let painted = json!({ "part": "painted" });
    let (hash, part) = (a.hash(), a.query("mesh.hash", painted.clone()));
    let steps = a.history()["steps"].as_array().map_or(0, Vec::len);
    assert!(steps > 0, "the first session has history");
    let after = next(&mut a);
    let export_a = at(&dir, "a.obj");
    a.ok(
        "file.export",
        json!({ "path": export_a, "part": "painted" }),
    );

    let mut b = session(99, None, None);
    let opened = b.ok("file.open", json!({ "path": path })).result;
    assert_eq!(opened["refused"], json!([]), "every part opens");
    assert_eq!(opened["file"], json!(path), "the scene belongs to the file");
    assert_eq!(b.hash(), hash, "the same scene, bit for bit");
    let numbered = b.query("mesh.hash", painted);
    assert_eq!(numbered, part, "the part in its own numbering");
    assert_eq!(
        b.history()["steps"],
        json!([]),
        "the history starts empty after an open"
    );
    assert_eq!(
        next(&mut b),
        after,
        "the next stroke and the next random jitter match"
    );
    let export_b = at(&dir, "b.obj");
    b.ok(
        "file.export",
        json!({ "path": export_b, "part": "painted" }),
    );
    let (ea, eb) = (
        std::fs::read(export_a).unwrap(),
        std::fs::read(export_b).unwrap(),
    );
    assert!(ea == eb, "the part's export scale came back too");
}

#[test]
fn two_sessions_saving_the_same_scene_write_the_same_bytes() {
    let dir = folder("bytes");
    let path = at(&dir, "scene.gss");
    let save_once = || {
        let mut s = session(3, None, None);
        build(&mut s, &dir);
        save(&mut s, json!({ "path": path }));
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        bytes
    };
    assert!(save_once() == save_once(), "byte-identical files");
}

#[test]
fn a_part_file_holds_only_its_parts() {
    let dir = folder("parts");
    let path = at(&dir, "painted.gss");
    let mut a = session(1, None, None);
    build(&mut a, &dir);
    let saved = save(&mut a, json!({ "path": path, "parts": ["painted"] }));
    assert_eq!(
        (saved["kind"].clone(), saved["parts"].clone()),
        (json!("parts"), json!(["painted"]))
    );
    let own = a.err("file.save", json!({}));
    assert_eq!(
        code(own.code),
        "E_PRECONDITION",
        "a Part file is not the scene's own file"
    );
    let part = a.query("mesh.hash", json!({ "part": "painted" }));
    let mut b = session(1, None, None);
    let opened = b.ok("file.open", json!({ "path": path })).result;
    assert_eq!(
        (opened["kind"].clone(), opened["parts"].clone()),
        (json!("parts"), json!(["painted"]))
    );
    assert_eq!(
        opened["file"],
        json!(null),
        "opening a Part file gives a scene with no file"
    );
    assert_eq!(b.query("mesh.hash", json!({ "part": "painted" })), part);
}

#[test]
fn saves_and_opens_refuse_what_would_lose_work() {
    let dir = folder("refusals");
    let (path, other) = (at(&dir, "scene.gss"), at(&dir, "other.gss"));
    let mut s = session(1, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let refuse = |s: &mut gs_testkit::Session, command: &str, args| code(s.err(command, args).code);
    assert_eq!(
        refuse(&mut s, "file.save", json!({})),
        "E_PRECONDITION",
        "no file yet"
    );
    assert_eq!(
        refuse(
            &mut s,
            "file.save",
            json!({ "path": at(&dir, "scene.obj") })
        ),
        "E_ARGS_INVALID"
    );
    save(&mut s, json!({ "path": path }));
    std::fs::write(&other, b"someone else's file").unwrap();
    assert_eq!(
        refuse(&mut s, "file.save", json!({ "path": other })),
        "E_PRECONDITION",
        "not ours"
    );
    s.ok("primitive.cube", json!({}));
    let again = save(&mut s, json!({}));
    assert_eq!(again["path"], json!(path), "a scene saves to its own file");
    let backup = at(&dir, "scene.backup.gss");
    assert_eq!(
        again["backup"],
        json!(backup),
        "and keeps the one it replaced"
    );
    s.ok(
        "mesh.translate",
        json!({ "part": "cube", "direction": [1, 0, 0], "distance": 1 }),
    );
    assert_eq!(
        refuse(&mut s, "file.open", json!({ "path": path })),
        "E_PRECONDITION",
        "unsaved"
    );
    assert_eq!(
        refuse(
            &mut s,
            "file.open",
            json!({ "path": other, "discard_changes": true })
        ),
        "E_PRECONDITION",
        "not a scene file"
    );
    let opened = s
        .ok(
            "file.open",
            json!({ "path": backup, "discard_changes": true }),
        )
        .result;
    assert_eq!(
        opened["parts"],
        json!(["sphere"]),
        "the backup is the first save"
    );
}

#[test]
fn a_damaged_part_is_refused_by_name_and_the_rest_open() {
    let dir = folder("damaged");
    let (path, damaged) = (at(&dir, "scene.gss"), at(&dir, "damaged.gss"));
    let mut s = session(1, None, None);
    for (name, x) in [("a", -3.0), ("b", 0.0), ("c", 3.0)] {
        s.ok(
            "primitive.sphere",
            json!({ "name": name, "subdivisions": 6, "center": [x, 0, 0] }),
        );
    }
    save(&mut s, json!({ "path": path }));
    let b = s.query("mesh.hash", json!({ "part": "b" }));
    let mut bytes = std::fs::read(&path).unwrap();
    // The first chunk, right after the 64-byte header, is part a's positions.
    bytes[64 + 10] ^= 0x10;
    std::fs::write(&damaged, &bytes).unwrap();
    let mut t = session(1, None, None);
    let opened = t.ok("file.open", json!({ "path": damaged })).result;
    assert_eq!(opened["parts"], json!(["b", "c"]), "the others open");
    assert_eq!(opened["refused"][0]["part"], json!("a"), "{opened}");
    let reason = opened["refused"][0]["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("positions") && reason.contains("damaged"),
        "{reason}"
    );
    assert_eq!(
        opened["file"],
        json!(null),
        "a plain save cannot overwrite the damaged file"
    );
    assert_eq!(t.query("mesh.hash", json!({ "part": "b" })), b);
    let seen = t.query("file.inspect", json!({ "path": damaged, "verify": true }));
    assert!(
        seen["damaged"][0]
            .as_str()
            .unwrap_or_default()
            .contains("part 'a'"),
        "{seen}"
    );
    std::fs::write(&damaged, &bytes[..bytes.len() - 1]).unwrap();
    let cut = t.err(
        "file.open",
        json!({ "path": damaged, "discard_changes": true }),
    );
    assert_eq!(code(cut.code), "E_DAMAGED", "{}", cut.message);
}

#[test]
fn an_open_starts_a_new_log_run_with_the_files_hash_pinned() {
    let dir = folder("log");
    let path = at(&dir, "scene.gss");
    let mut a = session(2, None, None);
    a.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    save(&mut a, json!({ "path": path }));
    let log = dir.join("session.jsonl");
    let mut b = session(2, Some(log.clone()), None);
    b.ok("primitive.cube", json!({}));
    b.ok(
        "file.open",
        json!({ "path": path, "discard_changes": true }),
    );
    b.ok(
        "mesh.translate",
        json!({ "part": "sphere", "direction": [0, 1, 0], "distance": 1 }),
    );
    let lines: Vec<String> = (std::fs::read_to_string(&log).unwrap().lines())
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 3, "a header, the open and the move: {lines:?}");
    let open: LogRecord = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(open.command, "file.open");
    let hash = blake3::hash(&std::fs::read(&path).unwrap())
        .to_hex()
        .to_string();
    assert_eq!(
        open.args["hash"],
        json!(hash),
        "the file's content is pinned"
    );
    let kept = std::fs::read_to_string(dir.join("session-1.jsonl")).unwrap();
    assert!(
        kept.contains("primitive.cube"),
        "the run before the open is kept"
    );
}
