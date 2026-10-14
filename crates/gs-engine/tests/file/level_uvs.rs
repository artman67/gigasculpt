//! Level 0's UVs written into a part with levels through `write_level_uvs`, the function
//! `uv.unwrap` calls (decision 0061 as amended 2026-10-13, step UV5), driven by a test-only
//! command; and the siblings decision 0014 section 14 named: `level.unfreeze` smooths the UVs
//! where the frozen levels did, and `level.describe` says which levels keep their own.

use gs_engine::{Command, Ctx, Example, PartArg, register_command, write_level_uvs};
use gs_io::mesh::UV_COLUMNS;
use gs_protocol::{AttrDomain, AttrValues, Attribute, CommandError, Kind, PartRef, PlainMesh};
use gs_testkit::{Session, code};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{NO, bits, only, painted};

/// Writes `corners` as the part's level-0 UVs, four `[u, v]` per base face. Registered in this
/// test program only.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestWriteLevelUvs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    pub corners: Vec<[[f32; 2]; 4]>,
}

impl Command for TestWriteLevelUvs {
    const NAME: &'static str = "test.write_level_uvs";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = ();

    fn examples() -> Vec<Example<Self>> {
        Vec::new()
    }

    fn run(self, cx: &mut Ctx) -> Result<(), CommandError> {
        let name = self.part.map(|p| p.0).unwrap_or_default();
        write_level_uvs(cx, &name, self.corners)
    }
}
register_command!(TestWriteLevelUvs);

/// `painted` with one island: each corner's UV is its point's x and y, so faces agree on the
/// points they share and a smooth divide moves the UVs inside the island.
fn one_island() -> PlainMesh {
    let mut m = painted();
    for (c, name) in UV_COLUMNS.iter().enumerate() {
        let values = (m.faces.iter())
            .map(|f| match f[c / 2] {
                NO => 0.0,
                v => m.positions[v as usize][c % 2],
            })
            .collect();
        let values = AttrValues::F32(values);
        let domain = AttrDomain::Face;
        m.attributes
            .insert((*name).to_string(), Attribute { domain, values });
    }
    m
}

/// Part "P", `mesh` (two quads and a triangle) divided twice, smoothing its UVs when asked: 3,
/// 11 and 44 faces.
fn leveled(s: &mut Session, mesh: &PlainMesh, smooth_uvs: bool) {
    s.add_part("P", mesh);
    for _ in 0..2 {
        s.ok(
            "level.subdivide",
            json!({ "part": "P", "smooth_uvs": smooth_uvs }),
        );
    }
}

/// Every level of "P" as `read` gives it, from the base up, then level `back` made active.
fn every_level(s: &mut Session, back: u32, read: fn(&PlainMesh) -> String) -> Vec<String> {
    let out = (1..=3)
        .map(|level| {
            s.ok("level.set", json!({ "part": "P", "level": level }));
            read(&s.mesh("P"))
        })
        .collect();
    s.ok("level.set", json!({ "part": "P", "level": back }));
    out
}

/// The faces and their UVs as bits, the points left out.
fn uvs(m: &PlainMesh) -> String {
    let positions = Vec::new();
    bits(&PlainMesh {
        positions,
        ..only(m, &UV_COLUMNS)
    })
}

/// A layout for the base's three faces unlike the one it has.
fn layout() -> Vec<[[f32; 2]; 4]> {
    let at = |f: usize, k: usize| [f as f32 * 0.25 + k as f32 * 0.0625, 0.5 - k as f32 * 0.125];
    (0..3).map(|f| std::array::from_fn(|k| at(f, k))).collect()
}

#[test]
fn undo_puts_every_level_back_and_the_step_holds_only_the_old_uvs() {
    for active in [3, 2] {
        let mut s = Session::new();
        leveled(&mut s, &painted(), true);
        let before = every_level(&mut s, active, bits);
        let hash = s.hash();
        s.ok(
            "test.write_level_uvs",
            json!({ "part": "P", "corners": layout() }),
        );
        let written = s.hash();
        assert_ne!(written, hash, "level {active}: the write changed the part");
        // 32 bytes per top face, the kept sets of levels 1 and 2, and with level 2 active its
        // own old columns (decision 0061 as amended, departure 1).
        let bytes = s.history()["steps"][0]["bytes"].as_u64().unwrap();
        let bound = 32 * (44 + 3 + 11) + if active == 2 { 32 * 11 } else { 0 };
        assert!(
            (32 * 44..=bound).contains(&bytes),
            "level {active}: {bytes} bytes"
        );
        s.ok("history.undo", json!({}));
        assert_eq!(s.hash(), hash, "level {active}: undone");
        s.ok("history.redo", json!({}));
        assert_eq!(s.hash(), written, "level {active}: redone");
        s.ok("history.undo", json!({}));
        assert_eq!(
            every_level(&mut s, active, bits),
            before,
            "level {active}: every level back"
        );
    }
}

#[test]
fn a_frozen_part_is_refused_naming_unfreeze() {
    let mut s = Session::new();
    leveled(&mut s, &painted(), false);
    s.ok("level.freeze", json!({ "part": "P" }));
    let err = s.err(
        "test.write_level_uvs",
        json!({ "part": "P", "corners": layout() }),
    );
    assert_eq!(code(err.code), "E_PRECONDITION", "{}", err.message);
    let hint = err.hint.unwrap_or_default();
    assert!(hint.contains("level.unfreeze"), "{hint}");
}

/// What `level.describe` says of each level of "P": whether it keeps its own UVs.
fn own_uvs(s: &mut Session) -> Vec<serde_json::Value> {
    let d = s.query("level.describe", json!({ "part": "P" }));
    (0..3).map(|j| d["levels"][j]["own_uvs"].clone()).collect()
}

#[test]
fn unfreeze_smooths_the_uvs_where_the_frozen_levels_did() {
    let mut flat = Session::new();
    leveled(&mut flat, &one_island(), false);
    let linear = every_level(&mut flat, 1, uvs);
    let mut s = Session::new();
    leveled(&mut s, &one_island(), true);
    let before = every_level(&mut s, 1, uvs);
    assert_ne!(before[2], linear[2], "smoothing moved the top's UVs");
    s.ok("level.freeze", json!({ "part": "P" }));
    // A group changed on the base: unfreeze projects, dividing the base again.
    s.ok(
        "group.assign",
        json!({ "part": "P", "select": { "by": "faces", "indexes": [0] }, "id": 5 }),
    );
    let back = s.ok("level.unfreeze", json!({ "part": "P" }));
    assert_eq!(back.result["how"], json!("projected"));
    assert_eq!(every_level(&mut s, 1, uvs), before, "every level's UVs");
}

#[test]
fn describe_says_which_levels_keep_their_own_uvs() {
    let mut s = Session::new();
    leveled(&mut s, &one_island(), true);
    assert_eq!(own_uvs(&mut s), [json!(true), json!(true), json!(false)]);
    let mut s = Session::new();
    leveled(&mut s, &one_island(), false);
    assert_eq!(own_uvs(&mut s), [json!(false), json!(false), json!(false)]);
}
