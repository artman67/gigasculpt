//! Masks through Subdivide and Voxel Remesh (decision 0037, section 5): a masked part divided
//! or remeshed keeps its mask where it was, and a stroke afterwards leaves the masked area
//! alone.

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::Session;
use serde_json::json;

use crate::support::agent;

/// How fast the mask rises along z: from 0 at z = -0.2 to 1 at z = 0.6.
const SLOPE: f32 = 1.25;

/// The mask at point `p`.
fn ramp(p: [f32; 3]) -> f32 {
    ((p[2] + 0.2) * SLOPE).clamp(0.0, 1.0)
}

fn mask_of(mesh: &PlainMesh) -> &[f32] {
    match mesh.attributes.get("mask").map(|a| &a.values) {
        Some(AttrValues::F32(m)) => m,
        _ => panic!("the part has no mask"),
    }
}

fn longest_edge(mesh: &PlainMesh) -> f32 {
    (mesh.faces.iter())
        .flat_map(|f| {
            let n = if f[3] == u32::MAX { 3 } else { 4 };
            (0..n).map(move |k| (f[k], f[(k + 1) % n]))
        })
        .map(|(a, b)| {
            let (p, q) = (mesh.positions[a as usize], mesh.positions[b as usize]);
            (0..3)
                .map(|k| (p[k] - q[k]) * (p[k] - q[k]))
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0, f32::max)
}

/// A session with the part "sphere", a unit sphere of 3,456 quads masked by `ramp`, and its
/// longest edge.
fn masked_sphere() -> (Session, f32) {
    let mut first = agent();
    first.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let mut mesh = first.mesh("sphere");
    let mask = mesh.positions.iter().map(|&p| ramp(p)).collect();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(mask),
    };
    mesh.attributes.insert("mask".into(), column);
    let mut s = agent();
    s.add_part("sphere", &mesh);
    (s, longest_edge(&mesh))
}

/// Every point holds the mask `ramp` gives where it is now within `near` (the mask comes from
/// old points no farther than that, so it can differ from the ramp by `SLOPE * near`), and
/// exactly 1 or 0 where the ramp is 1 or 0 for that far around. Returns the fully masked points.
fn mask_where_it_was(mesh: &PlainMesh, near: f32) -> usize {
    let mask = mask_of(mesh);
    assert_eq!(mask.len(), mesh.positions.len(), "one mask value per point");
    let mut full = 0;
    for (&p, &m) in mesh.positions.iter().zip(mask) {
        let off = (m - ramp(p)).abs();
        assert!(
            off <= SLOPE * near,
            "a point at {p:?} has mask {m}, {off} off"
        );
        let (below, above) = (ramp([0.0, 0.0, p[2] - near]), ramp([0.0, 0.0, p[2] + near]));
        if is(below, 1.0) {
            assert!(
                is(m, 1.0),
                "a point well inside the masked area at {p:?}: {m}"
            );
        }
        if is(above, 0.0) {
            assert!(
                is(m, 0.0),
                "a point well outside the masked area at {p:?}: {m}"
            );
        }
        full += usize::from(is(m, 1.0));
    }
    full
}

/// Whether `x` is exactly `v`.
fn is(x: f32, v: f32) -> bool {
    x.to_bits() == v.to_bits()
}

/// A Draw stroke across the edge of the masked area changes points, and none fully masked.
fn stroke_leaves_the_masked_area_alone(s: &mut Session) {
    let before = s.mesh("sphere");
    let samples: Vec<_> = (0..4)
        .map(|i| json!({ "position": [0.05 * i as f32, -0.8, 0.6] }))
        .collect();
    let draw = json!({ "brush": "Draw", "part": "sphere", "samples": samples, "size": 0.4 });
    s.ok("brush.stroke", draw);
    let after = s.mesh("sphere");
    let (mut moved, mut held) = (0, 0);
    let mask = mask_of(&before);
    for ((p, q), &m) in before.positions.iter().zip(&after.positions).zip(mask) {
        let same = p.map(f32::to_bits) == q.map(f32::to_bits);
        moved += usize::from(!same);
        if is(m, 1.0) {
            assert!(same, "a fully masked point at {p:?} moved");
            held += usize::from(p[2] > 0.6 && p[1] < -0.5);
        }
    }
    assert!(moved > 0, "the stroke moved nothing");
    assert!(held > 0, "the stroke reached no fully masked point");
}

#[test]
fn a_masked_part_subdivided_keeps_its_mask_and_a_stroke_leaves_it_alone() {
    let (mut s, edge) = masked_sphere();
    let made = s.ok("level.subdivide", json!({}));
    assert_eq!(made.result["count"], json!(2));
    let top = s.mesh("sphere");
    assert_eq!(top.faces.len(), 4 * 3456);
    let full = mask_where_it_was(&top, edge);
    assert!(full > 1000, "{full} points fully masked");
    stroke_leaves_the_masked_area_alone(&mut s);
}

#[test]
fn a_masked_part_remeshed_keeps_its_mask_and_a_stroke_leaves_it_alone() {
    let (mut s, edge) = masked_sphere();
    let out = s.ok("mesh.voxel_remesh", json!({ "resolution": 48 }));
    let cell = out.result["voxel_size"].as_f64().unwrap_or(0.0) as f32;
    let new = s.mesh("sphere");
    // A new point's source lies within about a cell of it, and its triangle's corners within
    // an edge of that.
    let full = mask_where_it_was(&new, edge + 2.0 * cell);
    assert!(full > 500, "{full} points fully masked");
    stroke_leaves_the_masked_area_alone(&mut s);
}
