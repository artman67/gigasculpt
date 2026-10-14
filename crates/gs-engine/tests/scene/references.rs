//! Reference Views in scene files (decision 0024, section 8): a save keeps every open
//! reference's source and setup and which one is active, an open brings them back in place of
//! the session's, and none of it touches the scene, its hash or its unsaved changes. A file
//! reference whose file went missing or changed opens empty with its setup kept and the reason.

use std::path::Path;

use gs_io::scene::Kept;
use gs_protocol::ScriptLine;
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::{OBJ, at, folder, save, session};

/// Every open reference as `view.get` reports it (what a save keeps and an open brings back:
/// camera, size, lock and setup), and the active one.
fn references(s: &mut Session) -> Value {
    let list = s.query("reference.list", json!({}));
    let open: Vec<Value> = (list.as_array().unwrap().iter())
        .map(|r| {
            let v = s.query("view.get", json!({ "view": r["view"] }));
            let keys = [
                "view",
                "camera",
                "framed",
                "viewport",
                "locked",
                "reference",
            ];
            keys.iter()
                .map(|k| ((*k).to_string(), v[*k].clone()))
                .collect()
        })
        .collect();
    let active = s.query("session.state", json!({}))["reference"].clone();
    json!({ "open": open, "active": active })
}

/// A sphere and a cube, a file reference with rotation unlinked and turned on its own, and a
/// parts reference of both, locked, in wireframe, whose cube then moves in the scene (the
/// frozen copy keeps it where it was); `ref:1` is made active again. Returns the file.
fn scene_with_references(s: &mut Session, dir: &Path) -> String {
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    let obj = at(dir, "reference.obj");
    std::fs::write(&obj, OBJ).unwrap();
    s.ok("reference.open", json!({ "file": obj, "up": "z" }));
    s.ok("reference.link", json!({ "rotation": false, "zoom": true }));
    s.ok("view.orbit", json!({ "view": "ref:1", "right_deg": 30 }));
    let turn = [0.0, 0.258_819, 0.0, 0.965_926];
    let to = json!({ "turn": turn, "scale": 1.5, "offset": [0.25, 0, -1] });
    s.ok("reference.align", json!({ "to": to }));
    s.ok("reference.open", json!({ "parts": ["cube", "sphere"] }));
    s.ok("view.look", json!({ "view": "ref:2", "wireframe": true }));
    s.ok("view.lock", json!({ "view": "ref:2", "locked": true }));
    s.ok(
        "mesh.translate",
        json!({ "part": "cube", "direction": [0, 1, 0], "distance": 2 }),
    );
    s.ok("reference.next", json!({}));
    obj
}

#[test]
fn a_saved_scene_brings_back_its_references_with_their_setups_and_the_active_one() {
    let dir = folder("references");
    let path = at(&dir, "scene.gss");
    let mut a = session(7, None, None);
    scene_with_references(&mut a, &dir);
    let before = references(&mut a);
    assert_eq!(before["active"], json!("ref:1"), "{before}");
    let hash = a.hash();
    save(&mut a, json!({ "path": path }));

    let mut b = session(99, None, None);
    b.ok("reference.open", json!({ "scene": true }));
    let opened = b.ok("file.open", json!({ "path": path })).result;
    let want = json!({ "open": ["ref:1", "ref:2"], "active": "ref:1", "empty": [] });
    assert_eq!(opened["references"], want);
    // What is linked follows main, which is the session's, not the file's: with main where it
    // was, every reference shows what it showed.
    let main = a.query("view.get", json!({ "view": "main" }))["camera"].clone();
    b.ok("view.set", json!({ "view": "main", "camera": main }));
    assert_eq!(
        references(&mut b),
        before,
        "the same sources, links, alignments, looks, own cameras, locks and sizes"
    );
    assert_eq!(b.hash(), hash, "the scene alone");
    // Framing the parts reference fits its frozen copies: the cube where it was, not where the
    // scene moved it.
    let framed = |s: &mut Session| {
        s.ok("view.lock", json!({ "view": "ref:2", "locked": false }));
        s.ok("view.frame", json!({ "view": "ref:2" }));
        s.query("view.get", json!({ "view": "ref:2" }))["camera"]["target"].clone()
    };
    let target = framed(&mut b);
    assert_eq!(target, framed(&mut a));
    assert_eq!(target[1].as_f64(), Some(0.0), "{target}");
}

#[test]
fn a_file_reference_whose_file_is_missing_or_changed_opens_empty_with_its_setup_and_why() {
    let dir = folder("references-gone");
    let path = at(&dir, "scene.gss");
    let mut a = session(4, None, None);
    a.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    let (gone, changed) = (at(&dir, "gone.obj"), at(&dir, "changed.obj"));
    for file in [&gone, &changed] {
        std::fs::write(file, OBJ).unwrap();
        a.ok("reference.open", json!({ "file": file }));
    }
    a.ok("reference.link", json!({ "rotation": false }));
    let setups = |s: &mut Session| {
        let list = s.query("reference.list", json!({}));
        let open = list.as_array().unwrap().iter();
        let setup = |r: &Value| {
            (
                r["reference"]["source"].clone(),
                r["reference"]["link"].clone(),
            )
        };
        open.map(setup).collect::<Vec<_>>()
    };
    let kept = setups(&mut a);
    save(&mut a, json!({ "path": path }));
    std::fs::remove_file(&gone).unwrap();
    std::fs::write(&changed, format!("{OBJ}v 9 9 9\n")).unwrap();

    let mut b = session(4, None, None);
    let opened = b.ok("file.open", json!({ "path": path })).result;
    let empty = opened["references"]["empty"].as_array().unwrap();
    let reasons: Vec<(&str, &str)> = (empty.iter())
        .map(|e| (e["view"].as_str().unwrap(), e["reason"].as_str().unwrap()))
        .collect();
    assert_eq!(reasons.len(), 2, "{opened}");
    assert_eq!(reasons[0].0, "ref:1");
    assert!(reasons[0].1.contains("missing"), "{}", reasons[0].1);
    assert_eq!(reasons[1].0, "ref:2");
    assert!(reasons[1].1.contains("changed"), "{}", reasons[1].1);
    assert_eq!(
        setups(&mut b),
        kept,
        "each keeps its source, pinned hash and link"
    );
    let list = b.query("reference.list", json!({}));
    let parts: Vec<&Value> = (list.as_array().unwrap().iter())
        .map(|r| &r["reference"]["parts"])
        .collect();
    assert_eq!(parts, [&json!([]), &json!([])], "both empty");
}

#[test]
fn references_never_change_the_scene_hash_or_count_as_unsaved_changes() {
    let dir = folder("references-unsaved");
    let path = at(&dir, "scene.gss");
    let mut a = session(5, None, None);
    a.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let hash = a.hash();
    a.ok("reference.open", json!({ "parts": "sphere" }));
    save(&mut a, json!({ "path": path }));
    a.ok("view.look", json!({ "view": "ref:1", "wireframe": true }));
    a.ok("reference.link", json!({ "rotation": false }));
    a.ok("view.orbit", json!({ "view": "ref:1", "right_deg": 45 }));
    a.ok("reference.open", json!({ "scene": true }));
    assert_eq!(a.hash(), hash, "references are not in the scene");
    // None of that is an unsaved change, so the open needs no discard_changes.
    let opened = a.ok("file.open", json!({ "path": path })).result;
    assert_eq!(
        opened["references"]["open"],
        json!(["ref:1"]),
        "the file's own"
    );
    assert_eq!(a.hash(), hash);
    a.ok("reference.close", json!({}));
    a.ok("file.open", json!({ "path": path }));
    let after = a.history()["steps"].as_array().map_or(0, Vec::len);
    assert_eq!(after, 0, "no undo steps either");
}

#[test]
fn a_save_without_references_keeps_none_and_its_open_closes_the_sessions() {
    let dir = folder("references-off");
    let (with, without) = (at(&dir, "with.gss"), at(&dir, "without.gss"));
    let mut a = session(6, None, None);
    a.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    a.ok("reference.open", json!({ "parts": "sphere" }));
    save(&mut a, json!({ "path": with }));
    let off = json!({ "path": without, "references": false });
    save(&mut a, off);
    let read = |p: &str| gs_io::scene::read(Path::new(p), Kept::None).unwrap();
    let (kept, left) = (read(&with), read(&without));
    assert!(kept.session.get("references").is_some(), "saved by default");
    let copy = ["kept Part 'ref:1/sphere', which no saved reference names"];
    assert_eq!(kept.skipped, copy, "with the frozen copy, not read");
    assert!(left.session.get("references").is_none(), "{}", left.session);
    assert!(left.skipped.is_empty(), "no frozen copies");
    let opened = a.ok("file.open", json!({ "path": without })).result;
    assert_eq!(opened["references"]["open"], json!([]));
    assert_eq!(a.query("reference.list", json!({})), json!([]));
}

/// A newer program may add keys to the session section, its `references` and an entry, or a
/// source this version does not know: the file still opens with what this version reads, and
/// what it left out is listed.
#[test]
fn saved_references_with_keys_this_version_does_not_know_still_open() {
    let dir = folder("references-newer");
    let (path, newer) = (at(&dir, "scene.gss"), at(&dir, "newer.gss"));
    let mut a = session(8, None, None);
    a.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    a.ok("reference.open", json!({ "scene": true }));
    a.ok("view.look", json!({ "view": "ref:1", "wireframe": true }));
    save(&mut a, json!({ "path": path }));
    let before = references(&mut a);
    let file = gs_io::scene::read(Path::new(&path), Kept::None).unwrap();
    let mut newer_session = file.session.clone();
    newer_session["later"] = json!({ "anything": true });
    let refs = &mut newer_session["references"];
    refs["later"] = json!(1);
    refs["open"][0]["later"] = json!("kept out");
    let mut unknown = refs["open"][0].clone();
    unknown["view"] = json!("ref:2");
    unknown["source"] = json!({ "stream": { "url": "gs://elsewhere" } });
    refs["open"].as_array_mut().unwrap().push(unknown);
    let out = gs_io::scene::SceneOut {
        kind: gs_io::scene::FileKind::Scene,
        writer: "a newer gigasculpt".into(),
        preview: None,
        scene: file.scene.clone(),
        session: newer_session,
        parts: Vec::new(),
        references: Vec::new(),
    };
    let mut bytes = Vec::new();
    gs_io::scene::write(&mut bytes, &out).unwrap();
    std::fs::write(&newer, bytes).unwrap();

    let mut b = session(8, None, None);
    let opened = b.ok("file.open", json!({ "path": newer })).result;
    assert_eq!(opened["references"]["open"], json!(["ref:1"]), "{opened}");
    let skipped = opened["skipped"].as_array().unwrap();
    assert!(
        skipped.len() == 1 && skipped[0].as_str().unwrap().contains("ref:2"),
        "the entry it cannot read is named: {skipped:?}"
    );
    // The rewritten file has no parts, so the scene reference shows none.
    let setup = |r: &Value| {
        let mut setup = r["open"][0]["reference"].clone();
        setup.as_object_mut().unwrap().remove("parts");
        setup
    };
    assert_eq!(setup(&references(&mut b)), setup(&before));
}

#[test]
fn a_replayed_open_brings_back_the_same_references() {
    let dir = folder("references-replay");
    let path = at(&dir, "scene.gss");
    let mut a = session(9, None, None);
    scene_with_references(&mut a, &dir);
    save(&mut a, json!({ "path": path }));
    let log = dir.join("session.jsonl");
    let mut b = session(9, Some(log.clone()), None);
    b.ok("file.open", json!({ "path": path }));
    let opened = references(&mut b);
    assert_eq!(opened["open"].as_array().map(Vec::len), Some(2));

    let mut c = session(1, None, None);
    let text = std::fs::read_to_string(&log).unwrap();
    for line in text.lines() {
        if let ScriptLine::Record(record) = ScriptLine::parse(line).unwrap() {
            success(c.request(record.request().unwrap()), &record.command);
        }
    }
    assert_eq!(
        references(&mut c),
        opened,
        "the same references, bit for bit"
    );
    assert_eq!(c.hash(), b.hash());
}
