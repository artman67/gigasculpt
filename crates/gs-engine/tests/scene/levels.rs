//! A part with detail levels through a scene file (decision 0016, section 15). Saved at its top
//! level, at a lower level with edits not carried up yet, with creases, or with a mask and
//! hidden faces, it reopens with the same scene hash and the same levels, and the next stroke,
//! undo, redo and switch to every level give the same bits as in the session that saved it. A
//! scene with levels replaces its own version 1 file only when asked.

use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::golden::golden;
use crate::{at, folder, save, session, stroke};

/// A sphere with three detail levels, with creases on its base when `creased`: two edges of
/// its first face, one sharp for 2 levels and one for good.
pub fn leveled(s: &mut Session, creased: bool) {
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let face = &s.query("part.get", json!({ "part": "sphere" }))["faces"][0];
    let creases = if creased {
        json!([
            { "a": face[0], "b": face[1], "sharpness": 2 },
            { "a": face[1], "b": face[2], "sharpness": 255 },
        ])
    } else {
        json!([])
    };
    s.ok("level.subdivide", json!({ "creases": creases }));
    s.ok("level.subdivide", json!({}));
}

/// Level 2 with a stroke and a mask on it and the unmasked faces hidden, none of it carried up.
pub fn masked_and_hidden(s: &mut Session) {
    s.ok("level.set", json!({ "level": 2 }));
    s.ok(
        "brush.stroke",
        stroke("Draw", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]),
    );
    s.ok(
        "brush.stroke",
        stroke("Mask Pen", [-0.3, -0.2, 1.0], [0.3, 0.2, 1.0]),
    );
    s.ok("visibility.hide", json!({ "select": { "by": "unmasked" } }));
}

/// The scene hash and the levels as `level.describe` gives them.
fn state(s: &mut Session) -> (String, Value) {
    (s.hash(), s.query("level.describe", json!({})))
}

/// What the next steps give, after each: a stroke, undo, redo, a switch to every level from
/// the top down, a stroke on the base and a switch back to the top.
fn next(s: &mut Session) -> Vec<(String, Value)> {
    let mut seen = Vec::new();
    s.ok(
        "brush.stroke",
        stroke("Draw", [0.2, 0.4, 1.0], [-0.2, 0.0, 1.0]),
    );
    seen.push(state(s));
    s.ok("history.undo", json!({}));
    seen.push(state(s));
    s.ok("history.redo", json!({}));
    seen.push(state(s));
    for level in [3, 2, 1] {
        s.ok("level.set", json!({ "level": level }));
        seen.push(state(s));
    }
    s.ok(
        "brush.stroke",
        stroke("Clay Build", [0.0, -0.3, 1.0], [0.1, 0.3, 1.0]),
    );
    s.ok("level.set", json!({ "level": 3 }));
    seen.push(state(s));
    seen
}

/// Builds a scene with `build`, saves it and opens it in another session; both must hold the
/// same scene and give the same bits for the next steps. Returns the levels as saved.
fn round_trip(name: &str, build: impl Fn(&mut Session)) -> Value {
    let dir = folder(name);
    let path = at(&dir, "scene.gss");
    let mut a = session(5, None, None);
    build(&mut a);
    let before = state(&mut a);
    save(&mut a, json!({ "path": path }));
    let mut b = session(9, None, None);
    let opened = b.ok("file.open", json!({ "path": path })).result;
    assert_eq!(opened["refused"], json!([]), "{name}: the part opens");
    assert_eq!(state(&mut b), before, "{name}: the same scene and levels");
    assert_eq!(next(&mut b), next(&mut a), "{name}: the next steps");
    before.1
}

#[test]
fn a_part_at_its_top_level_reopens_the_same() {
    let levels = round_trip("levels-top", |s| {
        leveled(s, false);
        s.ok(
            "brush.stroke",
            stroke("Draw", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]),
        );
    });
    assert_eq!(
        (&levels["count"], &levels["active"], &levels["pending"]),
        (&json!(3), &json!(3), &json!(false))
    );
}

#[test]
fn a_lower_level_with_edits_not_carried_up_reopens_the_same() {
    let levels = round_trip("levels-pending", |s| {
        leveled(s, false);
        s.ok(
            "brush.stroke",
            stroke("Draw", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]),
        );
        s.ok("level.set", json!({ "level": 2 }));
        s.ok(
            "brush.stroke",
            stroke("Clay", [0.3, -0.2, 1.0], [-0.1, 0.2, 1.0]),
        );
    });
    assert_eq!(
        (&levels["active"], &levels["pending"]),
        (&json!(2), &json!(true))
    );
}

#[test]
fn creases_reopen_the_same() {
    round_trip("levels-creases", |s| {
        leveled(s, true);
        s.ok("level.set", json!({ "level": 1 }));
        s.ok(
            "brush.stroke",
            stroke("Draw", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]),
        );
    });
}

#[test]
fn a_mask_and_hidden_faces_reopen_the_same() {
    round_trip("levels-masked", |s| {
        leveled(s, false);
        masked_and_hidden(s);
    });
}

#[test]
fn levels_replace_a_version_1_file_only_when_asked() {
    let dir = folder("levels-upgrade");
    let (plain, leveled) = (at(&dir, "plain.gss"), at(&dir, "leveled.gss"));
    let old = std::fs::read(golden("v1.gss")).expect("the golden file");
    std::fs::write(&plain, &old).expect("a copy");
    std::fs::write(&leveled, &old).expect("a copy");
    let mut s = session(1, None, None);
    s.ok("file.open", json!({ "path": plain }));
    s.ok(
        "mesh.translate",
        json!({ "part": "painted", "direction": [1, 0, 0], "distance": 1 }),
    );
    save(&mut s, json!({}));
    let seen = s.query("file.inspect", json!({ "path": plain }));
    assert_eq!(
        seen["oldest_reader"],
        json!(1),
        "a scene without levels keeps version 1 programs able to open its file"
    );

    s.ok(
        "file.open",
        json!({ "path": leveled, "discard_changes": true }),
    );
    s.ok("level.subdivide", json!({ "part": "painted" }));
    let refused = s.err("file.save", json!({}));
    assert_eq!(code(refused.code), "E_PRECONDITION");
    assert!(refused.message.contains("format 1"), "{}", refused.message);
    assert!(
        std::fs::read(&leveled).expect("still there") == old,
        "the file is untouched"
    );
    save(&mut s, json!({ "upgrade": true }));
    let seen = s.query("file.inspect", json!({ "path": leveled }));
    assert_eq!(seen["oldest_reader"], json!(2));
    let painted = &seen["parts"][0];
    assert_eq!(
        (
            &painted["name"],
            &painted["levels"],
            &painted["active_level"]
        ),
        (&json!("painted"), &json!(2), &json!(2))
    );
    let hash = s.hash();
    let mut t = session(1, None, None);
    t.ok("file.open", json!({ "path": leveled }));
    assert_eq!(t.hash(), hash, "the upgraded file opens to the scene");
}
