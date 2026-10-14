//! Behaviors research 10 settled for shipped presets (decision 0018): Flat Polish cuts what
//! sticks up and leaves hollows, keeping its first tilt while its height follows the form; Move
//! Elastic keeps the polygons around a pull more even than Move, and above its face limit it is
//! exactly Move. Smooth evens a noisy plane without shrinking its edge, and Alt roughens it.

use std::collections::BTreeSet;

use gs_core::brush::{KernelSpec, PlaneSource};
use gs_core::{BrushPreset, Vec3};
use gs_protocol::PlainMesh;

use crate::brush_kit::{after, at, grid, ico, params, preset, run, store};
use crate::common::surface_point;

/// The flat grid with its heights set by `z(x, y)`, in the store's vertex order.
fn shaped(z: impl Fn(f32, f32) -> f32) -> PlainMesh {
    let mut m = store(&grid(40)).to_plain();
    for p in &mut m.positions {
        p[2] = z(p[0], p[1]);
    }
    store(&m).to_plain()
}

/// Samples along the surface `z` from `x0` to `x1` at `y = 0`.
fn across(z: impl Fn(f32, f32) -> f32, x0: f32, x1: f32) -> Vec<gs_core::StrokeSample> {
    crate::brush_checks::along(&z, x0, x1, 0.25)
}

/// How far each vertex moved up (negative: down).
fn rises(before: &PlainMesh, after: &PlainMesh) -> Vec<f32> {
    (before.positions.iter().zip(&after.positions))
        .map(|(a, b)| b[2] - a[2])
        .collect()
}

#[test]
fn flat_polish_cuts_what_sticks_up_and_leaves_hollows() {
    // A dent at x = -0.3 and a bump at x = 0.3 on a flat grid, polished across both.
    let bumpy = |x: f32, y: f32| {
        let d = |cx: f32| ((x - cx) * (x - cx) + y * y).sqrt();
        -0.05 * (1.0 - d(-0.3) / 0.12).max(0.0) + 0.05 * (1.0 - d(0.3) / 0.12).max(0.0)
    };
    let mesh = shaped(bumpy);
    let polish = preset("flat-polish").preset;
    let path = across(bumpy, -0.6, 0.6);
    let cut = rises(&mesh, &after(&mesh, &polish, params(None), &path));
    let mut alt = params(None);
    alt.invert = true;
    let fill = rises(&mesh, &after(&mesh, &polish, alt, &path));
    let (mut bump, mut dent) = ([0.0f32; 2], [0.0f32; 2]);
    for (i, p) in mesh.positions.iter().enumerate() {
        let (x, y) = (p[0], p[1]);
        if (x - 0.3) * (x - 0.3) + y * y < 0.05 * 0.05 {
            bump = [bump[0].min(cut[i]), bump[1].max(fill[i])];
        }
        if (x + 0.3) * (x + 0.3) + y * y < 0.05 * 0.05 {
            dent = [dent[0].max(cut[i]), dent[1].max(fill[i])];
        }
    }
    assert!(bump[0] < -0.01, "the bump is cut down: {}", bump[0]);
    assert!(dent[0] <= 0.0, "the dent is left alone: {}", dent[0]);
    assert!(dent[1] > 0.01, "Alt fills the dent: {}", dent[1]);
    assert!(bump[1] <= 0.0, "and leaves the bump: {}", bump[1]);
}

#[test]
fn flat_polish_keeps_its_tilt_but_follows_the_height_of_the_form() {
    // Two flat levels joined by a ramp: z = 0 for x below -0.1, 0.12 above 0.1.
    let steps = |x: f32, _: f32| 0.12 * ((x + 0.1) / 0.2).clamp(0.0, 1.0);
    let mesh = shaped(steps);
    let path = across(steps, -0.6, 0.6);
    let polish = preset("flat-polish").preset;
    let with_source = |source: PlaneSource| {
        let mut p: BrushPreset = polish.clone();
        if let KernelSpec::Plane(k) = &mut p.kernel {
            k.source = source;
        }
        rises(&mesh, &after(&mesh, &p, params(None), &path))
    };
    let (shipped, kept) = (
        with_source(PlaneSource::StrokeNormal),
        with_source(PlaneSource::Stroke),
    );
    let upper = |r: &[f32]| {
        (mesh.positions.iter().zip(r))
            .filter(|(p, _)| p[0] >= 0.45 && p[1].abs() < 0.1)
            .map(|(_, d)| d.abs())
            .fold(0.0f32, f32::max)
    };
    // Dabs near the ramp still read a little of it, so the upper level moves a little.
    assert!(
        upper(&kept) > 0.02 && upper(&shipped) < 0.3 * upper(&kept),
        "the plane rises with the upper level ({}) where one kept whole cuts it down ({})",
        upper(&shipped),
        upper(&kept)
    );
}

/// The coefficient of variation of the lengths of the edges whose ends both moved.
pub fn edge_spread(before: &PlainMesh, after: &PlainMesh) -> f64 {
    let moved =
        |v: usize| before.positions[v].map(f32::to_bits) != after.positions[v].map(f32::to_bits);
    let mut edges = BTreeSet::new();
    for f in &after.faces {
        let c: Vec<usize> = (f.iter())
            .filter(|&&v| v != gs_core::NO)
            .map(|&v| v as usize)
            .collect();
        for k in 0..c.len() {
            let (a, b) = (c[k], c[(k + 1) % c.len()]);
            if moved(a) && moved(b) {
                edges.insert((a.min(b), a.max(b)));
            }
        }
    }
    let len = |(a, b): (usize, usize)| {
        let d = Vec3::from_array(after.positions[a]) - Vec3::from_array(after.positions[b]);
        f64::from(d.len2().sqrt())
    };
    let lengths: Vec<f64> = edges.into_iter().map(len).collect();
    let n = lengths.len() as f64;
    let mean = lengths.iter().sum::<f64>() / n;
    let var = lengths.iter().map(|l| (l - mean) * (l - mean)).sum::<f64>() / n;
    var.sqrt() / mean
}

/// A pull of `p` on `mesh`: grabbed at the surface toward `toward`, dragged out along it by
/// `distance`.
pub fn pull(mesh: &PlainMesh, p: &BrushPreset, toward: Vec3, r: f32, distance: f32) -> PlainMesh {
    let g = surface_point(&store(mesh), toward);
    let path: Vec<_> = (0..=8)
        .map(|i| at(g + toward.normalized() * (distance * i as f32 / 8.0), r))
        .collect();
    after(mesh, p, params(None), &path)
}

#[test]
fn move_elastic_evens_the_polygons_under_its_limit_and_is_move_above_it() {
    // Pulling the side of a sphere under the face limit by 2 brush radii: the relax pass leaves
    // the edge lengths less spread than Move does. (The catalog's proposed "a third less" is not
    // met: one relax pass inside the brush measured 0.413 against Move's 0.418, and 0.393 even
    // at sixteen times the gain, because relaxing along the surface cannot shorten a pull that
    // is stretched evenly along its length. Decision 0018 leaves that model open.)
    let (elastic, plain) = (preset("move-elastic").preset, preset("move").preset);
    let side = Vec3::new(1.0, 0.2, 0.1);
    let mesh = store(&ico(4)).to_plain();
    assert!(mesh.faces.len() < 25_000, "under the limit");
    let (e, m) = (
        pull(&mesh, &elastic, side, 0.3, 0.6),
        pull(&mesh, &plain, side, 0.3, 0.6),
    );
    let (se, sm) = (edge_spread(&mesh, &e), edge_spread(&mesh, &m));
    assert!(se < sm, "edge spread {se} against Move's {sm}");
    // Above 25,000 faces the evening step is skipped: exactly Move.
    let dense = store(&ico(6)).to_plain();
    assert!(dense.faces.len() > 25_000, "over the limit");
    let bits = |m: &PlainMesh| -> Vec<[u32; 3]> {
        m.positions.iter().map(|p| p.map(f32::to_bits)).collect()
    };
    assert_eq!(
        bits(&pull(&dense, &elastic, side, 0.2, 0.3)),
        bits(&pull(&dense, &plain, side, 0.2, 0.3)),
        "Move, bit for bit"
    );
}

/// The heights' root mean square about their mean, over the points within half the radius
/// (0.25) of a stroke along y = 0 and inside its ends.
fn roughness(m: &PlainMesh) -> f64 {
    let z: Vec<f64> = (m.positions.iter())
        .filter(|p| p[0].abs() < 0.4 && p[1].abs() < 0.12)
        .map(|p| f64::from(p[2]))
        .collect();
    let mean = z.iter().sum::<f64>() / z.len() as f64;
    (z.iter().map(|h| (h - mean) * (h - mean)).sum::<f64>() / z.len() as f64).sqrt()
}

/// Smooth's Check (catalog `brush.smooth`; its color clause is `paint_channels`): each pass
/// over a noisy plane takes its height RMS down and keeps every point and face, a pass along
/// the open edge keeps the edge where it was, and an Alt stroke on the smoothed plane roughens
/// it again.
#[test]
fn smooth_evens_a_noisy_plane_pass_by_pass_and_alt_roughens_it() {
    let noise = |x: f32, y: f32| {
        let k = (u64::from(x.to_bits()) << 32) | u64::from(y.to_bits());
        ((crate::common::mix(k) % 2001) as f32 / 1000.0 - 1.0) * 0.01
    };
    let smooth = preset("smooth").preset;
    let path = across(noise, -0.6, 0.6);
    let mut s = store(&shaped(noise));
    let mut m = s.to_plain();
    let (points, faces) = (m.positions.len(), m.faces.clone());
    let mut rms = vec![roughness(&m)];
    for _ in 0..3 {
        run(&mut s, &smooth, params(None), &path);
        m = s.to_plain();
        rms.push(roughness(&m));
        assert_eq!((m.positions.len(), &m.faces), (points, &faces), "same mesh");
    }
    assert!(
        rms.windows(2).all(|w| w[1] < w[0]),
        "each pass smooths: {rms:?}"
    );
    let edge: Vec<_> = (0..=24)
        .map(|i| at(Vec3::new(-0.6 + 0.05 * i as f32, -1.0, 0.0), 0.25))
        .collect();
    run(&mut s, &smooth, params(None), &edge);
    let along = s.to_plain();
    let mut on_edge = 0;
    for (p, q) in m.positions.iter().zip(&along.positions) {
        if p[1].to_bits() == (-1.0f32).to_bits() {
            assert_eq!(
                q[1].to_bits(),
                p[1].to_bits(),
                "{p:?} left the edge for {q:?}"
            );
            on_edge += 1;
        }
    }
    assert_eq!(on_edge, 41, "the plane's edge");
    let mut alt = params(None);
    alt.invert = true;
    run(&mut s, &smooth, alt, &path);
    let (smoothed, rough) = (roughness(&along), roughness(&s.to_plain()));
    eprintln!("height RMS by pass {rms:?}; Alt took {smoothed} to {rough}");
    assert!(rough > smoothed, "Alt roughens: {smoothed} became {rough}");
}
