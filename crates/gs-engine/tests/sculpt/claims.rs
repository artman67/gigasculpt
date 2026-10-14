//! Each brush's declared claims, checked through `brush.stroke` on the engine: `local` (nothing
//! farther than the brush's reach from the stroke changes), `masked_unchanged` (fully masked
//! vertices stay put) and `mirror_symmetric` (a stroke with X symmetry on a part symmetric
//! across x = 0 leaves it symmetric). The brushes and their claims come from `brush.list` and
//! `brush.describe`, so a new brush is checked with no new test code.

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::{line, sphere_session, stroke, vertex_bits};

/// Every brush with its description.
fn brushes(s: &mut Session) -> Vec<Value> {
    let list = s.query("brush.list", json!({}));
    let names: Vec<String> = (list["brushes"].as_array().into_iter().flatten())
        .filter_map(|b| b["name"].as_str().map(String::from))
        .collect();
    assert!(
        names.len() >= 19,
        "every shipped brush is listed: {names:?}"
    );
    (names.iter())
        .map(|n| s.query("brush.describe", json!({ "brush": n })))
        .collect()
}

fn claims(d: &Value) -> Vec<String> {
    (d["claims"].as_array().into_iter().flatten())
        .filter_map(|c| c.as_str().map(String::from))
        .collect()
}

/// A path across the front of the unit sphere for `d`'s brush: from the grab outward for
/// brushes that drag, a straight line across for the rest.
fn path(d: &Value, from: [f32; 3]) -> Vec<[f32; 3]> {
    let to = if d["stroke"] == "drag" {
        [from[0] * 0.5, from[1] + 0.25, from[2] + 0.3]
    } else {
        [from[0] + 0.45, from[1] + 0.15, from[2] - 0.05]
    };
    line(from, to, 7)
}

/// Distance from `p` to the polyline through `points`.
fn to_path(p: [f32; 3], points: &[[f32; 3]]) -> f32 {
    let d = |a: [f32; 3], b: [f32; 3]| (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum::<f32>();
    (points.windows(2))
        .map(|w| {
            let ab: [f32; 3] = std::array::from_fn(|k| w[1][k] - w[0][k]);
            let ap: [f32; 3] = std::array::from_fn(|k| p[k] - w[0][k]);
            let t = ((0..3).map(|k| ab[k] * ap[k]).sum::<f32>() / d(w[1], w[0])).clamp(0.0, 1.0);
            d(p, std::array::from_fn(|k| w[0][k] + ab[k] * t)).sqrt()
        })
        .fold(f32::INFINITY, f32::min)
}

const SIZE: f32 = 0.25;

#[test]
fn strokes_stay_within_the_brush_reach() {
    let mut s = sphere_session(None);
    // Not the white an unpainted part reads as, so a brush that paints changes what it reaches.
    s.ok("color.set", json!({ "main": "#ff3300" }));
    let mut checked = 0;
    for d in brushes(&mut s) {
        if !claims(&d).contains(&"local".to_string()) {
            continue;
        }
        let support = d["settings"]["kernel"]["support"].as_f64().unwrap_or(1.0) as f32;
        let reach = SIZE * support.max(1.0) + 1e-4;
        let points = path(&d, [-0.25, 0.0, 0.97]);
        let before = s.mesh("sphere");
        s.ok(
            "brush.stroke",
            stroke(
                d["name"].as_str().unwrap_or(""),
                &points,
                &json!({ "size": SIZE }),
            ),
        );
        let after = s.mesh("sphere");
        let (old, new) = vertex_bits(&before, &after);
        let changed = (0..old.len()).filter(|&v| old[v] != new[v]).count();
        assert!(changed > 0, "{} changed nothing", d["name"]);
        for v in 0..old.len() {
            let far = to_path(before.positions[v], &points) > reach;
            assert!(
                !far || old[v] == new[v],
                "{} changed vertex {v}, beyond its reach",
                d["name"]
            );
        }
        s.ok("history.undo", json!({}));
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} brushes claim to be local");
}

/// The sphere with its vertices on the +x side fully masked.
fn half_masked(sphere: &PlainMesh) -> PlainMesh {
    let mut m = sphere.clone();
    let mask = sphere
        .positions
        .iter()
        .map(|p| if p[0] > 0.0 { 1.0 } else { 0.0 })
        .collect();
    let mask = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    m.attributes.insert("mask".into(), mask);
    m
}

#[test]
fn masked_vertices_stay_put() {
    let mut s = sphere_session(None);
    let masked = half_masked(&s.mesh("sphere"));
    s.add_part("masked", &masked);
    let mut checked = 0;
    for d in brushes(&mut s) {
        if !claims(&d).contains(&"masked_unchanged".to_string()) {
            continue;
        }
        let points = path(&d, [-0.2, 0.0, 0.98]);
        let mut args = stroke(
            d["name"].as_str().unwrap_or(""),
            &points,
            &json!({ "size": 0.5 }),
        );
        args["part"] = json!("masked");
        let response = s.ok("brush.stroke", args);
        let after = s.mesh("masked");
        let mut moved_free = false;
        for (v, p) in masked.positions.iter().enumerate() {
            let same = after.positions[v].map(f32::to_bits) == p.map(f32::to_bits);
            assert!(p[0] <= 0.0 || same, "{} moved masked vertex {v}", d["name"]);
            moved_free |= !same;
        }
        assert!(
            moved_free,
            "{} moved nothing at all: {}",
            d["name"], response.result
        );
        if response.history_id.is_some() {
            s.ok("history.undo", json!({}));
        }
        checked += 1;
    }
    assert!(
        checked >= 15,
        "only {checked} brushes claim to spare the mask"
    );
}

/// For each vertex, the vertex at its mirror image across x = 0.
fn mirror_partners(m: &PlainMesh) -> Vec<usize> {
    let far =
        |a: [f32; 3], b: [f32; 3]| (a[0] + b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs();
    (m.positions.iter())
        .map(|&p| {
            let best = (0..m.positions.len())
                .min_by(|&i, &j| far(p, m.positions[i]).total_cmp(&far(p, m.positions[j])));
            let best = best.unwrap_or(0);
            assert!(far(p, m.positions[best]) < 1e-5, "the sphere is symmetric");
            best
        })
        .collect()
}

#[test]
fn mirrored_strokes_keep_a_symmetric_part_symmetric() {
    let mut s = sphere_session(None);
    let before = s.mesh("sphere");
    let partner = mirror_partners(&before);
    let mut checked = 0;
    for d in brushes(&mut s) {
        if !claims(&d).contains(&"mirror_symmetric".to_string()) {
            continue;
        }
        let points = path(&d, [-0.45, -0.1, 0.88]);
        let mirror = json!({ "size": 0.2, "symmetry": { "mirror": { "x": true } } });
        s.ok(
            "brush.stroke",
            stroke(d["name"].as_str().unwrap_or(""), &points, &mirror),
        );
        let after = s.mesh("sphere");
        let worst = (0..after.positions.len())
            .map(|v| {
                let (p, q) = (after.positions[v], after.positions[partner[v]]);
                let mut gap = (p[0] + q[0])
                    .abs()
                    .max((p[1] - q[1]).abs())
                    .max((p[2] - q[2]).abs());
                for a in after
                    .attributes
                    .values()
                    .filter(|a| a.domain == AttrDomain::Vertex)
                {
                    if let AttrValues::F32(x) = &a.values {
                        gap = gap.max((x[v] - x[partner[v]]).abs());
                    }
                }
                gap
            })
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1e-4,
            "{} left the sphere asymmetric by {worst}",
            d["name"]
        );
        let (old, new) = vertex_bits(&before, &after);
        assert_ne!(old, new, "{} did something", d["name"]);
        s.ok("history.undo", json!({}));
        checked += 1;
    }
    assert!(
        checked >= 15,
        "only {checked} brushes claim mirror symmetry"
    );
}
