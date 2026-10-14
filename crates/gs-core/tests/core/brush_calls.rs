//! Measurements for the calls on brushes and the Plane kernel, decided in decision 0062: each
//! keeps the behavior built, since the original's is undocumented until it is tried by
//! hand (research 29), and the Checks are reworded to it at the next catalog pass. Each test holds
//! what is true today, prints the numbers and names the call in its comment. The calls: R11
//! (which noise and density Smooth Strong's Check means), R12 (Move Elastic's spread), R13
//! (Smooth Polish against today's Smooth), Clay Line's end caps, the Plane kernel's full dab and
//! cap rule, and Smooth Strong's Live Detail merge.

use std::collections::BTreeMap;

use gs_core::Vec3;
use gs_core::brush::{BrushPreset, KernelSpec, Mode, Side, StrokeSpec};
use gs_protocol::PlainMesh;
use gs_testkit::fixtures;

use crate::brush_behaviors::{edge_spread, pull};
use crate::brush_checks::{
    Height, along, edge_width, jitter, relief, ridge_tops, ridges, roughness,
};
use crate::brush_kit::{after, at, params, preset, shipped, store};
use crate::common::ok;

/// The grid and radius the brush measurements use: 20 edges to the radius, near research 21's 22.
const N: u32 = 160;
const R: f32 = 0.25;

/// One edge of the `n` grid.
fn edge(n: u32) -> f32 {
    2.0 / n as f32
}

/// What is left of the roughness of surface `z` on the `n` grid after one stroke of `stem` along
/// y = 0: the height RMS within half the radius of the line, after over before.
fn left(n: u32, z: Height, stem: &str) -> f64 {
    let mesh = relief(n, z);
    let a = after(
        &mesh,
        &preset(stem).preset,
        params(None),
        &along(z, -0.6, 0.6, R),
    );
    roughness(&a, 0.5 * R) / roughness(&mesh, 0.5 * R)
}

/// R11: Smooth Strong's Check asks that one pass leave under half the noise Smooth leaves
/// for the same stroke, and says neither which noise nor how fine the mesh. At 20 edges to the
/// radius it holds on both noises: on white noise half an edge high Smooth leaves 0.30 of it
/// and Smooth Strong 0.11, 0.36 of Smooth's; on a ripple one edge high with crests 11 edges
/// apart across the stroke (research 21's ripple) Smooth leaves 0.86 and Smooth Strong 0.21,
/// 0.24 of Smooth's. At 5 edges to the radius the same white noise is left at 0.27 and 0.15,
/// 0.56 of Smooth's, and the clause fails: the two-ring mean reaches as far as the brush's own
/// falloff there (0.48 at 10 edges, 0.35 at 12.5). The catalog's earlier 0.61, on a jittered
/// 100-subdivision cube top, matches the coarse reading. The Check is read at 20 edges to the
/// radius, research 21's measuring grid (decision 0062; catalog `brush.smooth-strong`).
#[test]
fn smooth_strong_leaves_under_half_of_smooths_noise_on_a_fine_mesh_and_over_half_on_a_coarse_one() {
    let ratio = |n: u32, z: Height| left(n, z, "smooth-strong") / left(n, z, "smooth");
    let fine = ratio(N, &jitter(0.5 * edge(N)));
    let ripple = ratio(N, &ridges(N, 11.0, 0.5 * edge(N)));
    let coarse = ratio(40, &jitter(0.5 * edge(40)));
    eprintln!(
        "Smooth Strong over Smooth: white noise {fine} at 20 edges to the radius, {coarse} at 5; \
         the ripple {ripple}"
    );
    assert!(fine < 0.5 && ripple < 0.5, "fine: {fine} {ripple}");
    assert!(coarse > 0.5, "coarse: {coarse}");
}

/// Every preset whose smooth kernel runs merges at 0.45 under Live Detail, the most merge
/// allows, so a smoothing stroke thins the mesh it evens (decision 0021, section 14: "Smooth
/// presets should carry 0.45"; Smooth Strong's call taken in decision 0062). Paint Blend's
/// kernel never runs, so it changes no shape to merge.
#[test]
fn every_smooth_kernel_preset_merges_at_the_most_live_detail_allows() {
    let merges: Vec<(String, Option<f32>)> = (shipped().into_iter())
        .filter(|s| matches!(s.preset.kernel, KernelSpec::Smooth(_)) && s.preset.mode != Mode::Off)
        .map(|s| (s.stem, s.preset.live_detail.merge))
        .collect();
    assert!(merges.len() >= 2, "Smooth and Smooth Strong: {merges:?}");
    for (stem, merge) in merges {
        assert_eq!(merge, Some(0.45), "{stem}");
    }
}

/// R12: Move Elastic's Check asks that pulling the side of a 2,000-face sphere by 2 brush
/// radii leave the edge lengths among the moved points at least a third less spread than Move
/// does (a proposed threshold). On a 1,944-face sphere the spread is 0.405 against Move's 0.412,
/// 1.6 percent lower: the clause fails. One relax pass inside the brush cannot shorten a pull
/// stretched evenly along its length (decision 0018). The Check changes, not the effect
/// (decision 0062; catalog `brush.move-elastic`).
#[test]
fn move_elastic_spreads_the_edges_about_as_much_as_move_on_a_2000_face_sphere() {
    let mut ball = fixtures::cube(18);
    for p in &mut ball.positions {
        *p = Vec3::from_array(*p).normalized().to_array();
    }
    let ball = store(&ball).to_plain();
    assert_eq!(ball.faces.len(), 1944, "about 2,000 faces");
    let side = Vec3::new(1.0, 0.2, 0.1);
    let spread =
        |stem: &str| edge_spread(&ball, &pull(&ball, &preset(stem).preset, side, 0.3, 0.6));
    let (elastic, plain) = (spread("move-elastic"), spread("move"));
    let lower = 1.0 - elastic / plain;
    eprintln!("edge spread: Move Elastic {elastic}, Move {plain}, {lower} lower");
    assert!(lower > 0.0 && lower < 1.0 / 3.0, "{lower} lower");
}

/// R13: Smooth Polish's Check asks for a flatter surface than Smooth leaves and edges 15 to
/// 40 percent of the radius wide (proposed), wider than Flat Polish's. Against today's Smooth
/// (strength 1, gain 2, spacing 0.1) one stroke of Smooth Polish leaves 0.82 of white noise half
/// an edge high where Smooth leaves 0.30, and 0.82 of bumps 0.8 edges high 8 edges apart where
/// Smooth leaves 0.75: rougher on both (on research 21's ripple about even, 0.83 against 0.86).
/// Across the stroke its cut's edge (as `brush_checks` measures Flat Polish's) is 0.56 radii,
/// wider than Flat Polish's 0.39 but past the proposed 0.4. The brush stays as built and the
/// target moves to it (decision 0062; catalog `brush.smooth-polish`).
#[test]
fn smooth_polish_is_rougher_than_todays_smooth_and_its_edge_wider_than_four_tenths() {
    let noise = jitter(0.5 * edge(N));
    let bumps = ridges(N, 8.0, 0.01);
    let (smooth, polish) = (left(N, &noise, "smooth"), left(N, &noise, "smooth-polish"));
    let (smooth_b, polish_b) = (left(N, &bumps, "smooth"), left(N, &bumps, "smooth-polish"));
    let mesh = relief(N, &bumps);
    let width = |stem: &str| {
        let a = after(
            &mesh,
            &preset(stem).preset,
            params(None),
            &along(&bumps, -0.6, 0.6, R),
        );
        edge_width(&mesh, &a, ridge_tops(N, 8.0), R)
    };
    let (soft, crisp) = (width("smooth-polish"), width("flat-polish"));
    eprintln!(
        "left of the noise: Smooth Polish {polish}, Smooth {smooth}; of the bumps: {polish_b} and \
         {smooth_b}; edge: Smooth Polish {soft} radii, Flat Polish {crisp}"
    );
    assert!(
        polish > smooth && polish_b > smooth_b,
        "rougher than Smooth"
    );
    assert!(soft > crisp && soft > 0.4, "wider: {soft} against {crisp}");
}

/// For a stroke along y = 0 over the flat `n` grid `flat` that left `line`: each grid column's
/// rise on the line, and how far either side the rise stays at or above half of `peak`.
fn columns(flat: &PlainMesh, line: &PlainMesh, peak: f32) -> BTreeMap<i32, (f32, f32)> {
    let mut cols: BTreeMap<i32, (f32, f32)> = BTreeMap::new();
    for (p, q) in flat.positions.iter().zip(&line.positions) {
        let (k, rise) = ((p[0] / edge(N)).round() as i32, q[2] - p[2]);
        let col = cols.entry(k).or_default();
        if p[1] == 0.0 {
            col.0 = rise;
        }
        if rise >= 0.5 * peak {
            col.1 = col.1.max(p[1].abs());
        }
    }
    cols
}

/// Clay Line's Check: the stroke lays a line of constant width and height along its path, and
/// its end caps are flat-topped. Width and height hold: over the middle (a radius and a half in
/// from each end) the width varies by no grid row and the height by 3.8 percent, a ripple
/// Buildup leaves as each dab re-fits its plane on the last one's ridge (1 percent with Buildup
/// off; the Check sets no tolerance, read here as 5 percent). The caps taper: each end falls
/// from 90 to 10 percent of the height over 1.00 and 1.08 radii past the end point, the
/// falloff's taper. The tapered ends stay until the trial of the original (research 29) shows
/// its ends, which are undocumented (decision 0062; catalog `brush.clay-line`).
#[test]
fn clay_line_lays_an_even_line_whose_ends_taper_over_a_radius() {
    let r = 0.15;
    let flat = relief(N, &|_, _| 0.0);
    let ends = [
        at(Vec3::new(-0.5, 0.0, 0.0), r),
        at(Vec3::new(0.5, 0.0, 0.0), r),
    ];
    let line = after(&flat, &preset("clay-line").preset, params(None), &ends);
    let mid = columns(&flat, &line, 0.0)[&0].0;
    let cols = columns(&flat, &line, mid);
    let middle: Vec<(f32, f32)> = (cols.iter())
        .filter(|(k, _)| (**k as f32 * edge(N)).abs() <= 0.5 - 1.5 * r)
        .map(|(_, c)| *c)
        .collect();
    let span = |f: fn(&(f32, f32)) -> f32| {
        let v = middle.iter().map(f);
        v.clone().fold(f32::MIN, f32::max) - v.fold(f32::MAX, f32::min)
    };
    let (height, width) = (span(|c| c.0) / mid, span(|c| c.1));
    // Past each end, where the height on the line falls to 90 and to 10 percent of the middle's.
    let taper = |sign: f32| -> f32 {
        let fall = |f: f32| {
            let out = (cols.iter()).filter(|(k, c)| **k as f32 * sign > 0.0 && c.0 <= f * mid);
            out.map(|(k, _)| (*k as f32 * edge(N)).abs())
                .fold(f32::MAX, f32::min)
        };
        (fall(0.1) - fall(0.9)) / r
    };
    let (start, end) = (taper(-1.0), taper(1.0));
    eprintln!(
        "height {mid}, varying {height} of it and the width by {width} over the middle; the \
         ends fall from 90 to 10 percent over {start} and {end} radii"
    );
    assert!(mid > 0.0 && height < 0.05, "an even height: {height}");
    assert!(width == 0.0, "an even width: {width}");
    assert!(start > 0.5 && end > 0.5, "tapered ends: {start} {end}");
}

/// A test brush on the Plane kernel at gain 4, both sides, with a height cap of 0.1 radii and its
/// plane `imbed` radii above the surface.
fn capped(imbed: f32) -> BrushPreset {
    ok(BrushPreset::from_toml(&format!(
        "name = \"Plane\"\nid = \"t\"\nfamily = \"Build\"\nstrength = 0.5\n\
         [kernel]\ntype = \"plane\"\ngain = 4.0\nheight = 0.1\n[depth]\nimbed = {imbed}\n"
    )))
}

/// The Plane kernel's Check: a full-strength dab leaves the points on the fitted plane within
/// tolerance, and the height cap stops points at the set height. Both stay as built. One
/// full-strength tap moves the point under its center 1 - exp(-gain x 0.5) of the way, so it
/// never lands: it leaves Smooth Polish 0.37 of the gap, Clay Build 0.14, Flat Polish and Trim
/// 0.05. A point farther above the plane than the cap is left where it is, not stopped at the
/// cap's height: with the cap at 0.1 radii and the plane 0.2 below, the center does not move,
/// and 0.05 below it moves 0.0126 (0.63 of the way). A full dab need not land, and a cap selects
/// the points that move rather than clamping them (decision 0062; catalog `core.kernel-plane`).
#[test]
fn a_full_strength_plane_dab_never_lands_and_a_point_past_the_cap_stays_put() {
    let flat = relief(80, &|_, _| 0.0);
    let tap = [at(Vec3::ZERO, 0.4)];
    let center = |m: &PlainMesh| {
        let i = (flat.positions.iter()).position(|p| p[0] == 0.0 && p[1] == 0.0);
        m.positions[i.unwrap_or_else(|| panic!("a vertex at the center"))][2]
    };
    for s in shipped() {
        let (KernelSpec::Plane(k), StrokeSpec::Dab { .. }) = (&s.preset.kernel, &s.preset.stroke)
        else {
            continue;
        };
        let mut p = s.preset.clone();
        p.strength = 1.0;
        p.depth.imbed = if k.side == Side::Above { -0.1 } else { 0.1 };
        let gap = p.depth.imbed * 0.4;
        let left = 1.0 - center(&after(&flat, &p, params(None), &tap)) / gap;
        eprintln!("{}: a full-strength tap leaves {left} of the gap", s.stem);
        assert!(left > 0.01 && left < 0.5, "{}: {left}", s.stem);
    }
    let past = center(&after(&flat, &capped(-0.2), params(None), &tap));
    let within = center(&after(&flat, &capped(-0.05), params(None), &tap));
    eprintln!("past the cap the center moved {past}; within it {within}");
    assert_eq!(past, 0.0, "past the cap, left where it is");
    assert!(within < 0.0, "within the cap, pulled down: {within}");
}
