//! Where a dab stroke lays its dabs, and what that does to the surface (decision 0036 sections 1
//! and 4): Dots' and Freehand's spacings at constant speed, a Step's dab centers from the press,
//! Stroke Smoothing's bend on a zigzag, Directional, a fast stroke that leaves no gap, and
//! spacing that never changes how far a stroke moves the surface in total.

use gs_core::brush::BrushPreset;
use gs_core::stroke::placed;
use gs_core::{StrokeSample, Vec3};
use gs_protocol::PlainMesh;

use crate::brush_checks::total;
use crate::brush_kit::{after, at, grid, params, polyline, preset, store, to_segment};
use crate::common::{mix, ok, unit};

/// Draw with its stroke replaced by a dab stroke with `settings`.
fn draw_with(settings: &str) -> BrushPreset {
    let text = preset("draw").text;
    let cut = text.find("[stroke]").unwrap_or(text.len());
    let stroke = format!("[stroke]\nclass = \"dab\"\n{settings}");
    ok(BrushPreset::from_toml(&format!("{}{stroke}", &text[..cut])))
}

/// The `n` grid in the order the store keeps it, so a stroke's result lines up vertex for vertex.
fn flat(n: u32) -> PlainMesh {
    store(&grid(n)).to_plain()
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).len2().sqrt()
}

/// A constant-speed stroke: samples at most `gap` apart, evenly, along the straight line from
/// `a` to `b`, at full pressure with brush radius `r`.
fn line(a: Vec3, b: Vec3, gap: f32, r: f32) -> Vec<StrokeSample> {
    let pieces = (dist(a, b) / gap).ceil() as usize;
    polyline(&[(a, 1.0, r), (b, 1.0, r)], pieces)
}

/// The dabs a stroke lays as it moves: without the press's tap dab, first, and the end's
/// unfinished cell, last.
fn walked(dabs: &[(StrokeSample, f32)]) -> Vec<Vec3> {
    dabs[1..dabs.len() - 1].iter().map(|d| d.0.pos).collect()
}

#[test]
fn a_constant_speed_stroke_lays_dots_dabs_a_fifth_of_a_radius_apart_and_freehand_three_quarters() {
    let r = 0.1;
    // Not a whole number of steps long, so the stroke ends inside a cell.
    let (a, b) = (Vec3::new(-0.8, 0.1, 0.0), Vec3::new(0.805, 0.1, 0.0));
    let s = line(a, b, 0.013, r);
    // Dots' 0.2 and FreeHand's 0.75, read in radii, are spacings 0.1 and 0.375.
    for (spacing, apart) in [(0.1, 0.2), (0.375, 0.75)] {
        let dabs = placed(&draw_with(&format!("spacing = {spacing}\n")), &s);
        let centers = walked(&dabs);
        let first = dist(centers[0], a);
        assert!(
            first < 1e-6,
            "spacing {spacing}: the first lands {first} from the press"
        );
        for w in centers.windows(2) {
            let gap = dist(w[0], w[1]) / r;
            assert!(
                (gap - apart).abs() < 1e-3,
                "spacing {spacing}: dabs {gap} radii apart, not {apart}"
            );
        }
        // Every length lands in one dab: after the tap's, they add up to the stroke's.
        let length: f32 = dabs[1..].iter().map(|d| d.1).sum();
        assert!(
            (length - dist(a, b)).abs() < 1e-4,
            "spacing {spacing}: the dabs stand for {length}"
        );
    }
}

#[test]
fn a_step_puts_dab_centers_that_many_diameters_apart_so_step_1_stamps_just_touch() {
    let r = 0.05;
    let (a, b) = (Vec3::new(-0.9, 0.0, 0.0), Vec3::new(0.93, 0.0, 0.0));
    let s = line(a, b, 0.011, r);
    // Step is the dab spacing (decision 0036 section 4). Set on Draw, which strokes as Dots, it
    // wins over the 0.1 Draw names, with the string off and on. At Step 1 round stamps of radius
    // r sit 2r apart: they just touch.
    for string in ["", "smoothing = { radius = 2 }\n"] {
        for step in [1.0, 1.5, 2.0] {
            let p = draw_with(&format!("spacing = {step}\n{string}"));
            let centers = walked(&placed(&p, &s));
            let first = dist(centers[0], a);
            assert!(
                first < 1e-6,
                "Step {step}: the first lands {first} from the press"
            );
            assert!(centers.len() >= 3, "Step {step}: {centers:?}");
            for w in centers.windows(2) {
                let gap = dist(w[0], w[1]) / (2.0 * r);
                assert!(
                    (gap - step).abs() < 1e-3,
                    "Step {step} {string}: centers {gap} diameters apart"
                );
            }
        }
    }
}

/// The largest curvature along `p`: at each point, of the circle through it and the points `k`
/// before and after it.
fn bend(p: &[Vec3], k: usize) -> f32 {
    (k..p.len() - k)
        .map(|i| {
            let (a, b, c) = (p[i - k], p[i], p[i + k]);
            let twice_area = (b - a).cross(c - b).len2().sqrt();
            2.0 * twice_area / (dist(a, b) * dist(b, c) * dist(a, c))
        })
        .fold(0.0, f32::max)
}

/// Pointer samples along a zigzag of `legs` legs `leg` long, running `half` degrees either side
/// of +x, so each corner turns by twice that; samples `gap` apart.
fn zigzag(half: f32, leg: f32, legs: usize, gap: f32, r: f32) -> Vec<StrokeSample> {
    let (sin, cos) = (libm::sinf(half.to_radians()), libm::cosf(half.to_radians()));
    let mut p = Vec3::new(-1.0, 0.0, 0.0);
    let mut corners = vec![(p, 1.0, r)];
    for i in 0..legs {
        let up = if i % 2 == 0 { sin } else { -sin };
        p += Vec3::new(cos, up, 0.0) * leg;
        corners.push((p, 1.0, r));
    }
    polyline(&corners, (leg / gap).round() as usize)
}

#[test]
fn stroke_smoothing_bends_a_zigzag_no_tighter_than_one_over_the_string() {
    let (r, string) = (0.05, 2.0);
    let big_r = string * r;
    // Dabs a hundredth of the string apart trace the paint path.
    let p = draw_with(&format!(
        "spacing = 0.01\nsmoothing = {{ radius = {string} }}\n"
    ));
    let path = |half: f32| walked(&placed(&p, &zigzag(half, 4.0 * big_r, 6, big_r / 40.0, r)));
    // Points 20 dabs apart span a fifth of the string.
    let k = 20;
    // A string pulled along moves its end on a tractrix, bending at tan(theta) / R: corners of
    // 40 and 45 degrees stay within 1/R.
    for half in [20.0, 22.5] {
        let most = bend(&path(half), k) * big_r;
        assert!(
            most <= 1.0,
            "corners of {}: curvature {most} / R",
            2.0 * half
        );
    }
    // At a right angle the paint point stops and turns where it stands, past the bound: the
    // measure would see a zigzag the string did not smooth.
    let most = bend(&path(45.0), k) * big_r;
    assert!(most > 1.5, "corners of 90: curvature {most} / R");
}

#[test]
fn with_directional_on_going_back_along_the_path_adds_nothing() {
    let mesh = flat(60);
    let r = 0.15;
    let (a, b) = (Vec3::new(-0.5, 0.05, 0.0), Vec3::new(0.5, -0.05, 0.0));
    let out = line(a, b, 0.02, r);
    let mut out_and_back = out.clone();
    out_and_back.extend(line(b, a, 0.02, r).into_iter().skip(1));
    let moved =
        |p: &BrushPreset, s: &[StrokeSample]| total(&mesh, &after(&mesh, p, params(None), s));
    let on = draw_with("directional = true\n");
    let (there, again) = (moved(&on, &out), moved(&on, &out_and_back));
    assert!(there > 0.0, "the stroke moved the surface");
    assert!(
        (again - there).abs() <= 0.02 * there,
        "Directional: out {there}, out and back {again}"
    );
    // Off, going back counts again (measured: 1.79 times as much).
    let off = draw_with("");
    let (there, again) = (moved(&off, &out), moved(&off, &out_and_back));
    assert!(again > 1.5 * there, "out {there}, out and back {again}");
}

#[test]
fn a_fast_freehand_stroke_leaves_no_gap_between_its_samples() {
    let mesh = flat(80);
    let r = 0.15;
    // Three samples six radii and more apart: a pen moving fast.
    let c = [
        Vec3::new(-0.75, -0.3, 0.0),
        Vec3::new(0.0, 0.3, 0.0),
        Vec3::new(0.75, -0.2, 0.0),
    ];
    let s: Vec<StrokeSample> = c.iter().map(|&p| at(p, r)).collect();
    let done = after(&mesh, &draw_with("spacing = 0.375\n"), params(None), &s);
    let rise: Vec<(Vec3, f32)> = (mesh.positions.iter().zip(&done.positions))
        .map(|(q, d)| (Vec3::from_array(*q), d[2] - q[2]))
        .collect();
    let most = rise.iter().map(|x| x.1).fold(0.0, f32::max);
    // Every point within half a radius of the path between the first and last samples rose, by
    // a tenth of the most at least (measured: 0.21, outside the middle corner). A gap leaves 0.
    let near = |v: Vec3| to_segment(v, c[0], c[1]).min(to_segment(v, c[1], c[2])) <= 0.5 * r;
    let inside = |v: Vec3| dist(v, c[0]) > r && dist(v, c[2]) > r;
    let along: Vec<f32> = (rise.iter())
        .filter(|x| near(x.0) && inside(x.0))
        .map(|x| x.1)
        .collect();
    assert!(along.len() > 100, "{} points along the path", along.len());
    let low = along.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(low > 0.1 * most, "a point rose {low}, the most {most}");
}

#[test]
fn no_spacing_changes_how_far_a_stroke_moves_the_surface_in_total() {
    let mesh = flat(80);
    let moved = |spacing: f32, s: &[StrokeSample]| {
        let p = draw_with(&format!("spacing = {spacing}\n"));
        total(&mesh, &after(&mesh, &p, params(None), s))
    };
    for case in 0..12_u64 {
        // Numbers from -1 to 1 and from 0 to 1, the case's own.
        let u = |k: u64| unit(mix(case * 16 + k));
        let h = |k: u64| 0.5 + 0.5 * u(k);
        let spacing = 0.02 + 0.98 * h(0);
        let r = 0.1 + 0.08 * h(1);
        // A stroke of three legs, with corners anywhere within 0.55 of the middle, and from one
        // to six samples a leg.
        let corner = |k: u64| (Vec3::new(u(2 * k + 2), u(2 * k + 3), 0.0) * 0.55, 1.0, r);
        let corners = [corner(0), corner(1), corner(2), corner(3)];
        let s = polyline(&corners, 1 + (h(12) * 5.0) as usize);
        let (want, got) = (moved(0.1, &s), moved(spacing, &s));
        // Within 3% (measured: 1.6% at worst, case 2).
        assert!(
            (got - want).abs() <= 0.03 * want,
            "case {case}: spacing {spacing} moved {got}, spacing 0.1 {want}"
        );
    }
}
