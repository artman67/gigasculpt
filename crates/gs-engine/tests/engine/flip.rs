//! `mesh.flip` (decision 0041, section 9): a part turned into its own mirror image in place.
//! Every face's corners are reversed so it still faces out; every value stays on its point, its
//! face or its face corner; a placed part flips in its own space; a part with detail levels is
//! refused. Twice gives back every bit about the origin, and within one f32 step about the
//! middle.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::support::{EngineSession, agent};

const AXES: [&str; 3] = ["x", "y", "z"];

/// A closed square pyramid off the origin: one quad below and four triangles, its apex at
/// x = -0.0, with a mask, a color per point, a group per face and a UV per face corner, each
/// value different.
fn pyramid() -> PlainMesh {
    let positions = vec![
        [-0.7, 0.5, -0.9],
        [1.3, 0.5, -0.9],
        [1.3, 0.5, 1.1],
        [-0.7, 0.5, 1.1],
        [-0.0, 2.0, 0.1],
    ];
    let t = |a, b| [a, b, 4, NO_VERTEX];
    let faces = vec![[0, 1, 2, 3], t(1, 0), t(2, 1), t(3, 2), t(0, 3)];
    let column = |domain, values| Attribute { domain, values };
    let mut attributes = BTreeMap::new();
    let mask = AttrValues::F32(vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    let color = AttrValues::U32(vec![0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0x00ff_ff00, 7]);
    attributes.insert("mask".into(), column(AttrDomain::Vertex, mask));
    attributes.insert("color".into(), column(AttrDomain::Vertex, color));
    let group = AttrValues::U32(vec![1, 2, 3, 4, 5]);
    attributes.insert("group".into(), column(AttrDomain::Face, group));
    for (k, name) in gs_io::mesh::UV_COLUMNS.iter().enumerate() {
        let (corner, channel) = (k / 2, k % 2);
        let values = (faces.iter().enumerate())
            .map(|(f, face): (usize, &[u32; 4])| match face[corner] {
                NO_VERTEX => 0.0,
                _ => (10 * f + corner) as f32 + 0.5 * channel as f32,
            })
            .collect();
        let values = AttrValues::F32(values);
        attributes.insert((*name).to_string(), column(AttrDomain::Face, values));
    }
    PlainMesh {
        positions,
        faces,
        attributes,
    }
}

/// A session with the pyramid and a sphere off the origin.
fn two_parts() -> Session {
    let mut s = agent();
    s.add_part("pyramid", &pyramid());
    s.ok(
        "primitive.sphere",
        json!({ "subdivisions": 4, "center": [0.3, 0.2, 0.1] }),
    );
    s
}

fn flip(s: &mut Session, part: &str, axis: &str, about: &str) {
    s.ok(
        "mesh.flip",
        json!({ "part": part, "axis": axis, "about": about }),
    );
}

fn volume(s: &mut Session, part: &str) -> f64 {
    let stats = s.query("mesh.stats", json!({ "part": part }));
    stats["parts"][0]["volume"].as_f64().unwrap_or(f64::NAN)
}

/// Whether every face of a convex closed mesh faces away from the mean of its points.
fn faces_out(mesh: &PlainMesh) -> bool {
    let p = |v: u32| mesh.positions[v as usize].map(f64::from);
    let n = mesh.positions.len() as f64;
    let mean = (0..3).map(|k| mesh.positions.iter().map(|q| f64::from(q[k])).sum::<f64>() / n);
    let mean: Vec<f64> = mean.collect();
    mesh.faces.iter().all(|&[a, b, c, _]| {
        let (a, b, c) = (p(a), p(b), p(c));
        let (u, w) = (
            [0, 1, 2].map(|k| b[k] - a[k]),
            [0, 1, 2].map(|k| c[k] - a[k]),
        );
        let normal = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        (0..3).map(|k| normal[k] * (a[k] - mean[k])).sum::<f64>() > 0.0
    })
}

/// The UV of each face corner, by face and the corner's point, as bits (none on a part without
/// UVs).
fn corner_uvs(mesh: &PlainMesh) -> BTreeMap<(usize, u32), [u32; 2]> {
    if !mesh.attributes.contains_key(gs_io::mesh::UV_COLUMNS[0]) {
        return BTreeMap::new();
    }
    let column = |name: &str| match &mesh.attributes[name].values {
        AttrValues::F32(values) => values.clone(),
        AttrValues::U32(_) => panic!("{name} holds integers"),
    };
    let uv: Vec<Vec<f32>> = gs_io::mesh::UV_COLUMNS.iter().map(|n| column(n)).collect();
    let mut out = BTreeMap::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        for (k, &v) in face.iter().enumerate().filter(|(_, v)| **v != NO_VERTEX) {
            out.insert((f, v), [uv[2 * k][f].to_bits(), uv[2 * k + 1][f].to_bits()]);
        }
    }
    out
}

/// The point and face columns, as data to compare.
fn values(mesh: &PlainMesh) -> Value {
    let kept = ["mask", "color", "group"].map(|name| mesh.attributes.get(name));
    serde_json::to_value(kept).unwrap_or_default()
}

#[test]
fn a_flip_about_the_origin_negates_one_coordinate_and_turns_every_face_to_face_out() {
    for (k, axis) in AXES.iter().enumerate() {
        let mut s = two_parts();
        for part in ["pyramid", "sphere"] {
            let (before, inside) = (s.mesh(part), volume(&mut s, part));
            flip(&mut s, part, axis, "origin");
            let after = s.mesh(part);
            for (p, q) in before.positions.iter().zip(&after.positions) {
                let mut want = *p;
                want[k] = -want[k];
                assert_eq!(
                    q.map(f32::to_bits),
                    want.map(f32::to_bits),
                    "{part} on {axis}"
                );
            }
            let turned: Vec<[u32; 4]> = (before.faces.iter())
                .map(|&[a, b, c, d]| {
                    if d == NO_VERTEX {
                        [a, c, b, d]
                    } else {
                        [a, d, c, b]
                    }
                })
                .collect();
            assert_eq!(after.faces, turned, "{part} on {axis}: corners reversed");
            let outside = volume(&mut s, part);
            assert!(
                inside > 0.0 && (outside - inside).abs() <= 1e-12 * inside,
                "{part} on {axis}: the volume keeps its sign, {inside} then {outside}"
            );
            assert!(faces_out(&after), "{part} on {axis}: every face faces out");
        }
    }
}

#[test]
fn twice_about_the_origin_gives_back_every_bit_and_about_the_middle_within_one_step() {
    let mut s = two_parts();
    let start = s.hash();
    for axis in AXES {
        for part in ["pyramid", "sphere"] {
            flip(&mut s, part, axis, "origin");
            flip(&mut s, part, axis, "origin");
        }
    }
    assert_eq!(s.hash(), start, "every point, face and value, bit for bit");

    for (k, axis) in AXES.iter().enumerate() {
        for part in ["pyramid", "sphere"] {
            let before = s.mesh(part);
            flip(&mut s, part, axis, "middle");
            flip(&mut s, part, axis, "middle");
            let after = s.mesh(part);
            assert_eq!(after.faces, before.faces, "{part} on {axis}: faces");
            assert_eq!(values(&after), values(&before), "{part} on {axis}: values");
            assert_eq!(corner_uvs(&after), corner_uvs(&before), "{part} on {axis}");
            let far = (before.positions.iter()).fold(0.0_f32, |m, p| m.max(p[k].abs()));
            for (p, q) in before.positions.iter().zip(&after.positions) {
                let (mut p, mut q) = (*p, *q);
                let off = (p[k] - q[k]).abs();
                assert!(off <= far * f32::EPSILON, "{part} on {axis}: {off} off");
                (p[k], q[k]) = (0.0, 0.0);
                assert_eq!(q.map(f32::to_bits), p.map(f32::to_bits), "{part} on {axis}");
            }
        }
    }
    // A part whose box is centred on its origin flips about the middle by a sign change.
    s.ok("primitive.sphere", json!({ "name": "centred" }));
    let centred = s.hash();
    flip(&mut s, "centred", "y", "middle");
    flip(&mut s, "centred", "y", "middle");
    assert_eq!(s.hash(), centred, "a middle at 0 gives back every bit");
}

/// Each face corner keeps its UV through `mesh.flip` and through `part.mirror_copy`, which
/// shares its reflection, and every point keeps its mask and color, every face its group.
#[test]
fn uvs_follow_their_corners_and_point_and_face_values_stay_on_theirs() {
    let mut s = agent();
    s.add_part("pyramid", &pyramid());
    let start = s.mesh("pyramid");
    s.ok(
        "part.mirror_copy",
        json!({ "part": "pyramid", "name": "copy" }),
    );
    let copy = s.mesh("copy");
    assert_eq!(
        corner_uvs(&copy),
        corner_uvs(&start),
        "the mirror copy's UVs"
    );
    assert_eq!(values(&copy), values(&start), "the mirror copy's values");
    for axis in AXES {
        for about in ["origin", "middle"] {
            flip(&mut s, "pyramid", axis, about);
            let flipped = s.mesh("pyramid");
            let what = format!("on {axis} about the {about}");
            assert_eq!(corner_uvs(&flipped), corner_uvs(&start), "{what}: UVs");
            assert_eq!(values(&flipped), values(&start), "{what}: values");
        }
    }
}

/// A placed part flips in its own space and keeps its transform: flipped, it is the part
/// flipped before it was placed, bit for bit, transform and all.
#[test]
fn a_placed_part_flips_in_its_own_space_and_keeps_its_transform() {
    let frame = json!({ "origin": [0.5, -0.2, 0.3], "axes": [0.0, 0.0, 0.0, 1.0] });
    let ops = [
        json!({ "rotate": { "axis": [1.0, 2.0, 3.0], "degrees": 35 } }),
        json!({ "move": [0.3, -0.2, 0.7] }),
        json!({ "scale": [1.5, 1.5, 1.5] }),
    ];
    let run = |flip_first: bool| {
        let mut s = agent();
        s.ok(
            "primitive.sphere",
            json!({ "subdivisions": 4, "center": [0.3, 0.2, 0.1] }),
        );
        if flip_first {
            flip(&mut s, "sphere", "x", "middle");
        }
        for op in &ops {
            s.ok("transform.apply", json!({ "frame": frame, "op": op }));
        }
        if !flip_first {
            flip(&mut s, "sphere", "x", "middle");
        }
        s
    };
    let (mut placed, mut first) = (run(false), run(true));
    assert_eq!(placed.hash(), first.hash(), "the same points and transform");
    for s in [&mut placed, &mut first] {
        let baked = s.ok("part.apply_transform", json!({}));
        assert!(baked.warnings.is_empty(), "the part kept a transform");
    }
    assert_eq!(placed.hash(), first.hash(), "and baked, the same points");
}

#[test]
fn a_part_with_detail_levels_is_refused_and_left_as_it_was() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({}));
    s.ok("level.subdivide", json!({}));
    // `err` checks that the scene's hash did not change.
    let e = s.err("mesh.flip", json!({ "axis": "y" }));
    assert_eq!(e.code, ErrorCode::Precondition, "{e:?}");
    assert!(
        e.message.contains("'sphere'") && e.message.contains("detail levels"),
        "{}",
        e.message
    );
}

#[test]
fn one_undo_puts_a_flip_back_and_its_logged_line_replays_to_the_same_hash() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("flip-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let log = dir.join("flip.jsonl");
    let mut s = crate::determinism::seeded(1, Some(log.clone()));
    s.add_part("pyramid", &pyramid());
    let before = s.hash();
    flip(&mut s, "pyramid", "y", "middle");
    let once = s.hash();
    s.ok("mesh.flip", json!({ "axis": "z" }));
    let twice = s.hash();
    let mut replay = crate::determinism::seeded(2, None);
    replay.replay_log(&log);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(replay.hash(), twice, "the replay");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), once, "one undo, one flip");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "and the first");
}
