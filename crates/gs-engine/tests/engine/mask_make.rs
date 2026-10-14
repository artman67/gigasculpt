//! `mask.make` and `mask.filter`'s point maps against the catalog's checks (decision 0026, 7),
//! through commands the way an agent calls them: the cavity maker on a grooved plane and the
//! combine step's intensity, blend and blur, masking all but some face groups, features on a
//! hole's edge and where groups meet, and boost, dilute and normalize. Exact undo is the claims
//! harness's (`claims`), which runs each example.

use std::collections::BTreeSet;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::{Session, fixtures};
use serde_json::{Value, json};

use crate::groups::{corners, groups, mask};
use crate::support::agent;

/// A flat 16 by 16 grid over x and z from -1 to 1 with a groove 0.1 deep along x = 0.
fn grooved() -> PlainMesh {
    let mut m = fixtures::patch(16);
    for p in &mut m.positions {
        p[1] = if p[0].abs() < 1e-6 { -0.1 } else { 0.0 };
    }
    m
}

/// The mask after `steps` on a fresh session holding `mesh`.
fn masked(mesh: &PlainMesh, steps: &[(&str, Value)]) -> Vec<f64> {
    let mut s = agent();
    s.add_part("part", mesh);
    for (command, args) in steps {
        s.ok(command, args.clone());
    }
    mask(&s.mesh("part"))
}

/// The face groups around each point.
fn around(m: &PlainMesh) -> Vec<BTreeSet<u32>> {
    let g = groups(m);
    let mut out = vec![BTreeSet::new(); m.positions.len()];
    for (f, &id) in g.iter().enumerate() {
        for v in corners(m, f) {
            out[v].insert(id);
        }
    }
    out
}

#[test]
fn cavity_masks_a_groove_above_the_plane_and_intensity_blend_and_blur_shape_it() {
    let m = grooved();
    // Points away from the open border, where the walk is cut short.
    let pick = |far: bool| -> Vec<usize> {
        let p = &m.positions;
        (0..p.len())
            .filter(|&v| p[v][2].abs() < 0.6 && (p[v][0].abs() > 0.6) == far)
            .filter(|&v| far || p[v][0].abs() < 1e-6)
            .collect()
    };
    let (groove, plane) = (pick(false), pick(true));
    assert!(groove.len() > 5 && plane.len() > 20);
    // `mask.make` with these settings, by cavity unless they give a maker.
    let cavity = |mut args: Value| {
        if args.get("maker").is_none() {
            args["maker"] = json!({ "by": "cavity" });
        }
        ("mask.make", args)
    };
    let r = masked(&m, &[cavity(json!({}))]);
    let low = |k: &[f64], at: &[usize]| at.iter().map(|&v| k[v]).fold(1.0, f64::min);
    let high = |k: &[f64], at: &[usize]| at.iter().map(|&v| k[v]).fold(0.0, f64::max);
    assert!(
        low(&r, &groove) > 0.5,
        "the groove masked: {}",
        low(&r, &groove)
    );
    assert!(high(&r, &plane) <= 0.0, "the plane free");
    let flipped = masked(
        &m,
        &[cavity(
            json!({ "maker": { "by": "cavity", "contrast": -50 } }),
        )],
    );
    assert!(
        high(&flipped, &groove) < low(&flipped, &plane),
        "a negative contrast flips it"
    );
    let half = masked(&m, &[cavity(json!({ "intensity": 50 }))]);
    assert!(
        half.iter().all(|&k| k <= 0.5),
        "intensity 50 writes at most 0.5"
    );
    assert!(low(&half, &groove) > 0.25);
    let set = ("mask.set", json!({ "value": 0.4 }));
    let replaced = masked(&m, &[set.clone(), cavity(json!({}))]);
    assert_eq!(replaced, r, "blend 100 replaces the old mask");
    let averaged = masked(&m, &[set, cavity(json!({ "blend": 50 }))]);
    for (v, (&a, &x)) in averaged.iter().zip(&r).enumerate() {
        assert!(
            (a - f64::midpoint(0.4, x)).abs() < 1e-6,
            "point {v}: blend 50 averages"
        );
    }
    let blurred = masked(&m, &[cavity(json!({ "blur": 2 }))]);
    assert!(
        groove.iter().all(|&v| blurred[v] < r[v]),
        "blur softens the groove"
    );
    assert!(
        (0..r.len()).any(|v| r[v] <= 0.0 && blurred[v] > 0.0),
        "and spreads it"
    );
}

#[test]
fn mask_all_but_groups_masks_every_point_off_them_and_replaces_the_old_mask() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    s.ok("group.make", json!({ "maker": { "by": "normals" } }));
    let held = around(&s.mesh("cube"));
    // Two groups that meet, from the first point where two do.
    let meet: Vec<u32> = held
        .iter()
        .find(|h| h.len() > 1)
        .into_iter()
        .flatten()
        .copied()
        .take(2)
        .collect();
    assert_eq!(meet.len(), 2);
    for keep in [vec![2], meet] {
        s.ok("mask.set", json!({ "value": 0.3 }));
        let keep_args = json!({ "maker": { "by": "groups", "keep": keep } });
        s.ok("mask.make", keep_args);
        let want: Vec<f64> = (held.iter())
            .map(|h| {
                if keep.iter().any(|id| h.contains(id)) {
                    0.0
                } else {
                    1.0
                }
            })
            .collect();
        assert_eq!(mask(&s.mesh("cube")), want, "keeping {keep:?}");
    }
}

#[test]
fn features_mask_exactly_a_holes_edge_and_where_groups_meet_keeping_the_rest() {
    let mut holed = fixtures::cube(3);
    let hole = corners(&holed, 4);
    holed.faces.remove(4);
    let k = masked(
        &holed,
        &[
            ("mask.set", json!({ "value": 0.5 })),
            ("mask.make", json!({ "maker": { "by": "features" } })),
        ],
    );
    let want: Vec<f64> = (0..k.len())
        .map(|v| if hole.contains(&v) { 1.0 } else { 0.5 })
        .collect();
    assert_eq!(k, want, "the hole's edge loop, the rest kept");
    let mut s: Session = agent();
    s.add_part("cube", &fixtures::cube(3));
    s.ok("group.make", json!({ "maker": { "by": "normals" } }));
    let only_groups = json!({ "maker": { "by": "features", "border": false } });
    s.ok("mask.make", only_groups);
    let m = s.mesh("cube");
    let want: Vec<f64> = (around(&m).iter())
        .map(|h| f64::from(u8::from(h.len() > 1)))
        .collect();
    assert_eq!(mask(&m), want, "the cube's edges, where sides meet");
    assert_eq!(want.iter().filter(|&&x| x > 0.5).count(), 8 + 12 * 2);
}

#[test]
fn boost_dilute_and_normalize_move_each_value_alone() {
    let filtered = |values: &dyn Fn(usize) -> f32, args: Value| -> (Vec<f64>, Vec<f64>) {
        let mut m = fixtures::patch(8);
        let n = m.positions.len();
        let values = AttrValues::F32((0..n).map(values).collect());
        let domain = AttrDomain::Vertex;
        m.attributes
            .insert("mask".into(), Attribute { domain, values });
        let before = masked(&m, &[]);
        let after = masked(&m, &[("mask.filter", args)]);
        (before, after)
    };
    let steps = |v: usize| [0.0, 0.25, 0.5, 1.0][v % 4];
    for (op, curve) in [
        ("boost", f64::sqrt as fn(f64) -> f64),
        ("dilute", |m| m * m),
    ] {
        let (before, after) = filtered(&steps, json!({ "op": op }));
        for (v, (&b, &a)) in before.iter().zip(&after).enumerate() {
            assert!((a - curve(b)).abs() < 1e-6, "{op} point {v}: {b} to {a}");
        }
    }
    // Boosted again and again, the mask hardens and what was free stays free.
    let (before, after) = filtered(&steps, json!({ "op": "boost", "passes": 20 }));
    for (v, (&b, &a)) in before.iter().zip(&after).enumerate() {
        assert!(
            if b > 0.0 { a > 0.99 } else { a <= 0.0 },
            "point {v}: {b} to {a}"
        );
    }
    let n = fixtures::patch(8).positions.len();
    let ramp = |v: usize| 0.2 + 0.4 * v as f32 / (n - 1) as f32;
    let normalize = || json!({ "op": "normalize" });
    let (before, after) = filtered(&ramp, normalize());
    for (v, (&b, &a)) in before.iter().zip(&after).enumerate() {
        assert!(
            (a - (b - 0.2) / 0.4).abs() < 1e-5,
            "normalize point {v}: {b} to {a}"
        );
    }
    assert!(after.windows(2).all(|w| w[0] <= w[1]), "order kept");
    assert!(after[0] <= 0.0 && after[n - 1] >= 1.0, "0 to 1");
    let (before, after) = filtered(&|_| 0.4, normalize());
    assert_eq!(after, before, "a flat mask is unchanged");
}
