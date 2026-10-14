//! Unfreeze carries every point column the old top holds (decision 0030, amended 2026-10-13):
//! UV hints painted at the top of three levels come back from the old top where each point
//! landed, as the color and the mask do, with their sign and on the hint range; a column only the
//! frozen base holds stays as the base had it; a mirror map is not carried; and the color and the
//! mask come back bit for bit as before.

use gs_protocol::{AttrValues, PlainMesh};
use serde_json::{Value, json};

use crate::support::agent;

/// The new top's points, faces, color and mask as main at `bfd76307` (before unfreeze carried
/// every point column) hashed them. A change to the brushes' math or to the projection changes
/// it; record it again then, from the engine just before that change.
const MAIN_HASH: &str = "565c556dba82809b7be4b1bc8ed925ebd3e2685eaa0b9f182955acd9b2d5810d";

/// A stroke of `brush` toward `value` over the sphere near `x`: radius 0.3, so the strokes at
/// 0.55 and -0.55 never meet.
fn across(brush: &str, x: f32, value: f32) -> Value {
    let samples = [[x - 0.05, -0.1], [x + 0.05, 0.1]]
        .map(|[x, y]| json!({ "position": [x, y, (1.0 - x * x - y * y).sqrt()] }));
    let settings = json!({ "kernel": { "value": value } });
    json!({ "part": "sphere", "brush": brush, "size": 0.3, "samples": samples, "settings": settings })
}

/// How many points of `m` break what the strokes painted (the hint is the mask with the sign of
/// x, the density minus the hint, the hint on its range's grid of 2^-24 within -1 and 1), and how
/// many points are masked over half on each side, +x first. A column not held reads 0.
fn reads(m: &PlainMesh) -> (usize, [usize; 2]) {
    let [hint, density, mask] = ["uv_hint", "uv_density", "mask"].map(|name| {
        match m.attributes.get(name).map(|a| &a.values) {
            Some(AttrValues::F32(v)) => v.clone(),
            _ => vec![0.0; m.positions.len()],
        }
    });
    let grid = |h: f32| h.abs() <= 1.0 && (h * 16_777_216.0).fract() == 0.0;
    // Bit for bit, -0 read as 0 as the range stores it.
    let same = |a: f32, b: f32| a.to_bits() == (b + 0.0).to_bits();
    let mut out = (0, [0, 0]);
    for (i, p) in m.positions.iter().enumerate() {
        let s = if p[0] > 0.0 { 1.0 } else { -1.0 };
        let (h, d, k) = (hint[i], density[i], mask[i]);
        if !same(h, s * k) || !same(d, -h) || !grid(h) {
            out.0 += 1;
        }
        if k > 0.5 {
            out.1[usize::from(s < 0.0)] += 1;
        }
    }
    out
}

/// Every point column of the old top lands with its points: hints painted toward 1 on +x and -1
/// on -x, the density the other way and the mask on both are found again on the new top in the
/// same relation the strokes left them in, so each was carried from where its point landed.
#[test]
fn unfreeze_brings_the_hints_back_from_the_old_top_with_their_sign() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    // A map left registered at the top and holding nothing: carried, it would come back broken.
    s.ok("symmetry.map", json!({ "axis": "x" }));
    s.ok("level.subdivide", json!({}));
    s.ok("level.subdivide", json!({}));
    // The hints before the mask, so the mask holds no hint stroke back.
    for (brush, sign) in [("UV Hint", 1.0), ("UV Density", -1.0), ("Mask Pen", 0.0)] {
        for x in [0.55f32, -0.55] {
            let value = if sign == 0.0 { 1.0 } else { sign * x.signum() };
            s.ok("brush.stroke", across(brush, x, value));
        }
    }
    let (wrong, masked) = reads(&s.mesh("sphere"));
    assert!(
        wrong == 0 && masked.iter().all(|&n| n >= 20),
        "the old top: {wrong} wrong, {masked:?} masked"
    );
    s.ok("level.freeze", json!({}));
    s.ok("mesh.voxel_remesh", json!({ "resolution": 16 }));
    // The color only the frozen base holds.
    let mut paint = across("Paint", 0.0, 0.0);
    paint["settings"] = json!({});
    paint["colors"] = json!({ "main": "#ff3300", "secondary": "#102030" });
    s.ok("brush.stroke", paint);
    let back = s.ok("level.unfreeze", json!({ "distance": "5%" }));
    assert_eq!(back.result["how"], json!("projected"));
    let new = s.mesh("sphere");
    let mut kept = new.clone();
    kept.attributes
        .retain(|name, _| name == "color" || name == "mask");
    let hash = s.query("mesh.hash", json!({ "mesh": kept }));
    assert_eq!(hash["hash"], json!(MAIN_HASH), "the color and the mask");
    let colors = match new.attributes.get("color").map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.clone(),
        _ => Vec::new(),
    };
    assert!(
        colors.iter().any(|&c| c != colors[0]),
        "the base's color came back, painted and not"
    );
    assert!(!new.attributes.contains_key("mirror.x"), "no map carried");
    let (wrong, masked) = reads(&new);
    assert!(
        wrong == 0 && masked.iter().all(|&n| n >= 20),
        "the new top: {wrong} of {} points wrong, {masked:?} masked",
        new.positions.len()
    );
}
