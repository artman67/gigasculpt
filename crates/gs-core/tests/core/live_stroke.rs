//! Refining strokes through the stroke pipeline (decision 0021, L1): random strokes of the
//! shipped presets keep the mesh valid, with every derived list and normal exact after every
//! dab and no dead slot or turned face after the stroke; undo, redo and cancel are exact; the
//! same bits on 1 and 24 threads; the budget bounds every dab; history holds only nodes whose
//! columns changed; points at mask 1 keep their bits; strokes that never refine keep their
//! topology; every dab and every pass marks each node whose drawing changed.

use gs_core::live::{LiveDetail, LiveReport, Target};
use gs_core::symmetry::Symmetry;
use gs_core::{MeshStore, PosRead, Stroke, StrokeParams, StrokeReport, StrokeSample, Vec3};
use gs_protocol::AttrValues;

use crate::brush_kit::{at, grid, ico, params, preset, run_preset, shipped_alpha};
use crate::common::{
    check, check_closed, dir, fingerprint, mix, ok, small_nodes, snap, some, sphere, store,
    surface_point, turned_and_flat, unit,
};
use crate::live::changed_nodes;

fn refining(ld: LiveDetail) -> StrokeParams {
    let mut p = params(None);
    p.live_detail = Some(ld);
    p
}

fn constant(len: f32) -> LiveDetail {
    LiveDetail {
        target: Target::Constant(len),
        ..LiveDetail::default()
    }
}

/// A short path over the front (+z) of the test sphere, from direction `i`.
fn path(s: &MeshStore, i: u64, r: f32) -> Vec<StrokeSample> {
    let front = |d: Vec3| Vec3::new(d.x, d.y, d.z.abs() + 0.5).normalized();
    let (a, b) = (front(dir(i)), front(dir(i + 7)));
    let b = a + (b - a) * 0.5;
    (0..=8)
        .map(|k| at(surface_point(s, a + (b - a) * (k as f32 / 8.0)), r))
        .collect()
}

/// No dead point or face slot anywhere.
fn dense(s: &MeshStore) -> Result<(), String> {
    for (k, n) in snap(s).nodes.iter().enumerate() {
        if n.pos.iter().any(|p| p.is_dead()) || n.faces.iter().any(|f| f[0] == gs_core::NO) {
            return Err(format!("node {k} keeps a dead slot"));
        }
    }
    Ok(())
}

/// One stroke of shipped preset `stem`, fed a sample at a time, checking every derived list and
/// normal after each.
fn stroke_checked(
    s: &mut MeshStore,
    stem: &str,
    prm: StrokeParams,
    samples: &[StrokeSample],
) -> StrokeReport {
    let p = preset(stem).preset;
    let mut prm = prm;
    prm.alpha = shipped_alpha(&p);
    let mut st = ok(Stroke::begin(s, &p, prm));
    for (i, x) in samples.iter().enumerate() {
        ok(st.add(s, std::slice::from_ref(x)));
        if let Err(e) = check(&snap(s)) {
            panic!("{stem}, sample {i}: {e}");
        }
    }
    st.finish(s)
}

const REFINING: [&str; 9] = [
    "draw",
    "clay",
    "clay-build",
    "inflate",
    "pinch",
    "smooth",
    "flat-polish",
    "trim",
    "sharp-crease",
];

#[test]
fn random_refining_strokes_keep_the_mesh_valid() {
    let mut s = store(&sphere(8, 0.7, 21, true), small_nodes());
    let mut ops = 0;
    for (i, stem) in REFINING.iter().enumerate() {
        let i = i as u64;
        let ld = LiveDetail {
            merge: 0.2 + 0.125 * (unit(mix(i)) + 1.0),
            budget: 64 + (mix(i * 5) % 600) as u32,
            ..constant([0.04, 0.08, 0.2][i as usize % 3])
        };
        let samples = path(&s, 300 + i, 0.25);
        let out = stroke_checked(&mut s, stem, refining(ld), &samples);
        let live = some(out.live);
        assert!(live.passes > 0, "{stem}: {live:?}");
        ops += live.ops();
        assert!(some(out.step).topology == live.changed(), "{stem}");
        if let Err(e) = check(&snap(&s)).and(check_closed(&s)).and(dense(&s)) {
            panic!("{stem}: {e}");
        }
        let (turned, _) = turned_and_flat(&snap(&s));
        assert_eq!(turned, 0, "{stem}: faces turned over");
    }
    assert!(ops > 200, "the strokes changed the topology: {ops}");
}

#[test]
fn undo_redo_and_cancel_are_exact() {
    let mut s = store(&sphere(8, 1.0, 22, true), small_nodes());
    let samples = path(&s, 400, 0.3);
    let start = fingerprint(&snap(&s));
    let out = run_preset(&mut s, "clay-build", refining(constant(0.05)), &samples);
    assert!(some(out.live).ops() > 50);
    let end = fingerprint(&snap(&s));
    some(s.undo());
    ok(check(&snap(&s)));
    assert_eq!(
        fingerprint(&snap(&s)),
        start,
        "undo returns the start exactly"
    );
    some(s.redo());
    ok(check(&snap(&s)));
    assert_eq!(
        fingerprint(&snap(&s)),
        end,
        "redo returns the stroke exactly"
    );
    let p = preset("draw").preset;
    let mut st = ok(Stroke::begin(&mut s, &p, refining(constant(0.04))));
    let more = path(&s, 410, 0.3);
    ok(st.add(&mut s, &more));
    assert!(some(st.report().live).ops() > 0);
    st.cancel(&mut s);
    ok(check(&snap(&s)));
    assert_eq!(
        fingerprint(&snap(&s)),
        end,
        "cancel returns the pre-stroke mesh"
    );
}

/// A re-sort outside an open step numbers the nodes afresh, so it drops the undo and redo steps
/// recorded against the old numbering: nothing is left to undo or redo, and the mesh stays as
/// sorted.
#[test]
fn a_resort_outside_a_step_drops_the_history() {
    let mut s = store(&sphere(8, 1.0, 24, false), small_nodes());
    for i in 0..3 {
        let samples = path(&s, 420 + i, 0.3);
        run_preset(&mut s, "clay", refining(constant(0.03)), &samples);
    }
    some(s.undo());
    assert_eq!((s.undo_len(), s.redo_len()), (2, 1));
    let before = fingerprint(&snap(&s));
    ok(s.resort());
    let sorted = fingerprint(&snap(&s));
    assert_ne!(sorted, before, "the re-sort numbers the nodes afresh");
    assert!(s.undo().is_none(), "nothing to undo");
    assert!(s.redo().is_none(), "nothing to redo");
    ok(check(&snap(&s)).and(check_closed(&s)));
    assert_eq!(fingerprint(&snap(&s)), sorted);
}

fn script() -> Vec<u64> {
    let mut s = store(&sphere(8, 0.7, 23, true), small_nodes());
    let mut prints = Vec::new();
    for (i, stem) in ["draw", "clay", "smooth", "pinch"].iter().enumerate() {
        let samples = path(&s, 500 + i as u64, 0.3);
        run_preset(&mut s, stem, refining(constant(0.05)), &samples);
        prints.push(fingerprint(&snap(&s)));
    }
    prints
}

#[test]
fn refining_strokes_are_bit_identical_on_1_and_24_threads() {
    let runs = [1, 24].map(|t| gs_par::with_threads(t, script));
    assert_eq!(runs[0], runs[1]);
}

#[test]
fn the_budget_bounds_every_dab() {
    let mut s = store(&sphere(8, 1.0, 24, false), small_nodes());
    let ld = LiveDetail {
        budget: 12,
        ..constant(0.02)
    };
    let samples = path(&s, 600, 0.3);
    let live = some(run_preset(&mut s, "draw", refining(ld), &samples).live);
    assert!(live.most_ops <= 12, "{live:?}");
    assert!(
        live.budget_hits > 0,
        "a fine target runs out of budget: {live:?}"
    );
}

/// Samples along the line from `a` to `b`, each projected onto the surface.
fn sweep(s: &MeshStore, a: Vec3, b: Vec3, r: f32, n: usize) -> Vec<StrokeSample> {
    (0..=n)
        .map(|k| at(surface_point(s, a + (b - a) * (k as f32 / n as f32)), r))
        .collect()
}

/// Mirrored refining strokes over triangles straddling the plane end, keeping the map: the
/// split step once sent such a pair and its shorter crossing side back to the queue in turn
/// forever (review 0006, C1). The last case is the one that looped.
#[test]
fn mirrored_strokes_over_the_plane_end() {
    let mut s = store(&ico(4), small_nodes());
    ok(s.map_mirror(0, 0.0));
    let mirror = Symmetry {
        mirror: [true, false, false],
        ..Symmetry::default()
    };
    let cases = [
        (
            Vec3::new(-0.4, 0.3, 1.0),
            Vec3::new(0.4, 0.1, 1.0),
            0.25,
            0.04,
        ),
        (
            Vec3::new(0.0, -0.6, 1.0),
            Vec3::new(0.0, 0.6, 1.0),
            0.2,
            0.03,
        ),
        (
            Vec3::new(0.3, -0.6, 1.0),
            Vec3::new(0.35, 0.6, 1.0),
            0.3,
            0.05,
        ),
        (
            Vec3::new(0.05, 0.0, 1.0),
            Vec3::new(0.05, 0.4, 0.8),
            0.1,
            0.012,
        ),
        (
            Vec3::new(-0.6, 0.3, 1.0),
            Vec3::new(0.6, 0.1, 1.0),
            0.35,
            0.2,
        ),
        (
            Vec3::new(0.0, 0.9, 0.3),
            Vec3::new(0.0, 0.9, -0.3),
            0.3,
            0.03,
        ),
    ];
    for (a, b, r, len) in cases {
        for stem in ["draw", "clay", "pinch"] {
            let mut p = params(Some(mirror));
            p.live_detail = Some(constant(len));
            let samples = sweep(&s, a, b, r, 16);
            run_preset(&mut s, stem, p, &samples);
            ok(check(&snap(&s)).and(check_closed(&s)));
            ok(s.check_mirror(0));
        }
    }
}

/// A sculpting dab gives the same result from either side of its mirror planes: the same
/// refining stroke started on the other side of x = 0 (one plane), or of x = 0 and y = 0 near
/// where they meet (four passes overlapping), leaves the part the same bit for bit. A dab's
/// frame was sampled where the pen is, so the two runs summed the same points in other orders,
/// and the passes' deltas were summed pen first; both differed in the last bits, and a
/// refinement decision that close to its threshold went either way (decision 0021, L4). The
/// shipped presets without an alpha: an alpha is stamped as drawn under the pen and mirrored
/// on the copies, so with one the side is meant to show.
#[test]
fn a_dab_gives_the_same_result_from_either_side() {
    let base = {
        let mut s = store(&ico(4), small_nodes());
        ok(s.map_mirror(0, 0.0));
        s
    };
    let x_and_y = Symmetry {
        mirror: [true, true, false],
        ..Symmetry::default()
    };
    let cases = [
        (
            crate::brush_kit::mirror_x(),
            [-1.0, 1.0],
            (0.35, 0.2),
            (0.05, 0.5),
        ),
        (x_and_y, [-1.0, -1.0], (0.1, 0.05), (0.15, 0.1)),
        (x_and_y, [-1.0, 1.0], (0.1, 0.05), (0.15, 0.1)),
    ];
    for (sym, [fx, fy], a, b) in cases {
        let (a, b) = (Vec3::new(a.0, a.1, 1.0), Vec3::new(b.0, b.1, 1.0));
        let samples = sweep(&base, a, b, 0.25, 12);
        let other: Vec<StrokeSample> = (samples.iter())
            .map(|x| StrokeSample {
                pos: Vec3::new(fx * x.pos.x, fy * x.pos.y, x.pos.z),
                ..*x
            })
            .collect();
        for stem in ["draw", "inflate", "pinch", "smooth", "flat-polish", "trim"] {
            let run = |samples: &[StrokeSample]| {
                let mut s = base.share();
                let mut p = params(Some(sym));
                p.live_detail = Some(constant(0.04));
                let live = some(run_preset(&mut s, stem, p, samples).live);
                (fingerprint(&snap(&s)), live.splits)
            };
            let ((one, splits), (two, _)) = (run(&samples), run(&other));
            assert!(splits > 100, "{stem}: {splits} splits");
            assert_eq!(one, two, "{stem}, mirrored by {fx}, {fy}: the other side");
        }
    }
}

#[test]
fn history_holds_only_nodes_whose_columns_changed() {
    let mut s = store(&sphere(10, 1.0, 25, true), small_nodes());
    for (i, stem) in ["draw", "clay", "smooth"].iter().enumerate() {
        let before = snap(&s);
        let copies = s.work().copies;
        let samples = path(&s, 700 + i as u64, 0.25);
        let out = run_preset(&mut s, stem, refining(constant(0.05)), &samples);
        let changed = changed_nodes(&before, &snap(&s));
        assert!(changed > 0);
        assert_eq!(some(out.step).nodes, changed, "{stem}");
        // Each column of a changed node is copied at most once in the step: positions,
        // normals, topology and the attribute columns.
        let columns = 3 + s.attrs().len() as u64;
        let copied = s.work().copies - copies;
        assert!(
            copied > 0,
            "{stem}: the step held the columns the pass wrote"
        );
        assert!(
            copied <= changed as u64 * columns,
            "{stem}: {copied} copies, {changed} nodes"
        );
    }
}

#[test]
fn points_at_mask_one_keep_their_bits() {
    let mut m = sphere(8, 1.0, 26, true);
    let pos = m.positions.clone();
    if let Some(AttrValues::F32(mask)) = m.attributes.get_mut("mask").map(|a| &mut a.values) {
        for (v, p) in mask.iter_mut().zip(&pos) {
            *v = if p[0] > 0.0 { 1.0 } else { 0.0 };
        }
    }
    let mut s = store(&m, small_nodes());
    let held = |s: &MeshStore| {
        let mask = s.attr_reader(some(s.attr_id("mask")));
        let mut out: Vec<[u32; 3]> = (snap(s).nodes.iter())
            .flat_map(|n| (n.base..).zip(n.pos.clone()))
            .filter(|&(v, p)| !p.is_dead() && mask.scalar(v) >= 1.0)
            .map(|(_, p)| p.to_array().map(f32::to_bits))
            .collect();
        out.sort_unstable();
        out
    };
    let before = held(&s);
    // Across x = 0, so the dabs reach both sides.
    let samples: Vec<StrokeSample> = (0..=8)
        .map(|k| {
            let d = Vec3::new(-0.3 + 0.075 * k as f32, 0.1, 1.0);
            at(surface_point(&s, d), 0.3)
        })
        .collect();
    let live = some(run_preset(&mut s, "draw", refining(constant(0.04)), &samples).live);
    assert!(live.ops() > 50, "{live:?}");
    assert_eq!(held(&s), before, "points at mask 1 neither move nor go");
}

#[test]
fn strokes_that_never_refine_keep_their_topology() {
    for stem in ["mask-pen", "move", "move-elastic"] {
        let mut s = store(&sphere(8, 1.0, 27, false), small_nodes());
        let samples = path(&s, 800, 0.3);
        let out = run_preset(&mut s, stem, refining(constant(0.02)), &samples);
        assert!(out.live.is_none(), "{stem} never refines");
        assert!(out.step.is_none_or(|st| !st.topology), "{stem}");
    }
}

/// The Mask Pen painting face group 3 instead.
fn grouper() -> gs_core::BrushPreset {
    let text = (preset("mask-pen").text)
        .replace("column = \"mask\"", "column = \"group\"")
        .replace("value = 1.0", "value = 3");
    ok(gs_core::BrushPreset::from_toml(&text))
}

/// A stroke that paints color or face groups moves no point, so it leaves the topology alone
/// as the Mask Pen does (research 24, wrong result 8): over coarse quads that Live Detail
/// would otherwise split everywhere under the brush. Paint and Paint Blend only paint
/// (decision 0039, section 8); Draw painting as it draws still refines.
#[test]
fn painting_color_or_groups_keeps_the_topology() {
    let draw = preset("draw").text + "\n[paint]\nchannel = \"color\"\n";
    let brushes = [
        ("paint", preset("paint-draw").preset, false),
        ("paint blend", preset("paint-blend").preset, false),
        ("group", grouper(), false),
        (
            "draw painting",
            ok(gs_core::BrushPreset::from_toml(&draw)),
            true,
        ),
    ];
    for (name, p, refines) in brushes {
        let mut s = store(&grid(6), small_nodes());
        let before = (s.vertex_count(), s.face_count());
        let samples: Vec<StrokeSample> = (0..=8)
            .map(|k| at(Vec3::new(-0.4 + 0.1 * k as f32, 0.05, 0.0), 0.3))
            .collect();
        let mut st = ok(Stroke::begin(&mut s, &p, refining(constant(0.02))));
        ok(st.add(&mut s, &samples));
        let out = st.finish(&mut s);
        let painted = match s.to_plain().attributes.get("color").map(|a| &a.values) {
            Some(AttrValues::U32(v)) => v.iter().filter(|&&c| c != u32::MAX).count(),
            _ => 0,
        };
        // Paint Blend has nothing to blend on an unpainted part.
        let paints = matches!(name, "paint" | "draw painting");
        assert_eq!(painted > 0, paints, "{name} painted {painted}");
        if refines {
            // Its nodes' buffers start again from their colors as refined (decision 0039,
            // section 6).
            assert!(out.live.is_some(), "{name} refines");
            assert_ne!((s.vertex_count(), s.face_count()), before, "{name}");
            continue;
        }
        assert!(out.live.is_none(), "{name}: {:?}", out.live);
        assert!(out.step.as_ref().is_some_and(|st| !st.topology), "{name}");
        assert_eq!((s.vertex_count(), s.face_count()), before, "{name}");
    }
}

/// What the renderer draws of each node: its points, normals and faces, and the corners of its
/// faces wherever they are stored.
fn drawn(s: &MeshStore) -> Vec<Vec<u32>> {
    let p = s.pos();
    (0..s.node_count() as u32)
        .map(|n| {
            let v = s.node(n);
            let corners = (v.faces.iter().flatten())
                .filter(|&&u| u != gs_core::NO)
                .map(|&u| p.get(u));
            (v.pos.iter().chain(v.nrm).copied().chain(corners))
                .flat_map(|x| x.to_array().map(f32::to_bits))
                .chain(v.faces.iter().flatten().copied())
                .collect()
        })
        .collect()
}

/// The renderer sends again only the nodes marked dirty: after every dab of a refining stroke,
/// and after a pass alone, each node whose points, normals or faces changed, or whose faces'
/// corners in other nodes moved, is marked. (Refining can leave a node storing a face none of
/// whose corners it owns; its halo did not hold that face, so a dab moving the face's corners
/// neither marked the node nor refreshed its box, and the GPU drew a stale box; a pass moving
/// them did not mark it either. Review 0006's m1 fix exposed it.)
#[test]
fn every_dab_and_pass_marks_each_node_whose_drawing_changed() {
    let mut s = store(&sphere(10, 1.0, 25, true), small_nodes());
    for (i, stem) in REFINING.iter().enumerate() {
        let p = preset(stem).preset;
        let mut prm = refining(constant(0.05));
        prm.alpha = shipped_alpha(&p);
        let samples = path(&s, 700 + i as u64, 0.25);
        let mut st = ok(Stroke::begin(&mut s, &p, prm));
        s.take_dirty();
        let mut before = drawn(&s);
        let mut check_marks = |s: &mut MeshStore, what: &str| {
            let dirty = s.take_dirty();
            let now = drawn(s);
            for (n, d) in now.iter().enumerate() {
                let marked = dirty.iter().any(|x| x.0 == n as u32);
                assert!(
                    marked || before.get(n) == Some(d),
                    "{stem}, {what}: node {n} changed unmarked"
                );
            }
            before = now;
        };
        for (k, x) in samples.iter().enumerate() {
            ok(st.add(&mut s, std::slice::from_ref(x)));
            check_marks(&mut s, &format!("dab {k}"));
        }
        st.finish(&mut s);
        check_marks(&mut s, "the end");
        // A pass alone, coarser then finer: its relax moves points other nodes draw.
        for (k, x) in path(&s, 900 + i as u64, 0.3).iter().enumerate() {
            let ld = constant(if k % 2 == 0 { 0.12 } else { 0.03 });
            let mut rep = LiveReport::default();
            gs_core::live::refine(&mut s, &[(x.pos, 0.3)], &ld, [false; 3], &mut rep);
            check_marks(&mut s, &format!("pass {k}"));
        }
    }
}
