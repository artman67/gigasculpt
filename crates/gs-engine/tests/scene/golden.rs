//! One small file per scene file format version, written by that version's writer and kept
//! here, which every later version must open to the same scene (decision 0016, section 3).
//!
//! `golden/v1.gss` (format 1, written 2026-10-10, 12 KB): a 4-subdivision sphere remeshed at
//! resolution 16 (Voxel Remesh's kept cell size) with a Mask Pen stroke, and the OBJ part of
//! `main.rs` imported at scale 0.5 (its file's numbering, face groups, an export scale), in a
//! session of seed 0.
//!
//! `golden/v2.gss` (format 2, written 2026-10-10, 23 KB): a 4-subdivision sphere with two
//! creases on its base (sharp for 2 levels and for good), divided twice, then at level 2 a Draw
//! stroke, a Mask Pen stroke and the unmasked faces hidden, none of it carried up to the top
//! (`levels.rs`, `leveled` and `masked_and_hidden`), in a session of seed 0.
//!
//! `golden/v3.gss` (format 3, written 2026-10-11, 27 KB): a 3-subdivision sphere divided
//! twice, a Draw stroke on its top, a Clay stroke on level 2 not carried up, frozen (decision
//! 0030), then its base remeshed at resolution 16 (`frozen.rs`, `frozen`), in a session of
//! seed 0.
//!
//! `golden/v4.gss` (format 4, written 2026-10-11, 4 KB): a 4 by 2 grid of quads imported from
//! OBJ with two UV islands side by side, divided twice with smooth UVs (decision 0014, section
//! 14), so levels 1 and 2 keep their own UVs, then set to level 2, in a session of seed 0.
//!
//! `golden/v5.gss` (format 5) is described and tested in `palette.rs`, beside the palette it
//! keeps.

use std::path::PathBuf;

use serde_json::json;

use crate::session;

/// The scene hash the version 1 file was written with.
const V1_HASH: &str = "c99b9d748e53c86b115210f809edc86356b8dcec0f860c89e3ecba5d71858bd8";
/// The scene hash the version 2 file was written with.
const V2_HASH: &str = "53e3c0776237751f624031acbf5f4e58e24fded390cf33b2abdc7faf15cf1081";
/// The scene hash the version 3 file was written with.
const V3_HASH: &str = "822b00f64605580ca584f7e0ddfea55493f5c7ac73854d053e1363f958f50ef7";
/// The scene hash the version 4 file was written with.
const V4_HASH: &str = "cffad68cdc79aaab8ff966126ed824eb99ca48bdca5a30786dea39351a9bf2c0";

/// The golden file `name`.
pub fn golden(name: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scene/golden");
    dir.join(name).to_string_lossy().into_owned()
}

#[test]
fn the_version_1_file_opens_to_the_scene_it_was_written_from() {
    let path = golden("v1.gss");
    let before = std::fs::read(&path).expect("the golden file");
    let mut s = session(0, None, None);
    let seen = s.query("file.inspect", json!({ "path": path, "verify": true }));
    assert_eq!(
        (seen["format"].clone(), seen["oldest_reader"].clone()),
        (json!(1), json!(1))
    );
    assert_eq!(seen["damaged"], json!([]));
    let opened = s.ok("file.open", json!({ "path": path })).result;
    assert_eq!(opened["parts"], json!(["painted", "sphere"]));
    assert_eq!(s.hash(), V1_HASH, "the same scene");
    let parts = seen["parts"].as_array().expect("parts");
    let scale = &parts[0]["props"]["export"]["scale"];
    assert_eq!(
        scale,
        &json!(2.0),
        "properties came back: write back at the file's size"
    );
    assert!(
        parts[1]["props"]["voxel"]["voxel"].is_number(),
        "{}",
        parts[1]["props"]
    );
    assert!(
        std::fs::read(&path).expect("still there") == before,
        "opening leaves it untouched"
    );
}

#[test]
fn the_version_2_file_opens_to_the_scene_it_was_written_from() {
    let path = golden("v2.gss");
    let mut s = session(0, None, None);
    let seen = s.query("file.inspect", json!({ "path": path, "verify": true }));
    assert_eq!(
        (seen["format"].clone(), seen["oldest_reader"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(seen["damaged"], json!([]));
    let sphere = &seen["parts"][0];
    assert_eq!(
        (&sphere["levels"], &sphere["active_level"]),
        (&json!(3), &json!(2))
    );
    s.ok("file.open", json!({ "path": path }));
    assert_eq!(s.hash(), V2_HASH, "the same scene");
    let levels = s.query("level.describe", json!({}));
    assert_eq!(
        (&levels["active"], &levels["pending"]),
        (&json!(2), &json!(true)),
        "level 2, its edits still to carry up"
    );
}

#[test]
fn the_version_3_file_opens_to_the_scene_it_was_written_from() {
    let path = golden("v3.gss");
    let mut s = session(0, None, None);
    let seen = s.query("file.inspect", json!({ "path": path, "verify": true }));
    assert_eq!(
        (seen["format"].clone(), seen["oldest_reader"].clone()),
        (json!(3), json!(3))
    );
    assert_eq!(seen["damaged"], json!([]));
    s.ok("file.open", json!({ "path": path }));
    assert_eq!(s.hash(), V3_HASH, "the same scene");
    let levels = s.query("level.describe", json!({}));
    assert_eq!(
        (&levels["count"], &levels["frozen"]),
        (&json!(1), &json!(3)),
        "the remeshed base, its three levels set aside"
    );
    let back = s.ok("level.unfreeze", json!({})).result;
    assert_eq!(back["how"], json!("projected"));
}

#[test]
fn the_version_4_file_opens_to_the_scene_it_was_written_from() {
    let path = golden("v4.gss");
    let mut s = session(0, None, None);
    let seen = s.query("file.inspect", json!({ "path": path, "verify": true }));
    assert_eq!(
        (seen["format"].clone(), seen["oldest_reader"].clone()),
        (json!(4), json!(4))
    );
    assert_eq!(seen["damaged"], json!([]));
    s.ok("file.open", json!({ "path": path }));
    assert_eq!(s.hash(), V4_HASH, "the same scene");
    let levels = s.query("level.describe", json!({}));
    assert_eq!(
        (&levels["count"], &levels["active"]),
        (&json!(3), &json!(2)),
        "level 2, showing the layout it was smoothed from"
    );
}
