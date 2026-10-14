//! A frozen part through a scene file (decision 0030, section 7): frozen after a remesh, or with
//! a stroke on its base, it reopens with the same scene hash and its levels set aside, and
//! unfreezing it then gives the same bits as in the session that saved it. A frozen part
//! replaces a version 2 file only when asked, a parts reference of one saves and reopens, and
//! frozen levels no command makes refuse their part by name.

use gs_core::{MeshStore, StoredLevels};
use gs_io::scene::{FileKind, Kept, NumberingIn, NumberingOut, PartIn, PartOut, SceneOut};
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::golden::golden;
use crate::{at, folder, save, session, stroke};

/// A sphere with three levels, detail on its top and an edit pending on level 2, frozen.
pub fn frozen(s: &mut Session) {
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    s.ok(
        "brush.stroke",
        stroke("Draw", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]),
    );
    s.ok("level.set", json!({ "level": 2 }));
    s.ok(
        "brush.stroke",
        stroke("Clay", [0.3, -0.2, 1.0], [-0.1, 0.2, 1.0]),
    );
    s.ok("level.freeze", json!({}));
}

/// The scene hash and the levels as `level.describe` gives them.
fn state(s: &mut Session) -> (String, Value) {
    (s.hash(), s.query("level.describe", json!({})))
}

/// Builds a scene with `build`, saves it and opens it in another session; both must hold the
/// same scene, and unfreeze the same way to the same bits. Returns how unfreeze brought the
/// levels back.
fn round_trip(name: &str, build: impl Fn(&mut Session)) -> Value {
    let dir = folder(name);
    let path = at(&dir, "scene.gss");
    let mut a = session(5, None, None);
    build(&mut a);
    let before = state(&mut a);
    save(&mut a, json!({ "path": path }));
    let seen = a.query("file.inspect", json!({ "path": path, "verify": true }));
    assert_eq!(
        seen["oldest_reader"],
        json!(3),
        "{name}: a frozen part needs format 3"
    );
    let mut b = session(9, None, None);
    let opened = b.ok("file.open", json!({ "path": path })).result;
    assert_eq!(opened["refused"], json!([]), "{name}: the part opens");
    assert_eq!(
        state(&mut b),
        before,
        "{name}: the same scene, levels set aside"
    );
    let how = a.ok("level.unfreeze", json!({})).result["how"].clone();
    let other = b.ok("level.unfreeze", json!({})).result["how"].clone();
    assert_eq!(other, how, "{name}: unfrozen the same way");
    assert_eq!(state(&mut b), state(&mut a), "{name}: unfrozen the same");
    how
}

#[test]
fn a_frozen_part_remeshed_reopens_the_same_and_unfreezes_the_same() {
    let how = round_trip("frozen-remeshed", |s| {
        frozen(s);
        s.ok("mesh.voxel_remesh", json!({ "resolution": 16 }));
    });
    assert_eq!(how, json!("projected"));
}

#[test]
fn a_stroke_on_a_frozen_base_reopens_the_same_and_carries_the_same() {
    let how = round_trip("frozen-stroked", |s| {
        frozen(s);
        s.ok(
            "brush.stroke",
            stroke("Draw", [0.0, 0.3, 1.0], [0.2, 0.4, 1.0]),
        );
    });
    assert_eq!(
        how,
        json!("carried"),
        "the base kept its faces through the file"
    );
}

#[test]
fn frozen_levels_replace_a_version_2_file_only_when_asked() {
    let dir = folder("frozen-upgrade");
    let path = at(&dir, "scene.gss");
    let old = std::fs::read(golden("v2.gss")).expect("the golden file");
    std::fs::write(&path, &old).expect("a copy");
    let mut s = session(1, None, None);
    s.ok("file.open", json!({ "path": path }));
    s.ok("level.freeze", json!({}));
    let refused = s.err("file.save", json!({}));
    assert_eq!(code(refused.code), "E_PRECONDITION");
    assert!(
        refused.message.contains("format 3 (frozen detail levels)"),
        "{}",
        refused.message
    );
    assert!(
        std::fs::read(&path).expect("still there") == old,
        "the file is untouched"
    );
    save(&mut s, json!({ "upgrade": true }));
    let seen = s.query("file.inspect", json!({ "path": path }));
    assert_eq!(seen["oldest_reader"], json!(3));
    let hash = s.hash();
    let mut t = session(1, None, None);
    t.ok("file.open", json!({ "path": path }));
    assert_eq!(t.hash(), hash, "the upgraded file opens to the scene");
}

/// Review 0007, S3: a parts reference keeps a copy of what a part shows, so a copy of a frozen
/// part is saved without its frozen levels (the writer refuses a kept Part holding them) and
/// comes back with the scene.
#[test]
fn a_parts_reference_of_a_frozen_part_saves_and_reopens() {
    let dir = folder("frozen-reference");
    let path = at(&dir, "scene.gss");
    let mut a = session(5, None, None);
    frozen(&mut a);
    a.ok("reference.open", json!({ "parts": "sphere" }));
    let hash = a.hash();
    save(&mut a, json!({ "path": path }));
    let mut b = session(9, None, None);
    b.ok("reference.open", json!({ "scene": true }));
    let opened = b.ok("file.open", json!({ "path": path })).result;
    assert_eq!(opened["references"]["open"], json!(["ref:1"]), "{opened}");
    assert_eq!(b.hash(), hash, "the scene, its frozen part included");
}

/// A Part as a file holds it, its store rebuilt, to be written again.
struct Again {
    name: String,
    props: Value,
    store: MeshStore,
    numbering: NumberingIn,
    levels: Option<StoredLevels>,
}

impl Again {
    fn of(p: PartIn) -> Again {
        let store = MeshStore::from_exact(p.config, p.attrs, p.nodes).expect("a store");
        let (name, props, numbering, levels) = (p.name, p.props, p.numbering, p.levels);
        Again {
            name,
            props,
            store,
            numbering,
            levels,
        }
    }

    /// As gs-io writes it, with `frozen` as its frozen levels.
    fn out<'a>(&'a self, frozen: Option<&'a Again>) -> PartOut<'a> {
        let numbering = match &self.numbering {
            NumberingIn::Spatial => NumberingOut::Spatial,
            NumberingIn::Given {
                vertices,
                faces,
                first_corner,
            } => NumberingOut::Given {
                vertices,
                faces,
                first_corner,
            },
        };
        PartOut {
            name: &self.name,
            props: self.props.clone(),
            store: &self.store,
            numbering,
            levels: self.levels.as_ref(),
            frozen: frozen.map(|f| Box::new(f.out(None))),
        }
    }
}

/// Review 0007, T1 (decision 0030, section 7): frozen levels of one level, and frozen levels
/// beside a part's own detail levels, refuse that part by name. No command makes either, so a
/// saved frozen part is read with gs-io and written again with its pieces swapped; written
/// again as read, it opens and unfreezes as saved.
#[test]
fn frozen_levels_of_one_level_or_beside_levels_refuse_their_part_by_name() {
    let dir = folder("frozen-crafted");
    let path = at(&dir, "saved.gss");
    let mut a = session(5, None, None);
    frozen(&mut a);
    let hash = a.hash();
    save(&mut a, json!({ "path": path }));
    let file =
        gs_io::scene::read_bytes(&std::fs::read(&path).expect("saved"), Kept::None).expect("reads");
    let mut part = file
        .parts
        .into_iter()
        .next()
        .expect("one part")
        .expect("reads");
    let record = Again::of(*part.frozen.take().expect("frozen levels"));
    let base = Again::of(part);
    let one = Again {
        name: record.name.clone(),
        props: record.props.clone(),
        store: record.store.share(),
        numbering: NumberingIn::Spatial,
        levels: None,
    };
    for (case, (part, frozen), refused) in [
        ("as-read", (&base, &record), json!([])),
        (
            "one-level",
            (&base, &one),
            json!([{ "part": "sphere", "reason": "its frozen levels hold one level" }]),
        ),
        (
            "beside-levels",
            (&record, &record),
            json!([{ "part": "sphere", "reason": "it has detail levels and frozen ones" }]),
        ),
    ] {
        let scene = SceneOut {
            kind: FileKind::Scene,
            writer: "a test".into(),
            preview: None,
            scene: file.scene.clone(),
            session: file.session.clone(),
            parts: vec![part.out(Some(frozen))],
            references: Vec::new(),
        };
        let mut bytes = Vec::new();
        gs_io::scene::write(&mut bytes, &scene).expect("written");
        let path = at(&dir, &format!("{case}.gss"));
        std::fs::write(&path, bytes).expect("a file");
        let mut b = session(9, None, None);
        let opened = b.ok("file.open", json!({ "path": path })).result;
        assert_eq!(opened["refused"], refused, "{case}");
        if case == "as-read" {
            assert_eq!(b.hash(), hash, "{case}: the scene as saved");
            let back = b.ok("level.unfreeze", json!({})).result;
            assert_eq!(back["how"], json!("restored"), "{case}");
        }
    }
}
