//! Fill Color (decision 0039, section 11, amended for P6): `paint.fill` mixes one color into
//! every point of a part that the mask and hiding leave free, through the stroke's own mix and
//! dither, as one undo step, the same on any number of threads and on replay.

use gs_io::mesh::column;
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::Session;
use serde_json::json;

use crate::determinism::seeded;
use crate::support::{EngineSession, agent, log_records};
use crate::views::log_path;

/// Red and blue, as the color column holds them (red in the low byte).
const RED: u32 = 0xff00_00ff;
const BLUE: u32 = 0xffff_0000;

fn colors(m: &PlainMesh) -> Vec<u32> {
    let white = vec![u32::MAX; m.positions.len()];
    column(m, "color", AttrDomain::Vertex).map_or(white, <[u32]>::to_vec)
}

fn byte(c: u32, channel: u32) -> u32 {
    c >> (8 * channel) & 255
}

/// Whether each point is hidden: every face around it is.
fn hidden_points(m: &PlainMesh) -> Vec<bool> {
    let hide: Vec<bool> = match m.attributes.get("hide").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&h| h != 0).collect(),
        Some(AttrValues::F32(v)) => v.iter().map(|&h| h != 0.0).collect(),
        _ => vec![false; m.faces.len()],
    };
    let mut seen = vec![(0, 0); m.positions.len()];
    for (face, &hidden) in m.faces.iter().zip(&hide) {
        for &v in face.iter().filter(|&&v| v != u32::MAX) {
            seen[v as usize].0 += 1;
            seen[v as usize].1 += usize::from(hidden);
        }
    }
    seen.iter()
        .map(|&(all, hidden)| all > 0 && hidden == all)
        .collect()
}

/// Adds "ball" to `s`: a sphere of `subdivisions` painted blue, masked fully above y = 0.5 and
/// half between y = 0.2 and 0.5, with the faces facing the front camera hidden.
fn ball(mut s: Session, subdivisions: u32) -> Session {
    let mut first = agent();
    first.ok("primitive.sphere", json!({ "subdivisions": subdivisions }));
    let mut mesh = first.mesh("sphere");
    let mask = (mesh.positions.iter())
        .map(|p| {
            if p[1] > 0.5 {
                1.0
            } else if p[1] > 0.2 {
                0.5
            } else {
                0.0
            }
        })
        .collect();
    let blue = vec![BLUE; mesh.positions.len()];
    for (name, values) in [
        ("mask", AttrValues::F32(mask)),
        ("color", AttrValues::U32(blue)),
    ] {
        let domain = AttrDomain::Vertex;
        mesh.attributes
            .insert(name.into(), Attribute { domain, values });
    }
    s.add_part("ball", &mesh);
    let facing = json!({ "select": { "by": "facing" }, "camera": "front" });
    s.ok("visibility.hide", facing);
    s
}

/// At full strength every free point becomes Main's exact bytes; a fully masked point and a
/// hidden one keep theirs, a half masked one goes part of the way, and no point moves.
#[test]
fn a_full_fill_gives_every_free_point_main_and_holds_masked_and_hidden_ones() {
    let mut s = ball(agent(), 16);
    let before = s.mesh("ball");
    s.ok("color.set", json!({ "main": "#ff0000" }));
    s.ok("paint.fill", json!({}));
    let after = s.mesh("ball");
    assert_eq!(after.positions, before.positions, "no point moves");
    let (was, now) = (colors(&before), colors(&after));
    let mask = column::<f32>(&before, "mask", AttrDomain::Vertex).unwrap_or_default();
    let hidden = hidden_points(&before);
    let mut counts = [0; 4];
    for v in 0..was.len() {
        let kind = if hidden[v] {
            0
        } else if mask[v] >= 1.0 {
            1
        } else if mask[v] > 0.0 {
            2
        } else {
            3
        };
        counts[kind] += 1;
        match kind {
            0 | 1 => assert_eq!(now[v], was[v], "point {v} keeps its bytes"),
            2 => assert!(
                now[v] != was[v] && now[v] != RED,
                "point {v}: part of the way"
            ),
            _ => assert_eq!(now[v], RED, "point {v}: Main exactly"),
        }
    }
    assert!(
        counts.iter().all(|&n| n > 10),
        "every kind of point: {counts:?}"
    );
}

/// Below full strength each fill goes part of the way from where the last one left off, so two
/// fills at a half go further than one and still not all the way.
#[test]
fn below_full_strength_two_fills_build_up() {
    let mut s = ball(agent(), 16);
    s.ok("visibility.show_all", json!({}));
    s.ok("mask.set", json!({ "value": 0.0, "scope": "all" }));
    let fill = json!({ "color": "#ff0000", "strength": 0.5 });
    s.ok("paint.fill", fill.clone());
    let once = colors(&s.mesh("ball"));
    s.ok("paint.fill", fill);
    let twice = colors(&s.mesh("ball"));
    for (v, (&a, &b)) in once.iter().zip(&twice).enumerate() {
        assert!(
            byte(BLUE, 0) < byte(a, 0) && byte(a, 0) < byte(b, 0),
            "point {v}: redder"
        );
        assert!(
            byte(BLUE, 2) > byte(a, 2) && byte(a, 2) > byte(b, 2),
            "point {v}: less blue"
        );
        assert_ne!(b, RED, "point {v}: not all the way");
    }
}

/// The session's Main and the seed are pinned in the log, and a half strength fill (which
/// dithers) gives the same bits on 1 and 24 threads and on replay.
#[test]
fn a_fill_gives_the_same_bits_on_1_and_24_threads_and_on_replay() {
    let script = |log: Option<std::path::PathBuf>| {
        let mut s = ball(seeded(1, log), 64);
        s.ok("color.set", json!({ "main": "#ff8000" }));
        s.ok("paint.fill", json!({ "strength": 0.5 }));
        s.hash()
    };
    let log = log_path("fill");
    let one = gs_par::with_threads(1, || script(Some(log.clone())));
    let many = gs_par::with_threads(24, || script(None));
    assert_eq!(one, many, "the same bits on 1 and 24 threads");
    let (_, records) = log_records(&log);
    let fill = (records.iter().find(|r| r.command == "paint.fill"))
        .unwrap_or_else(|| panic!("a fill line"));
    assert_eq!(
        fill.args["color"],
        json!("#ff8000"),
        "Main pinned: {}",
        fill.args
    );
    assert!(fill.args["seed"].is_u64(), "the seed pinned: {}", fill.args);
    let mut replay = seeded(2, None);
    replay.replay_log(&log);
    assert_eq!(replay.hash(), one, "the replay");
}

/// One undo gives back every byte, of one part or of every part a fill of every part changed,
/// and a fill that would change nothing says so, makes no step and adds no column.
#[test]
fn one_undo_gives_back_every_byte_and_a_fill_that_changes_nothing_makes_no_step() {
    let mut s = ball(agent(), 16);
    s.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    s.ok("primitive.cylinder", json!({ "center": [-3, 0, 0] }));
    s.ok(
        "part.set_visible",
        json!({ "parts": ["cylinder"], "visible": false }),
    );
    // The active part always shows (decision 0033): the cylinder hides only once it is not.
    s.ok("part.select", json!({ "part": "ball" }));
    let (hash, was) = (s.hash(), colors(&s.mesh("ball")));
    let steps = s.history()["undoable"].as_u64().unwrap_or_default();
    let done = s.ok("paint.fill", json!({ "every": true, "color": "#00ff00" }));
    let parts = done.effect.map(|e| e.parts).unwrap_or_default();
    assert_eq!(parts, ["ball", "cube"], "every part that shows");
    assert_eq!(s.history()["undoable"], json!(steps + 1), "one step");
    assert!(
        s.query("part.get", json!({ "part": "cylinder" }))["attributes"]
            .get("color")
            .is_none()
    );
    s.ok("history.undo", json!({}));
    assert_eq!(
        (s.hash(), colors(&s.mesh("ball"))),
        (hash.clone(), was),
        "every byte back"
    );
    let white = s.ok("paint.fill", json!({ "part": "cube", "color": "#ffffff" }));
    let reason = (white.effect.as_ref()).and_then(|e| e.noop_reason.clone());
    assert!(
        reason.is_some() && white.history_id.is_none(),
        "white on white: no step"
    );
    assert_eq!(s.hash(), hash, "and no column");
}
