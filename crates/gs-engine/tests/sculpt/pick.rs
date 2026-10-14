//! `live_detail.pick` (decision 0021, L4; catalog `geometry.live-detail-picker`): on a region of
//! 0.1 edges it measures 0.1, and a refining stroke at the size it gives, set with
//! `live_detail.set`, makes edges about that long on a coarser part.

use std::collections::BTreeSet;

use gs_protocol::{NO_VERTEX, PlainMesh};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::{line, stroke, with};

/// A number of a result.
fn num(v: &Value, key: &str) -> f32 {
    v[key].as_f64().unwrap_or_else(|| panic!("no {key} in {v}")) as f32
}

/// The mean length of the edges of `mesh`'s faces whose centers `keep` takes, each edge once.
pub(crate) fn mean_edge(mesh: &PlainMesh, keep: impl Fn([f32; 3]) -> bool) -> (f32, usize) {
    let mut edges = BTreeSet::new();
    for f in &mesh.faces {
        let c: Vec<u32> = f.iter().copied().filter(|&v| v != NO_VERTEX).collect();
        let center: [f32; 3] = std::array::from_fn(|k| {
            c.iter()
                .map(|&v| mesh.positions[v as usize][k])
                .sum::<f32>()
                / c.len() as f32
        });
        if keep(center) {
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                edges.insert((a.min(b), a.max(b)));
            }
        }
    }
    let len = |(a, b): (u32, u32)| {
        let (p, q) = (mesh.positions[a as usize], mesh.positions[b as usize]);
        let d: [f32; 3] = std::array::from_fn(|k| p[k] - q[k]);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let sum: f32 = edges.iter().map(|&e| len(e)).sum();
    (sum / edges.len().max(1) as f32, edges.len())
}

#[test]
fn a_picked_size_makes_new_edges_as_long_as_those_measured() {
    let mut s = Session::new();
    s.ok(
        "primitive.cube",
        json!({ "name": "fine", "subdivisions": 20 }),
    );
    let coarse = json!({ "name": "coarse", "subdivisions": 4, "center": [4, 0, 0] });
    s.ok("primitive.cube", coarse);
    let pick = |s: &mut Session, extra: Value| {
        let at = json!({ "part": "fine", "position": [0.02, 0.03, 1.2] });
        s.query("live_detail.pick", with(at, &extra))
    };
    let picked = pick(&mut s, json!({ "size": 0.4 }));
    let (edge, size) = (num(&picked, "edge"), num(&picked, "constant"));
    assert!((edge - 0.1).abs() < 1e-5, "{picked}");
    assert!((size - 0.1 / 0.7).abs() < 1e-5, "{picked}");
    assert!(
        (num(&picked, "brush") * 0.4 - size).abs() < 1e-5,
        "{picked}"
    );
    assert!(picked.get("screen").is_none(), "{picked}");
    let seen = pick(&mut s, json!({ "camera": "front", "size_px": 30 }));
    let px = num(&seen, "screen");
    assert!(
        px > 0.0 && (num(&seen, "brush") * 30.0 - px).abs() < 1e-3 * px,
        "{seen}"
    );

    s.ok(
        "live_detail.set",
        json!({ "mode": "constant", "size": size }),
    );
    let points = line([3.5, 0.0, 1.0], [4.5, 0.0, 1.0], 20);
    let args = json!({ "part": "coarse", "size": 0.4, "strength": 0.0, "live_detail": {} });
    let done = s.query("brush.stroke", stroke("Draw", &points, &args));
    assert_eq!(done["live_detail"]["refined"], json!(true), "{done}");
    let under = |[x, y, z]: [f32; 3]| (3.7..=4.3).contains(&x) && y.abs() < 0.2 && z > 0.999;
    let (made, count) = mean_edge(&s.mesh("coarse"), under);
    assert!(count > 50, "{count} edges under the stroke");
    assert!(
        (made - edge).abs() < 0.15 * edge,
        "the stroke made edges {made} long on average, against {edge} measured"
    );
}
