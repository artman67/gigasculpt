//! The UV hint columns (decision 0061 section 6, amended 2026-10-13, step UV7): UV Hint and UV
//! Density paint `uv_hint` and `uv_density` from -1 to 1 on the 2^-24 grid, Alt moves them toward
//! 0 from either side, the loader takes a value in the column's range and names it, a hint
//! stroke on a lower level carries up with its sign, a subdivision step and an edge split keep
//! the means on the grid, and a sculpt or paint stroke leaves both columns bit for bit.

use gs_core::brush::BrushPreset;
use gs_core::{Levels, MeshStore, Settings, StrokeSample, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::brush_kit::{arc, floats, ico, params, preset, run};
use crate::common::{mix, ok, refine, small_nodes, sphere, store, unit};
use crate::levels::stack;

/// Each hint brush and the column it paints.
const BRUSHES: [(&str, &str); 2] = [("uv-hint", "uv_hint"), ("uv-density", "uv_density")];
const HINTS: [&str; 2] = ["uv_hint", "uv_density"];
/// Steps per unit of the grid.
const STEP: f32 = 16_777_216.0;

/// Whether a hint column may hold `x`: from -1 to 1, a whole number of grid steps, and not -0.
fn held(x: f32) -> bool {
    x.abs() <= 1.0 && (x * STEP).fract() == 0.0 && x.to_bits() != (-0.0f32).to_bits()
}

/// The shipped preset `stem` painting `value`.
fn painting(stem: &str, value: f64) -> BrushPreset {
    let text = preset(stem).text;
    let shipped = "value = 1.0";
    assert!(text.contains(shipped), "{stem} ships value 1");
    ok(BrushPreset::from_toml(
        &text.replace(shipped, &format!("value = {value:?}")),
    ))
}

/// A stroke across the front of the unit sphere, `radius` wide.
fn across(radius: f32) -> Vec<StrokeSample> {
    let (a, b) = (Vec3::new(-0.3, 0.0, 1.0), Vec3::new(0.3, 0.05, 1.0));
    arc(a, b, 6, radius)
}

/// Two strokes of `brush` [`across`], with Alt when `invert`: one stroke leaves its centre
/// about 1.5e-5 short of the value (each dab goes part of the way), a second one closes it.
fn twice(m: &mut MeshStore, brush: &BrushPreset, invert: bool, radius: f32) {
    for _ in 0..2 {
        let mut prm = params(None);
        prm.invert = invert;
        run(m, brush, prm, &across(radius));
    }
}

#[test]
fn a_hint_brush_writes_its_value_at_the_centre_on_the_grid() {
    for (stem, column) in BRUSHES {
        for value in [-1.0, 1.0] {
            let mut m = store(&ico(4), small_nodes());
            twice(&mut m, &painting(stem, value), false, 0.3);
            let after = m.to_plain();
            let v = floats(&after, column);
            let near = v
                .iter()
                .map(|&x| (f64::from(x) - value).abs())
                .fold(2.0, f64::min);
            assert!(
                near == 0.0,
                "{stem} at {value}: the centre reaches it, off by {near}"
            );
            let off = v.iter().filter(|&&x| !held(x)).count();
            assert_eq!(off, 0, "{stem} at {value}: values off the grid or range");
            let other = HINTS.into_iter().find(|&c| c != column).unwrap_or_default();
            assert!(
                !after.attributes.contains_key(other),
                "{stem} paints {column} alone"
            );
        }
    }
}

#[test]
fn alt_moves_a_hint_toward_0_from_either_side() {
    for (stem, column) in BRUSHES {
        for value in [-1.0f32, 1.0] {
            let brush = painting(stem, f64::from(value));
            let mut m = store(&ico(4), small_nodes());
            twice(&mut m, &brush, false, 0.3);
            let before = floats(&m.to_plain(), column);
            twice(&mut m, &brush, true, 0.15);
            let after = floats(&m.to_plain(), column);
            let mut erased = 0;
            for (i, (&b, &a)) in before.iter().zip(&after).enumerate() {
                let toward = a.abs() <= b.abs() && a * b >= 0.0 && held(a);
                assert!(toward, "{stem} from {value}: point {i} went {b} to {a}");
                erased += usize::from(b.to_bits() == value.to_bits() && a == 0.0);
            }
            assert!(erased > 0, "{stem} from {value}: Alt erased its centre");
        }
    }
}

/// Why `stem` with `value` (and `op`, when given) does not load, or "" when it does.
fn refused(stem: &str, value: &str, op: Option<&str>) -> String {
    let mut text = (preset(stem).text).replace("value = 1.0", &format!("value = {value}"));
    if let Some(op) = op {
        text = text.replace("op = \"set\"", &format!("op = \"{op}\""));
    }
    BrushPreset::from_toml(&text)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default()
}

#[test]
fn the_loader_takes_a_hint_from_minus_1_to_1_and_a_mask_from_0_naming_the_range() {
    for (stem, _) in BRUSHES {
        for value in ["-1.0", "-0.25", "0.0", "1.0"] {
            assert_eq!(refused(stem, value, None), "", "{stem} takes {value}");
        }
        for value in ["-1.5", "1.01", "nan"] {
            let why = refused(stem, value, None);
            assert!(why.contains("from -1 to 1"), "{stem} at {value}: {why}");
        }
        for op in ["blur", "sharpen"] {
            let why = refused(stem, "1.0", Some(op));
            assert!(why.contains("only apply to the mask"), "{stem} {op}: {why}");
        }
    }
    let why = refused("mask-pen", "-0.5", None);
    assert!(why.contains("from 0 to 1"), "Mask Pen at -0.5: {why}");
}

/// A lumpy 16 by 16 cube sphere divided twice, at level 1.
fn below_the_top() -> Levels {
    let mut l = stack(&sphere(16, 0.3, 31, false), &Settings::default());
    ok(l.divide(true));
    ok(l.divide(true));
    ok(l.set(1));
    l
}

/// The top's `column` after a wide stroke of `brush` on level 1, carried to level 2.
fn carried(brush: &BrushPreset, column: &str) -> Vec<f32> {
    let mut l = below_the_top();
    run(l.store_mut(), brush, params(None), &across(0.8));
    let report = ok(l.set(2));
    let n = report.values;
    assert!(n > 4096, "more than one chunk was carried, got {n}");
    floats(&l.top().to_plain(), column)
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn a_hint_stroke_on_a_lower_level_carries_up_with_its_sign_on_the_grid() {
    for (stem, column) in BRUSHES {
        let brush = painting(stem, -1.0);
        let one = gs_par::with_threads(1, || carried(&brush, column));
        let many = gs_par::with_threads(24, || carried(&brush, column));
        assert!(bits(&one) == bits(&many), "{stem}: 1 and 24 threads differ");
        assert!(one.iter().any(|&x| x < -0.99), "{stem}: the centre came up");
        let soft = one.iter().filter(|&&x| x < 0.0 && x > -0.99).count();
        assert!(soft > 0, "{stem}: the soft edge came up");
        let off = one.iter().filter(|&&x| !held(x)).count();
        assert_eq!(off, 0, "{stem}: values off the grid or range at the top");
    }
}

/// The mask after [`carried`]'s Mask Pen stroke, folded into one number.
fn carried_mask() -> u64 {
    let mask = carried(&preset("mask-pen").preset, "mask");
    (mask.iter()).fold(0, |h, x| mix(h ^ u64::from(x.to_bits())))
}

#[test]
fn a_mask_stroke_on_a_lower_level_carries_up_as_before_to_the_bit() {
    let one = gs_par::with_threads(1, carried_mask);
    let many = gs_par::with_threads(24, carried_mask);
    assert_eq!(one, many, "1 and 24 threads");
    // Recorded at 9e45b530, before the hint columns.
    assert_eq!(
        one, 18292381406152781779,
        "the carried mask, as before this step"
    );
}

/// `m` with both hint columns: a value from -1 to 1 on the grid at random on every point.
fn hinted(mut m: PlainMesh) -> PlainMesh {
    for (k, column) in HINTS.into_iter().enumerate() {
        let values = (0..m.positions.len() as u64)
            .map(|i| (unit(mix(i * 3 + k as u64)) * STEP).round() / STEP + 0.0)
            .collect();
        let (domain, values) = (AttrDomain::Vertex, AttrValues::F32(values));
        m.attributes
            .insert(column.into(), Attribute { domain, values });
    }
    m
}

/// Fails naming how many of `s`'s hint values are off the grid or out of range, or a column
/// that lost one sign, which would test less.
fn all_held(s: &MeshStore, what: &str) {
    let plain = s.to_plain();
    for column in HINTS {
        let v = floats(&plain, column);
        let off = v.iter().filter(|&&x| !held(x)).count();
        assert_eq!(off, 0, "{what}: {column} has {off} of {} off", v.len());
        let signs = v.iter().any(|&x| x < 0.0) && v.iter().any(|&x| x > 0.0);
        assert!(signs, "{what}: {column} keeps both signs");
    }
}

#[test]
fn a_subdivision_step_and_an_edge_split_keep_the_hints_means_on_the_grid() {
    for smooth in [true, false] {
        let mut l = stack(&hinted(sphere(6, 0.3, 31, false)), &Settings::default());
        for step in 1..=2 {
            ok(l.divide(smooth));
            all_held(l.top(), &format!("smooth {smooth}, step {step}"));
        }
    }
    let mut s = store(&hinted(sphere(6, 1.0, 9, false)), small_nodes());
    let points = s.to_plain().positions.len();
    refine(&mut s, Vec3::new(0.0, 0.0, 1.0), 0.8, 0.05, 4);
    let split = s.to_plain().positions.len() - points;
    assert!(split > 50, "{split} edges split");
    all_held(&s, "after the splits");
}

#[test]
fn a_sculpt_stroke_and_a_paint_stroke_leave_both_hint_columns_to_the_bit() {
    let mut m = store(&ico(4), small_nodes());
    run(
        &mut m,
        &painting("uv-hint", -1.0),
        params(None),
        &across(0.4),
    );
    run(
        &mut m,
        &painting("uv-density", 1.0),
        params(None),
        &across(0.4),
    );
    let before = m.to_plain();
    for stem in ["draw", "paint-draw"] {
        run(&mut m, &preset(stem).preset, params(None), &across(0.4));
    }
    let after = m.to_plain();
    assert!(before.positions != after.positions, "Draw moved points");
    assert!(after.attributes.contains_key("color"), "Paint painted");
    for column in HINTS {
        let (b, a) = (floats(&before, column), floats(&after, column));
        assert!(b.iter().any(|&x| x != 0.0), "{column} was painted");
        assert!(bits(&b) == bits(&a), "{column} changed");
    }
}
