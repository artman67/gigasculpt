//! Claims each shipped preset declares in its own file (ARCHITECTURE 10.1, kind 1): one generic
//! check per claim, so a new brush adds a line of data and no test code. Every preset is also
//! held to exact undo and to bit-identical results at 1 and 24 threads.

use gs_core::brush::{BrushPreset, KernelSpec, Select, StrokeSpec};
use gs_core::{MeshStore, Stroke, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::compare::canonical;
use gs_testkit::measure::stats;

use crate::brush_kit::{
    Shipped, arc, at, colors, flat, floats, ico, masked, max_diff, max_value_diff, mirror_pairs,
    mirror_x, painted, params, polyline, preset, run, samples, scalars, shipped, store, to_segment,
};
use crate::common::ok;

/// Claims the harness knows. A preset naming anything else fails.
const KNOWN: [&str; 8] = [
    "local",
    "masked_unchanged",
    "mirror_symmetric",
    "spacing_invariant",
    "rate_invariant",
    "drag_idempotent",
    "volume_non_decreasing",
    "reference",
];

const R: f32 = 0.3;

/// The presets declaring `claim`. Fails if none does, so a renamed claim cannot pass unseen.
fn claiming(claim: &str) -> Vec<Shipped> {
    let found: Vec<Shipped> = (shipped().into_iter())
        .filter(|s| s.preset.claims.iter().any(|c| c == claim))
        .collect();
    assert!(!found.is_empty(), "no preset claims {claim}");
    found
}

/// Whether anything (positions or any attribute) differs.
fn changed(a: &PlainMesh, b: &PlainMesh) -> bool {
    canonical(a) != canonical(b)
}

/// How far from its stroke a vertex can be and still change: the dab radius, times the elastic
/// support, plus the Stroke Smoothing string.
fn support(p: &BrushPreset) -> f32 {
    let reach = match &p.kernel {
        KernelSpec::Move(m) if m.select == Select::Elastic => m.support,
        _ => 1.0,
    };
    let rope = p.stroke.smoothing().radius;
    R * p.pressure.size.eval(1.0) * (reach + rope) + 1e-4
}

#[test]
fn every_shipped_preset_parses_and_names_known_claims() {
    let all = shipped();
    assert_eq!(
        all.len(),
        24,
        "one preset per phase 1 brush, Paint and Paint Blend, and the two UV hint brushes"
    );
    for s in &all {
        assert_eq!(
            s.preset.id,
            format!("brush.{}", s.stem),
            "file name follows the id"
        );
        for c in &s.preset.claims {
            assert!(KNOWN.contains(&c.as_str()), "{}: unknown claim {c}", s.stem);
        }
    }
}

#[test]
fn local_presets_leave_everything_outside_their_support_bit_identical() {
    for s in claiming("local") {
        let p = &s.preset;
        let mut m = store(&ico(4));
        let before = m.to_plain();
        let smp = samples(p, R);
        run(&mut m, p, params(Some(mirror_x())), &smp);
        let after = m.to_plain();
        assert!(
            changed(&before, &after),
            "{}: the stroke changed something",
            s.stem
        );
        let flip = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
        let reach = support(p);
        let (mb, ma) = (scalars(&before), scalars(&after));
        let (cb, ca) = (colors(&before), colors(&after));
        for (i, (q, r)) in before.positions.iter().zip(&after.positions).enumerate() {
            let q3 = Vec3::from_array(*q);
            let near = (smp.windows(2).chain(std::iter::once(&smp[..1])))
                .flat_map(|w| {
                    let (a, b) = (w[0].pos, w[w.len() - 1].pos);
                    [to_segment(q3, a, b), to_segment(q3, flip(a), flip(b))]
                })
                .fold(f32::INFINITY, f32::min);
            if near > reach {
                assert_eq!(
                    q.map(f32::to_bits),
                    r.map(f32::to_bits),
                    "{}: vertex {i} moved",
                    s.stem
                );
                assert_eq!(
                    mb[i].map(f32::to_bits),
                    ma[i].map(f32::to_bits),
                    "{}: mask and UV hints of vertex {i}",
                    s.stem
                );
                assert_eq!(cb[i], ca[i], "{}: color of vertex {i}", s.stem);
            }
        }
    }
}

#[test]
fn masked_vertices_never_move() {
    for s in claiming("masked_unchanged") {
        let p = &s.preset;
        let mut m = store(&masked(&ico(4)));
        let before = m.to_plain();
        run(&mut m, p, params(Some(mirror_x())), &samples(p, R));
        let after = m.to_plain();
        let mask = floats(&after, "mask");
        let mut free_moved = false;
        for (i, (q, r)) in before.positions.iter().zip(&after.positions).enumerate() {
            let same = q.map(f32::to_bits) == r.map(f32::to_bits);
            if mask[i] >= 1.0 {
                assert!(same, "{}: masked vertex {i} moved", s.stem);
            }
            free_moved |= !same;
        }
        assert!(free_moved, "{}: unmasked vertices moved", s.stem);
    }
}

#[test]
fn symmetric_strokes_on_a_symmetric_mesh_stay_symmetric() {
    let mesh = ico(4);
    for s in claiming("mirror_symmetric") {
        let p = &s.preset;
        let mut m = store(&mesh);
        let start = m.to_plain();
        let pair = mirror_pairs(&start);
        run(&mut m, p, params(Some(mirror_x())), &samples(p, R));
        let after = m.to_plain();
        assert!(
            changed(&start, &after),
            "{}: the stroke changed something",
            s.stem
        );
        let values = scalars(&after);
        let mut worst = 0.0f32;
        for (i, &j) in pair.iter().enumerate() {
            let [x, y, z] = after.positions[i];
            let want = Vec3::new(-x, y, z);
            worst = worst.max((Vec3::from_array(after.positions[j]) - want).len2().sqrt());
            for (a, b) in values[i].iter().zip(&values[j]) {
                worst = worst.max((a - b).abs());
            }
        }
        // Most presets come out bit-exact; the worst measured was 1.2e-7 (Smooth).
        assert!(worst <= 2e-6, "{}: off by {worst}", s.stem);
    }
}

/// `p` with its spacing scaled by `k`.
fn spaced(p: &BrushPreset, k: f32) -> BrushPreset {
    let mut q = p.clone();
    match &mut q.stroke {
        StrokeSpec::Dab { spacing, .. } | StrokeSpec::Curve { spacing, .. } => *spacing *= k,
        _ => panic!("{} has no spacing", p.name),
    }
    q
}

/// What a stroke along `smp` does: the mesh after, and the largest change of position, mask or
/// UV hint.
fn effect(mesh: &PlainMesh, p: &BrushPreset, smp: &[gs_core::StrokeSample]) -> (PlainMesh, f32) {
    let mut m = store(mesh);
    let before = m.to_plain();
    run(&mut m, p, params(None), smp);
    let after = m.to_plain();
    let size = off(&before, &after);
    (after, size)
}

/// The largest difference of position, mask or UV hint between `a` and `b`.
fn off(a: &PlainMesh, b: &PlainMesh) -> f32 {
    max_diff(a, b).max(max_value_diff(&flat(a), &flat(b)))
}

#[test]
fn strength_does_not_depend_on_spacing() {
    let mesh = ico(4);
    let smp = arc(Vec3::new(0.3, 0.1, 1.0), Vec3::new(-0.3, 0.2, 1.0), 6, 0.25);
    // Every preset as shipped, and both Smooth brushes at full strength, where a dab's step is
    // largest (decision 0036 section 2).
    let full = ["smooth", "smooth-strong"].map(|stem| {
        let mut s = preset(stem);
        s.preset.strength = 1.0;
        s.stem = format!("{stem} at strength 1");
        s
    });
    for s in claiming("spacing_invariant").into_iter().chain(full) {
        let p = &s.preset;
        let (a, size) = effect(&mesh, p, &smp);
        let (b, _) = effect(&mesh, &spaced(p, 0.5), &smp);
        let off = off(&a, &b);
        assert!(size > 0.0, "{}: the stroke changed something", s.stem);
        // Measured worst at the shipped values: 2.3% (Pinch); Smooth Strong, which runs
        // sub-steps, 0.2%. Smooth at gain 2 takes one step a dab (research 21 measured 1.4%); at
        // decision 0036's gain of 32 its worst over strengths 0.01 to 1 was 4.4%, at 0.31, a dab
        // just under one sub-step. Strength per dab instead of per length would double the effect.
        assert!(
            off <= 0.05 * size,
            "{}: spacing changed the result by {off} of {size}",
            s.stem
        );
    }
}

#[test]
fn strength_does_not_depend_on_sample_rate() {
    let mesh = ico(4);
    // Pressure and radius change along the stroke, so the samples in between matter.
    let corners = [
        (Vec3::new(0.4, 0.1, 0.9), 0.4, 0.25),
        (Vec3::new(0.0, 0.3, 0.95), 1.0, 0.32),
        (Vec3::new(-0.3, 0.0, 0.95), 0.6, 0.28),
    ];
    for s in claiming("rate_invariant") {
        let p = &s.preset;
        let (a, size) = effect(&mesh, p, &polyline(&corners, 1));
        let (b, _) = effect(&mesh, p, &polyline(&corners, 7));
        let off = off(&a, &b);
        assert!(size > 0.0, "{}: the stroke changed something", s.stem);
        // Ten times the worst rounding difference measured (6e-5, Trim).
        assert!(
            off <= 6e-4 * size,
            "{}: sample rate changed the result by {off} of {size}",
            s.stem
        );
    }
}

#[test]
fn dragging_out_and_back_returns_the_start_exactly() {
    for s in claiming("drag_idempotent") {
        let p = &s.preset;
        let mut m = store(&ico(4));
        let start = m.to_plain();
        let g = Vec3::new(0.3, 0.2, 1.0).normalized();
        let d = Vec3::new(-0.2, 0.25, 0.1);
        let mut k = ok(Stroke::begin(&mut m, p, params(Some(mirror_x()))));
        ok(k.add(&mut m, &[at(g, R), at(g + d * 0.5, R), at(g + d, R)]));
        assert!(
            changed(&start, &m.to_plain()),
            "{}: dragging out moved something",
            s.stem
        );
        ok(k.add(&mut m, &[at(g + d * 0.5, R), at(g, R)]));
        let back = m.to_plain();
        assert!(
            !changed(&start, &back),
            "{}: back at the start, the mesh is the start",
            s.stem
        );
        let rep = k.finish(&mut m);
        assert_eq!(
            rep.step.map(|x| x.nodes),
            Some(0),
            "{}: no undo step for no change",
            s.stem
        );
    }
}

#[test]
fn inflating_never_shrinks_the_volume() {
    for s in claiming("volume_non_decreasing") {
        for strength in [0.02, 0.1, 0.3] {
            let p = &BrushPreset {
                strength,
                ..s.preset.clone()
            };
            let mut m = store(&ico(4));
            let v0 = stats(&m.to_plain()).volume;
            run(&mut m, p, params(Some(mirror_x())), &samples(p, R));
            let v1 = stats(&m.to_plain()).volume;
            assert!(
                v1 > v0,
                "{} at strength {strength}: volume {v0} became {v1}",
                s.stem
            );
        }
    }
}

/// The mesh each preset's undo and determinism checks stroke: masked and painted.
fn start() -> PlainMesh {
    painted(&masked(&ico(4)))
}

/// The stroke each preset's undo and determinism checks run, with symmetry.
fn standard(p: &BrushPreset) -> MeshStore {
    let mut m = store(&start());
    run(&mut m, p, params(Some(mirror_x())), &samples(p, R));
    m
}

#[test]
fn every_stroke_is_one_undo_step_and_undo_is_exact() {
    for s in shipped() {
        let p = &s.preset;
        let mut m = store(&start());
        let (before, undo_len) = (m.to_plain(), m.undo_len());
        run(&mut m, p, params(Some(mirror_x())), &samples(p, R));
        let after = m.to_plain();
        assert!(
            changed(&before, &after),
            "{}: the stroke changed something",
            s.stem
        );
        assert_eq!(m.undo_len(), undo_len + 1, "{}: one undo step", s.stem);
        ok(m.undo().ok_or("nothing to undo"));
        assert!(
            !changed(&before, &m.to_plain()),
            "{}: undo is exact",
            s.stem
        );
        ok(m.redo().ok_or("nothing to redo"));
        assert!(!changed(&after, &m.to_plain()), "{}: redo is exact", s.stem);
    }
}

#[test]
fn every_stroke_is_bit_identical_on_1_and_24_threads() {
    for s in shipped() {
        let p = &s.preset;
        let one = gs_par::with_threads(1, || canonical(&standard(p).to_plain()));
        let many = gs_par::with_threads(24, || canonical(&standard(p).to_plain()));
        assert!(one == many, "{}: 1 and 24 threads differ", s.stem);
    }
}

/// Smooth's sub-steps run as the rounds of one step unless its deltas read normals, and then as
/// whole runs (decision 0036, section 2); no shipped preset takes the whole runs, so nothing
/// else holds them. A backface automask that spares nothing (the stroke stays on the side facing
/// the view) sends Uniform and Stronger down the whole runs with the same numbers: both paths
/// give the same bits. Tangent and the cavity automask read normals, so they take the whole
/// runs: the same bits on 1 and 24 threads. Smooth ships at gain 2, where a dab is one step, so
/// the test raises the gain to 32 to make every dab sub-step.
#[test]
fn smooth_gives_the_same_bits_by_rounds_and_by_whole_runs() {
    let text = shipped()
        .into_iter()
        .find(|s| s.stem == "smooth")
        .map(|s| s.text)
        .unwrap_or_default();
    assert!(text.contains("gain = 2.0"), "Smooth's gain moved");
    let text = text.replace("gain = 2.0", "gain = 32.0");
    let smooth = |mode: &str, extra: &str| {
        let t = text.replace("mode = \"uniform\"", &format!("mode = \"{mode}\""));
        ok(BrushPreset::from_toml(&format!("{t}\n{extra}\n")))
    };
    let spare_nothing = "[automask]\nbackface = 1.0";
    for mode in ["uniform", "stronger"] {
        let rounds = canonical(&standard(&smooth(mode, "")).to_plain());
        let runs = canonical(&standard(&smooth(mode, spare_nothing)).to_plain());
        let before = canonical(&store(&start()).to_plain());
        assert!(rounds != before, "{mode}: the stroke moved something");
        assert!(rounds == runs, "{mode}: rounds and whole runs differ");
    }
    for p in [
        smooth("tangent", ""),
        smooth("uniform", "[automask]\ncavity = 0.5"),
    ] {
        let one = gs_par::with_threads(1, || canonical(&standard(&p).to_plain()));
        let many = gs_par::with_threads(24, || canonical(&standard(&p).to_plain()));
        assert!(one == many, "{:?}: 1 and 24 threads differ", p.kernel);
    }
}
