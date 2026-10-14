//! The material palette saved with the scene (decision 0043, section 7): a scene file keeps
//! every material in full, the id counter and the active material, and `file.open` and
//! `file.recover` put them back; a file without a palette opens with the shipped one; opening
//! replaces the palette, clears its undo and hands it to the renderer; `file.new` keeps it; a
//! palette step is an unsaved change; and a palette that does not read opens as the shipped one.
//!
//! `golden/v5.gss` (format 5, written 2026-10-12, 2 KB): a 2-subdivision sphere and the palette
//! `shape` leaves, in a session of seed 0.

use std::path::Path;
use std::sync::{Arc, Mutex};

use gs_core::MeshStore;
use gs_engine::{
    EngineConfig, MaterialPalette, RenderHost, RenderRequest, Screenshot, StateFolder,
};
use gs_io::scene::{FileKind, Kept, SceneOut};
use gs_protocol::Role;
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::autosave::mark_dead;
use crate::golden::golden;
use crate::{at, folder, save, session};

/// The scene hash the version 5 file was written with.
const V5_HASH: &str = "dc809e286499dcc3c3a652da06d30d97bb088de89c1f2f5fdecba8de76f660cc";

/// A change to a saved palette section.
type Change = fn(&mut Value);

/// The palette as `material.list` gives it: every material's id, name, title, built-in flag,
/// kind, environment and values, and the active one.
fn listed(s: &mut Session) -> Value {
    let mut list = s.query("material.list", json!({}));
    list.as_object_mut().map(|l| l.remove("refused"));
    list
}

/// Palette steps of every kind a file must keep: a built-in edited, a standard material's
/// cavity settings, a copy edited, renamed, retitled and made active, a second copy deleted (so
/// the id counter is past every id left), and Flat Color as it shipped.
fn shape(s: &mut Session) {
    let steps = [
        (
            "material.set",
            json!({ "material": "red-wax", "specular_roughness": 0.123_456_79, "base_color": [0.3, 0.1, 0.05] }),
        ),
        (
            "material.set",
            json!({ "material": "basic-material", "cavity_intensity": 1.75, "cavity_color": "#204080" }),
        ),
        (
            "material.copy",
            json!({ "material": "basic-material", "name": "my-basic", "title": "My Basic" }),
        ),
        ("material.copy", json!({ "material": "chrome-a" })),
        ("material.delete", json!({ "material": "#15" })),
        (
            "material.set",
            json!({ "material": "my-basic", "cavity_radius": 2.5, "environment": "soft-studio" }),
        ),
        (
            "material.rename",
            json!({ "material": "my-basic", "name": "mine", "title": "Mine" }),
        ),
        ("material.select", json!({ "name": "mine" })),
    ];
    for (command, args) in steps {
        s.ok(command, args);
    }
}

/// A scene with something in it, its palette shaped.
fn shaped(seed: u64) -> Session {
    let mut s = session(seed, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    shape(&mut s);
    s
}

/// The Check clause "a scene round trip keeps slots and edits".
#[test]
fn a_scene_round_trip_keeps_slots_and_edits() {
    let dir = folder("palette-roundtrip");
    let path = at(&dir, "scene.gss");
    let mut a = shaped(5);
    let before = listed(&mut a);
    save(&mut a, json!({ "path": path }));
    let mut b = session(6, None, None);
    b.ok("file.open", json!({ "path": path }));
    assert_eq!(
        listed(&mut b),
        before,
        "every material's id, name, title, built-in flag and recipe, and the active one"
    );
    let copied = b.ok("material.copy", json!({})).result;
    assert_eq!(
        copied["material"]["id"],
        json!(16),
        "the id counter came back: #15 was handed out and deleted"
    );
    let seen = b.query("file.inspect", json!({ "path": path }));
    assert_eq!(
        (&seen["format"], &seen["oldest_reader"], &seen["materials"]),
        (&json!(5), &json!(1), &json!(21)),
        "format 5, which any reader opens; the twenty built-ins and the copy"
    );
}

/// A file of format 4 or older, or a Part file, keeps no palette: it opens with the shipped one
/// and MatCap Gray active, whatever the session had.
#[test]
fn a_file_without_a_palette_opens_with_the_shipped_one() {
    let shipped = listed(&mut session(0, None, None));
    let dir = folder("palette-none");
    let part_file = at(&dir, "sphere.gss");
    let mut s = shaped(0);
    save(&mut s, json!({ "path": part_file, "parts": ["sphere"] }));
    for file in [golden("v4.gss"), part_file] {
        s.ok(
            "material.copy",
            json!({ "material": "bone", "name": "other" }),
        );
        s.ok("material.select", json!({ "name": "other" }));
        s.ok(
            "file.open",
            json!({ "path": file, "discard_changes": true }),
        );
        assert_eq!(listed(&mut s), shipped, "{file}");
    }
}

/// A recovery opens the autosave's palette and active material, then replays the palette steps
/// logged after it.
#[test]
fn a_recovery_keeps_the_palette() {
    let dir = folder("palette-recover");
    let state = dir.join("state");
    let config = EngineConfig {
        session: "crashed".into(),
        log_path: Some(dir.join("crashed.jsonl")),
        autosave: Some(StateFolder::in_folder(state.clone())),
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Human);
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    shape(&mut s);
    s.ok("file.quick_save", json!({}));
    s.ok(
        "material.copy",
        json!({ "material": "white", "name": "after" }),
    );
    let before = listed(&mut s);
    mark_dead(&state);
    let mut r = session(0, None, Some(&state));
    let id = r.query("file.recovery", json!({}))["candidates"][0]["id"].clone();
    r.ok("file.recover", json!({ "session": id }));
    assert_eq!(listed(&mut r), before);
}

/// `file.new` keeps the palette, and with it the palette steps no file holds yet: it asks only
/// about the document's changes, which are all it drops.
#[test]
fn a_new_scene_keeps_the_palette() {
    let dir = folder("palette-new");
    let path = at(&dir, "scene.gss");
    let mut s = session(0, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    save(&mut s, json!({ "path": path }));
    shape(&mut s);
    let before = listed(&mut s);
    let made = s.ok("file.new", json!({})).result;
    assert_eq!(made["discarded"], json!(false), "the document was saved");
    assert_eq!(listed(&mut s), before);
    let e = s.err("file.open", json!({ "path": path }));
    assert_eq!(
        code(e.code),
        "E_PRECONDITION",
        "the palette steps are still unsaved: {}",
        e.message
    );
}

/// A renderer that draws nothing and keeps every palette the engine hands it.
struct Recorder(Arc<Mutex<Vec<MaterialPalette>>>);

impl RenderHost for Recorder {
    fn render(
        &mut self,
        _parts: &[&MeshStore],
        _request: &RenderRequest,
    ) -> Option<Result<Screenshot, String>> {
        None
    }

    fn materials(&mut self, palette: &MaterialPalette) -> Option<Result<(), String>> {
        self.0.lock().unwrap().push(palette.clone());
        Some(Ok(()))
    }
}

#[test]
fn an_open_replaces_the_palette_clears_its_undo_and_hands_it_over() {
    let dir = folder("palette-open");
    let path = at(&dir, "scene.gss");
    let handed = Arc::new(Mutex::new(Vec::new()));
    let mut s = shaped(0);
    s.engine.attach_renderer(Box::new(Recorder(handed.clone())));
    let saved = listed(&mut s);
    save(&mut s, json!({ "path": path }));
    s.ok(
        "material.copy",
        json!({ "material": "bone", "name": "later" }),
    );
    s.ok("material.select", json!({ "name": "later" }));
    s.ok(
        "file.open",
        json!({ "path": path, "discard_changes": true }),
    );
    assert_eq!(listed(&mut s), saved, "the file's palette");
    let e = s.err("material.undo", json!({}));
    assert_eq!(code(e.code), "E_PRECONDITION", "{}", e.message);
    let last = handed
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("a palette handed over");
    let looks = saved["looks"].as_array().unwrap();
    let ids = |m: &[(u64, &str)]| {
        m.iter()
            .map(|&(id, name)| format!("#{id} {name}"))
            .collect::<Vec<_>>()
    };
    let theirs: Vec<(u64, &str)> = (last.materials.iter())
        .map(|m| (m.id, m.name.as_str()))
        .collect();
    let ours: Vec<(u64, &str)> = (looks.iter())
        .map(|m| (m["id"].as_u64().unwrap(), m["name"].as_str().unwrap()))
        .collect();
    assert_eq!(
        ids(&theirs),
        ids(&ours),
        "the renderer draws the file's palette"
    );
    assert_eq!(last.active().name, "mine", "and its active material");
}

/// A palette step is an unsaved change, an undone one too; choosing a material is none.
#[test]
fn a_palette_step_marks_the_scene_unsaved() {
    let dir = folder("palette-unsaved");
    let path = at(&dir, "scene.gss");
    let mut s = session(0, None, None);
    s.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    save(&mut s, json!({ "path": path }));
    s.ok("material.select", json!({ "name": "red-wax" }));
    s.ok("file.open", json!({ "path": path }));
    let unsaved = |s: &mut Session, why: &str| {
        let e = s.err("file.open", json!({ "path": path }));
        assert_eq!(code(e.code), "E_PRECONDITION", "{why}: {}", e.message);
    };
    s.ok("material.copy", json!({}));
    unsaved(&mut s, "a copy");
    save(&mut s, json!({}));
    s.ok("material.undo", json!({}));
    unsaved(&mut s, "its undo");
    save(&mut s, json!({}));
    s.ok("file.open", json!({ "path": path }));
}

#[test]
fn the_same_palette_saves_to_the_same_bytes() {
    let dir = folder("palette-bytes");
    let path = at(&dir, "scene.gss");
    let save_once = || {
        let mut s = shaped(3);
        save(&mut s, json!({ "path": path }));
        let seen = s.query("file.inspect", json!({ "path": path }));
        assert_eq!(seen["materials"], json!(21), "the file holds the palette");
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        bytes
    };
    assert!(save_once() == save_once(), "byte-identical files");
}

/// Writes a scene file of no Parts with sections `scene` and `session` at `path`.
pub fn write(path: &str, scene: Value, session: Value) {
    let out = SceneOut {
        kind: FileKind::Scene,
        writer: "a test".into(),
        preview: None,
        scene,
        session,
        parts: Vec::new(),
        references: Vec::new(),
    };
    let mut bytes = Vec::new();
    gs_io::scene::write(&mut bytes, &out).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A palette this version cannot read (a key it does not know, a value out of range, two
/// materials with one id or one name, no material) opens as the shipped one and the open says
/// why; a counter below an id the file holds is raised past it.
#[test]
fn a_palette_that_does_not_read_opens_as_the_shipped_one() {
    let shipped = listed(&mut session(0, None, None));
    let dir = folder("palette-unread");
    let (path, wrong) = (at(&dir, "empty.gss"), at(&dir, "wrong.gss"));
    let mut s = session(0, None, None);
    s.ok(
        "material.copy",
        json!({ "material": "white", "name": "mine" }),
    );
    save(&mut s, json!({ "path": path }));
    let file = gs_io::scene::read(Path::new(&path), Kept::None).unwrap();
    let changes: [(Change, &str); 5] = [
        (
            |p| p["materials"][1]["recipe"]["sheen"] = json!(1.0),
            "unknown field `sheen`",
        ),
        (
            |p| p["materials"][1]["recipe"]["material"]["specular_ior"] = json!(9.0),
            "specular_ior: 9 is outside 1 to 3",
        ),
        (|p| p["materials"][1]["id"] = json!(1), "#1"),
        (|p| p["materials"][1]["name"] = json!("gray"), "'gray'"),
        (|p| p["materials"] = json!([]), "no material"),
    ];
    for (change, why) in changes {
        let mut scene = file.scene.clone();
        change(&mut scene["palette"]);
        write(&wrong, scene, file.session.clone());
        let opened = s.ok(
            "file.open",
            json!({ "path": wrong, "discard_changes": true }),
        );
        let said = opened.result["skipped"].to_string();
        assert!(
            said.contains("palette") && said.contains(why),
            "{why}: {said}"
        );
        assert_eq!(listed(&mut s), shipped, "{why}");
    }
    let mut scene = file.scene.clone();
    scene["palette"]["issued"] = json!(2);
    write(&wrong, scene, file.session.clone());
    s.ok(
        "file.open",
        json!({ "path": wrong, "discard_changes": true }),
    );
    let copied = s.ok("material.copy", json!({})).result;
    assert_eq!(
        copied["material"]["id"],
        json!(15),
        "past #14, the copy the file holds"
    );
}

/// Decision 0016, section 3: the version 5 file opens to the scene and the palette it was
/// written with.
#[test]
fn the_version_5_file_opens_to_the_scene_and_palette_it_was_written_from() {
    let path = golden("v5.gss");
    let mut s = session(0, None, None);
    let seen = s.query("file.inspect", json!({ "path": path, "verify": true }));
    assert_eq!(
        (&seen["format"], &seen["oldest_reader"], &seen["materials"]),
        (&json!(5), &json!(1), &json!(21))
    );
    assert_eq!(seen["damaged"], json!([]));
    s.ok("file.open", json!({ "path": path }));
    assert_eq!(s.hash(), V5_HASH, "the same scene");
    let list = listed(&mut s);
    let named = |name: &str| {
        let looks = list["looks"].as_array().unwrap();
        looks
            .iter()
            .find(|m| m["name"] == json!(name))
            .cloned()
            .unwrap_or_default()
    };
    let mine = named("mine");
    assert_eq!(
        (
            &mine["id"],
            &mine["title"],
            &mine["built_in"],
            &mine["environment"]
        ),
        (
            &json!(14),
            &json!("Mine"),
            &json!(false),
            &json!("soft-studio")
        )
    );
    assert_eq!(
        (
            &mine["values"]["cavity_radius"],
            &mine["values"]["cavity_intensity"]
        ),
        (&json!(2.5), &json!(1.75))
    );
    let wax = named("red-wax");
    assert_eq!(wax["values"]["specular_roughness"], json!(0.123_456_79_f32));
    assert_eq!(named("flat-color")["kind"], json!("flat"));
    assert_eq!(list["selected"], json!("mine"));
}
