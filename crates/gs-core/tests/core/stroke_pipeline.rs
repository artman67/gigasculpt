//! The stroke pipeline's own behavior: stroke classes, gestures, refused input, and the work
//! counter that keeps a small dab small (ARCHITECTURE 4.3, 10.1).

use gs_core::brush::{BrushPreset, StrokeSpec};
use gs_core::symmetry::Symmetry;
use gs_core::{AttrDef, AttrKind, Domain, Stroke, StrokeParams, StrokeSample, Vec3, View, stroke};
use gs_protocol::PlainMesh;
use gs_testkit::compare::canonical;
use gs_testkit::measure::stats;

use crate::brush_kit::{at, front, ico, masked, params, preset, run, store};
use crate::common::{ok, sphere, surface_point};

fn same(a: &PlainMesh, b: &PlainMesh) -> bool {
    canonical(a) == canonical(b)
}

/// The Draw preset with its stroke replaced.
fn draw_with(stroke_toml: &str) -> BrushPreset {
    let text = preset("draw").text;
    let cut = text.find("[stroke]").unwrap_or(text.len());
    ok(BrushPreset::from_toml(&format!(
        "{}{stroke_toml}",
        &text[..cut]
    )))
}

fn top(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 1.0).normalized()
}

#[test]
fn bad_parameters_and_samples_are_refused_and_change_nothing() {
    let p = preset("draw").preset;
    let mut m = store(&ico(3));
    let before = m.to_plain();
    let mut bad_params = Vec::new();
    let mut prm = params(None);
    prm.view.dir = Vec3::ZERO;
    bad_params.push((prm, "view direction"));
    let mut prm = params(None);
    prm.view.dir = Vec3::new(f32::INFINITY, 0.0, 0.0);
    bad_params.push((prm, "view direction"));
    for radial in [(3, 4), (1, 1), (0, 65)] {
        let mut prm = params(None);
        prm.symmetry.radial = Some(radial);
        bad_params.push((prm, "radial symmetry"));
    }
    for clip in [-0.1, f32::NAN] {
        let mut prm = params(None);
        prm.symmetry.clip = clip;
        bad_params.push((prm, "clamp distance"));
    }
    for (prm, want) in bad_params {
        let e = Stroke::begin(&mut m, &p, prm).err().map(|e| e.to_string());
        assert!(
            e.as_deref().is_some_and(|e| e.contains(want)),
            "{e:?} should say {want}"
        );
        assert!(!m.step_open(), "a refused stroke opens no step");
    }
    // A preset built in code skips the file's checks; the stroke still refuses its strength.
    let strong = BrushPreset {
        strength: 1.5,
        ..p.clone()
    };
    let e = Stroke::begin(&mut m, &strong, params(None)).err();
    let e = e.map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("strength must be from 0 to 1"), "{e}");
    assert!(!m.step_open(), "a refused stroke opens no step");
    let g = top(0.1, 0.1);
    let bad = [
        StrokeSample {
            pressure: 1.5,
            ..at(g, 0.3)
        },
        StrokeSample {
            pressure: -0.1,
            ..at(g, 0.3)
        },
        StrokeSample {
            pressure: f32::NAN,
            ..at(g, 0.3)
        },
        at(g, 0.0),
        at(g, f32::INFINITY),
        at(Vec3::new(f32::NAN, 0.0, 1.0), 0.3),
        at(Vec3::new(0.0, f32::INFINITY, 1.0), 0.3),
        at(Vec3::new(0.0, 0.0, f32::NEG_INFINITY), 0.3),
    ];
    for b in bad {
        let e = stroke(&mut m, &p, params(None), &[at(g, 0.3), b]);
        let msg = e.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            msg.contains("sample 1"),
            "{b:?} refused, naming the sample: {msg}"
        );
        assert!(same(&before, &m.to_plain()), "{b:?}: nothing changed");
        assert!(
            !m.step_open() && m.undo_len() == 0,
            "{b:?}: no step left behind"
        );
    }
    let mut k = ok(Stroke::begin(&mut m, &p, params(None)));
    ok(k.add(&mut m, &[at(g, 0.3), at(top(0.3, 0.1), 0.3)]));
    assert!(
        k.add(&mut m, &[at(g, 0.0)]).is_err(),
        "a bad later batch is refused"
    );
    k.cancel(&mut m);
    assert!(
        same(&before, &m.to_plain()) && !m.step_open(),
        "cancel restores everything"
    );
}

#[test]
fn an_anchored_stroke_is_one_dab_redone_from_the_start() {
    let press = draw_with("[stroke]\nclass = \"anchored\"\ntap = 0.5");
    let follow = draw_with("[stroke]\nclass = \"anchored\"\nfollow = true\ntap = 0.5");
    let tap = draw_with("[stroke]\nclass = \"dab\"\ntap = 0.5");
    let (g, a, b) = (top(0.1, 0.1), top(0.35, 0.1), top(0.1, 0.3));
    let one = |p: &BrushPreset, s: &[StrokeSample]| {
        let mut m = store(&ico(4));
        run(&mut m, p, params(None), s);
        m.to_plain()
    };
    let dist = |x: Vec3| (x - g).len2().sqrt();
    let mut m = store(&ico(4));
    let mut k = ok(Stroke::begin(&mut m, &press, params(None)));
    ok(k.add(&mut m, &[at(g, 0.2), at(a, 0.2)]));
    assert!(
        same(&m.to_plain(), &one(&tap, &[at(g, dist(a))])),
        "sized by the drag"
    );
    ok(k.add(&mut m, &[at(b, 0.2)]));
    assert!(
        same(&m.to_plain(), &one(&tap, &[at(g, dist(b))])),
        "redone, not added"
    );
    ok(k.add(&mut m, &[at(g, 0.2)]));
    assert!(
        same(&m.to_plain(), &ico_plain()),
        "a drag of zero places nothing"
    );
    let rep = k.finish(&mut m);
    assert_eq!(rep.dabs, 2, "two dabs placed");
    // The dab takes the pen's pressure now, not at the press.
    let light = |p| StrokeSample {
        pressure: p,
        ..at(g, 0.2)
    };
    let pressed = one(
        &press,
        &[
            light(0.3),
            StrokeSample {
                pressure: 0.9,
                ..at(a, 0.2)
            },
        ],
    );
    let now = StrokeSample {
        pressure: 0.9,
        ..at(g, dist(a))
    };
    assert!(same(&pressed, &one(&tap, &[now])), "the latest pressure");
    let follows = one(&follow, &[at(g, 0.2), at(a, 0.25)]);
    assert!(
        same(&follows, &one(&tap, &[at(a, 0.25)])),
        "follow: at the pointer, brush size"
    );
}

fn ico_plain() -> PlainMesh {
    store(&ico(4)).to_plain()
}

#[test]
fn a_curve_stroke_lays_its_dabs_when_it_ends() {
    let line = preset("clay-line").preset;
    let StrokeSpec::Curve { spacing, .. } = line.stroke else {
        panic!("clay line is a curve stroke");
    };
    let mut bent = line.clone();
    bent.stroke = StrokeSpec::Curve {
        spacing,
        straight: false,
        smoothing: gs_core::brush::Smoothing::default(),
    };
    let mut dabs = line.clone();
    dabs.stroke = StrokeSpec::Dab {
        spacing,
        tap: 0.0,
        smoothing: gs_core::brush::Smoothing::default(),
        directional: false,
    };
    let path = [
        at(top(0.3, 0.0), 0.2),
        at(top(0.1, 0.3), 0.2),
        at(top(-0.2, 0.0), 0.2),
    ];
    let mut m = store(&ico(4));
    // Clay Line stamps its square, as `run` gives every stroke here.
    let mut prm = params(None);
    prm.alpha = crate::brush_kit::shipped_alpha(&line);
    let mut k = ok(Stroke::begin(&mut m, &line, prm));
    ok(k.add(&mut m, &path));
    assert!(
        same(&m.to_plain(), &ico_plain()),
        "nothing until the stroke ends"
    );
    k.finish(&mut m);
    let straight = m.to_plain();
    let mut d = store(&ico(4));
    run(&mut d, &dabs, params(None), &[path[0], path[2]]);
    assert!(
        same(&straight, &d.to_plain()),
        "a straight line from the first to the last point"
    );
    let mut c = store(&ico(4));
    run(&mut c, &bent, params(None), &path);
    let mut e = store(&ico(4));
    run(&mut e, &dabs, params(None), &path);
    assert!(
        same(&c.to_plain(), &e.to_plain()),
        "a bent curve follows every point"
    );
    assert!(
        !same(&c.to_plain(), &straight),
        "and differs from the straight line"
    );
}

#[test]
fn stroke_smoothing_trails_the_pointer() {
    let roped = draw_with("[stroke]\nclass = \"dab\"\nsmoothing = { radius = 1.0 }");
    let lagged = draw_with("[stroke]\nclass = \"dab\"\nsmoothing = { factor = 0.5 }");
    let plain = draw_with("[stroke]\nclass = \"dab\"");
    let g = top(0.0, 0.0);
    let near = [at(g, 0.3), at(top(0.1, 0.0), 0.3), at(top(0.2, 0.05), 0.3)];
    let result = |p: &BrushPreset, s: &[StrokeSample]| {
        let mut m = store(&ico(4));
        run(&mut m, p, params(None), s);
        m.to_plain()
    };
    let press = result(&plain, &near[..1]);
    assert!(
        same(&result(&roped, &near), &press),
        "moves within the string place nothing"
    );
    assert!(
        !same(&result(&plain, &near), &press),
        "without smoothing they do"
    );
    let far = top(0.4, 0.0);
    let half = far - (far - g) * 0.5;
    let lag = result(&lagged, &[at(g, 0.3), at(far, 0.3)]);
    assert!(
        same(&lag, &result(&plain, &[at(g, 0.3), at(half, 0.3)])),
        "half the way"
    );
    // The string is one brush radius long (0.3 here): a pointer 0.5 away pulls the point to
    // 0.3 behind it.
    let d = far - g;
    let len = d.len2().sqrt();
    let trailing = far - d * (1.0 - (len - 0.3) / len);
    let pulled = result(&roped, &[at(g, 0.3), at(far, 0.3)]);
    assert!(len > 0.35, "the pointer goes past the string ({len})");
    assert!(
        same(&pulled, &result(&plain, &[at(g, 0.3), at(trailing, 0.3)])),
        "trails by the string"
    );
}

/// Clay Build over the top of the sphere, `legs` times back and forth, with Buildup on or off:
/// the highest rise above the start.
fn rise(accumulate: bool, legs: usize) -> f32 {
    let mut p = preset("clay-build").preset;
    p.sampling.accumulate = accumulate;
    let mut m = store(&ico(4));
    let before = m.to_plain();
    let path: Vec<StrokeSample> = (0..=legs)
        .map(|i| at(top(if i % 2 == 0 { -0.15 } else { 0.15 }, 0.0), 0.2))
        .collect();
    run(&mut m, &p, params(None), &path);
    let after = m.to_plain();
    let len = |p: [f32; 3]| Vec3::from_array(p).len2().sqrt();
    (after.positions.iter().zip(&before.positions))
        .map(|(a, b)| len(*a) - len(*b))
        .fold(0.0, f32::max)
}

#[test]
fn buildup_lets_a_doubled_back_stroke_stack() {
    // Catalog brush-settings.samples, its Check as decision 0058 (section 5) words it: with
    // Buildup each pass reads the height the last one added and fills above it; without, every
    // pass reads the surface as the stroke started, so the passes level off. Measured after 1,
    // 2 and 8 passes: on 0.0329, 0.0608 and 0.1196; off 0.0191, 0.0230 and 0.0292.
    let on = [1, 2, 8].map(|n| rise(true, n));
    let off = [1, 2, 8].map(|n| rise(false, n));
    assert!(
        on[1] >= 1.7 * on[0] && on[2] >= 3.0 * on[0],
        "with Buildup the second pass nearly doubles the rise, and the rest add on: {on:?}"
    );
    assert!(
        off[1] <= 1.25 * off[0] && off[2] <= 1.6 * off[0],
        "without it the passes level off: {off:?}"
    );
}

#[test]
fn flat_polish_keeps_the_plane_of_its_first_dab() {
    let polish = preset("flat-polish").preset;
    let mut follow = polish.clone();
    if let gs_core::brush::KernelSpec::Plane(k) = &mut follow.kernel {
        k.source = gs_core::brush::PlaneSource::Dab;
    }
    let path = crate::brush_kit::arc(top(0.5, 0.0), top(-0.5, 0.1), 10, 0.2);
    let directions = |p: &BrushPreset| {
        let mut m = store(&ico(4));
        let before = m.to_plain();
        run(&mut m, p, params(None), &path);
        let after = m.to_plain();
        (after.positions.iter().zip(&before.positions))
            .map(|(a, b)| Vec3::from_array(*a) - Vec3::from_array(*b))
            .filter(|d| d.len2() > 1e-8)
            .map(Vec3::normalized)
            .collect::<Vec<Vec3>>()
    };
    let spread = |ds: &[Vec3]| {
        ds.iter()
            .map(|d| d.cross(ds[0]).len2().sqrt())
            .fold(0.0, f32::max)
    };
    let (kept, refit) = (directions(&polish), directions(&follow));
    assert!(
        kept.len() > 20 && refit.len() > 20,
        "both moved many vertices"
    );
    assert!(
        spread(&kept) < 1e-3,
        "one plane: every move along its normal ({})",
        spread(&kept)
    );
    assert!(
        spread(&refit) > 0.1,
        "refitted planes follow the form ({})",
        spread(&refit)
    );
}

#[test]
fn clip_curve_pushes_the_shadow_side_onto_the_curve() {
    let clip = preset("clip-curve").preset;
    let line = [
        at(Vec3::new(-2.0, 0.3, 2.0), 0.1),
        at(Vec3::new(2.0, 0.3, 2.0), 0.1),
    ];
    for invert in [false, true] {
        let mut m = store(&masked(&ico(4)));
        let before = m.to_plain();
        let mut prm = params(None);
        prm.invert = invert;
        let rep = run(&mut m, &clip, prm, &line);
        assert_eq!(
            (rep.dabs, rep.max_dab_nodes),
            (1, rep.nodes.len()),
            "one gesture"
        );
        let after = m.to_plain();
        let mask = crate::brush_kit::floats(&after, "mask");
        let mut moved = 0;
        for (i, (b, a)) in before.positions.iter().zip(&after.positions).enumerate() {
            let shadow = if invert { b[1] < 0.3 } else { b[1] > 0.3 };
            if shadow && mask[i] < 1.0 {
                assert!(
                    (a[1] - 0.3).abs() < 1e-6,
                    "vertex {i} lands on the curve: {a:?}"
                );
                assert_eq!(
                    [a[0], a[2]].map(f32::to_bits),
                    [b[0], b[2]].map(f32::to_bits),
                    "only across the curve"
                );
                moved += 1;
            } else {
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "vertex {i} stays");
            }
        }
        assert!(moved > 100, "the shadow side moved ({moved})");
    }
    let mut m = store(&ico(4));
    let rep = run(&mut m, &clip, params(None), &line[..1]);
    assert_eq!(rep.dabs, 0, "one point is not a curve");
    assert!(same(&m.to_plain(), &ico_plain()), "and changes nothing");
}

#[test]
fn clip_curve_in_perspective_cuts_along_the_rays_from_the_eye() {
    let clip = preset("clip-curve").preset;
    let eye = Vec3::new(0.0, 0.0, 5.0);
    let (a, b) = (Vec3::new(-2.0, 0.3, 2.0), Vec3::new(2.0, 0.3, 2.0));
    let mut prm = params(None);
    prm.view = View {
        dir: Vec3::new(0.0, 0.0, -1.0),
        eye: Some(eye),
    };
    let mut m = store(&ico(4));
    let before = m.to_plain();
    run(&mut m, &clip, prm, &[at(a, 0.1), at(b, 0.1)]);
    let after = m.to_plain();
    let n = (b - a).cross((a - eye).normalized()).normalized();
    assert!((eye - a).dot(n).abs() < 1e-5, "the cut plane holds the eye");
    let mut moved = 0;
    for (bp, ap) in before.positions.iter().zip(&after.positions) {
        let (bp, ap) = (Vec3::from_array(*bp), Vec3::from_array(*ap));
        if (bp - a).dot(n) > 1e-6 {
            assert!(
                (ap - a).dot(n).abs() < 1e-5,
                "{bp:?} lands on the cut plane"
            );
            moved += 1;
        } else {
            assert_eq!(ap.bits(), bp.bits(), "{bp:?} stays");
        }
    }
    assert!(moved > 100, "the shadow side moved ({moved})");
}

#[test]
fn a_small_dab_touches_few_nodes_with_or_without_symmetry() {
    // Measured: 9 nodes and 109 boxes for one pass, 43 nodes for eight mirrored passes. A dab
    // that queried the box around all its passes, or every node, would touch all 346.
    let mesh = sphere(48, 0.5, 42, false);
    let draw = preset("draw").preset;
    let all = Symmetry {
        mirror: [true; 3],
        ..Symmetry::default()
    };
    for (sym, most, boxes_most) in [(Symmetry::default(), 12, 173), (all, 96, u64::MAX)] {
        let mut m = store(&mesh);
        let nodes = m.node_count();
        assert!(nodes > 300, "{nodes} nodes");
        let g = surface_point(&m, Vec3::new(0.3, 0.5, 0.8));
        let tested = m.work().boxes_tested;
        let rep = run(&mut m, &draw, params(Some(sym)), &[at(g, 0.04)]);
        let boxes = m.work().boxes_tested - tested;
        let touched = rep.max_dab_nodes;
        assert!(touched > 0 && touched <= most, "{touched} of {nodes} nodes");
        assert!(
            boxes <= boxes_most,
            "{boxes} boxes tested for {nodes} nodes"
        );
        assert_eq!(
            rep.nodes.len(),
            touched,
            "one dab: its nodes are the stroke's"
        );
    }
}

#[test]
fn attributes_beyond_the_limit_are_an_error_not_a_crash() {
    let mut m = store(&ico(2));
    for i in 0..61 {
        ok(m.add_attr(AttrDef::new(
            &format!("extra{i}"),
            Domain::Vertex,
            AttrKind::Scalar,
        )));
    }
    let e = Stroke::begin(&mut m, &preset("mask-pen").preset, params(None)).err();
    assert!(
        e.is_some_and(|e| e.message.contains("attributes")),
        "refused: too many attributes"
    );
    assert!(!m.step_open(), "no step left open");
    let draw = preset("draw").preset;
    assert!(
        stroke(&mut m, &draw, params(None), &[at(top(0.0, 0.0), 0.3)]).is_ok(),
        "other brushes still work"
    );
}

#[test]
fn inverted_inflate_shrinks_and_strength_zero_changes_nothing() {
    let inflate = preset("inflate").preset;
    let path = crate::brush_kit::arc(top(0.2, 0.0), top(-0.2, 0.1), 4, 0.3);
    let mut m = store(&ico(4));
    let v0 = stats(&m.to_plain()).volume;
    let mut prm = params(None);
    prm.invert = true;
    run(&mut m, &inflate, prm, &path);
    assert!(stats(&m.to_plain()).volume < v0, "Alt deflates");
    let mut z = store(&ico(4));
    let still = BrushPreset {
        strength: 0.0,
        ..inflate
    };
    let rep = run(&mut z, &still, StrokeParams::new(front()), &path);
    assert!(
        same(&z.to_plain(), &ico_plain()),
        "strength 0 changes nothing"
    );
    assert_eq!(
        rep.step.map(|s| s.nodes),
        Some(0),
        "and records no undo step"
    );
}

#[test]
fn a_bent_clip_curve_cuts_along_the_nearest_segment() {
    let clip = preset("clip-curve").preset;
    // A lopsided V, over a mesh moved away from the origin (and from any unit sphere there).
    let pts = [
        Vec3::new(-2.0, 0.7, 2.0),
        Vec3::new(0.25, 0.0, 2.0),
        Vec3::new(2.0, 0.9, 2.0),
    ];
    let mut shifted = ico(4);
    for p in &mut shifted.positions {
        p[2] -= 4.0;
    }
    let mut m = store(&shifted);
    let before = m.to_plain();
    // A double tap at the middle sample makes the V's corner (decision 0055).
    let mut k = ok(Stroke::begin(&mut m, &clip, params(None)));
    for (i, p) in pts.iter().enumerate() {
        ok(k.add(&mut m, &[at(*p, 0.1)]));
        if i == 1 {
            k.point();
            k.corner();
        }
    }
    k.finish(&mut m);
    let after = m.to_plain();
    // Each segment sweeps a plane along the view (-z); its normal points to the shadow side,
    // the left of the curve's direction as the camera sees it: above the V.
    let cut = |a: Vec3, b: Vec3| (a, (b - a).cross(Vec3::new(0.0, 0.0, -1.0)).normalized());
    let cuts = [cut(pts[0], pts[1]), cut(pts[1], pts[2])];
    let flat_dist = |p: Vec3, a: Vec3, b: Vec3| {
        let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.0);
        crate::brush_kit::to_segment(flat(p), flat(a), flat(b))
    };
    let mut moved = 0;
    for (b, a) in before.positions.iter().zip(&after.positions) {
        let p = Vec3::from_array(*b);
        let (d0, d1) = (flat_dist(p, pts[0], pts[1]), flat_dist(p, pts[1], pts[2]));
        if (d0 - d1).abs() < 1e-4 {
            // Equally near both segments: rounding picks one, so skip it.
            continue;
        }
        let near = usize::from(d1 < d0);
        let (o, n) = cuts[near];
        let h = (p - o).dot(n);
        let want = if h > 0.0 { p - n * h } else { p };
        assert!(
            (Vec3::from_array(*a) - want).len2().sqrt() < 1e-5,
            "{p:?} went to {a:?}, not {want:?}"
        );
        moved += usize::from(h > 0.0);
    }
    assert!(moved > 100, "the shadow side moved ({moved})");
}

#[test]
fn the_spacing_pressure_curve_scales_the_step() {
    // Half the spacing through the pressure curve (a fraction of the slider, review 0004 M3)
    // is half the spacing in the preset.
    let curved = draw_with(
        "[stroke]\nclass = \"dab\"\nspacing = 0.2\n[pressure]\nspacing = [[0, 0.5], [1, 0.5]]",
    );
    let wide = draw_with("[stroke]\nclass = \"dab\"\nspacing = 0.2");
    let narrow = draw_with("[stroke]\nclass = \"dab\"\nspacing = 0.1");
    let path = crate::brush_kit::arc(top(0.3, 0.0), top(-0.3, 0.1), 5, 0.2);
    let result = |p: &BrushPreset| {
        let mut m = store(&ico(4));
        let rep = run(&mut m, p, params(None), &path);
        (m.to_plain(), rep.dabs)
    };
    let ((c, cd), (_, wd), (n, nd)) = (result(&curved), result(&wide), result(&narrow));
    assert!(same(&c, &n) && cd == nd, "the curve halves the step");
    assert!(
        nd > wd,
        "and a step that is not halved places fewer dabs ({wd} vs {nd})"
    );
}

#[test]
fn edge_walks_stay_within_the_footprint() {
    let mesh = sphere(48, 0.5, 42, false);
    let draw = preset("draw").preset;
    let grab = preset("move-connected");
    let topo = draw_with("[stroke]\nclass = \"dab\"\n[automask]\ntopology = 2.0");
    let all = Symmetry {
        mirror: [true; 3],
        ..Symmetry::default()
    };
    let walked = |p: &BrushPreset, sym: Option<Symmetry>, two: bool| {
        let mut m = store(&mesh);
        let g = surface_point(&m, Vec3::new(0.3, 0.5, 0.8));
        let s = [at(g, 0.04), at(g + Vec3::new(0.01, 0.0, 0.0), 0.04)];
        run(&mut m, p, params(sym), if two { &s } else { &s[..1] }).walked
    };
    assert_eq!(
        walked(&draw, None, false),
        1,
        "a plain dab walks to its start vertex only"
    );
    assert_eq!(
        walked(&draw, Some(all), false),
        8,
        "one start vertex per pass"
    );
    let connected = walked(&grab.preset, None, true);
    let ranged = walked(&topo, None, false);
    // Measured: 5 and 17 vertices on a mesh of about 14,000.
    assert!(
        connected > 1 && connected < 30,
        "a connected move walks its radius ({connected})"
    );
    assert!(
        ranged > connected && ranged < 80,
        "the topology automask walks its range ({ranged})"
    );
}

#[test]
fn a_pen_jump_places_a_bounded_number_of_dabs_spread_over_the_jump() {
    // At the smallest spacing a jump across the grid needs about 1,340 cells; one sample
    // places at most MOST_DABS_PER_SAMPLE, stretched evenly over the whole jump.
    let dense = draw_with("[stroke]\nclass = \"dab\"\nspacing = 0.01");
    let mesh = store(&crate::brush_kit::grid(40)).to_plain();
    let (a, b) = (Vec3::new(-0.95, -0.95, 0.0), Vec3::new(0.95, 0.95, 0.0));
    let mut m = store(&mesh);
    let rep = run(&mut m, &dense, params(None), &[at(a, 0.1), at(b, 0.1)]);
    let after = m.to_plain();
    let most = gs_core::stroke::MOST_DABS_PER_SAMPLE as u32;
    assert!(rep.dabs > most && rep.dabs <= most + 2, "{} dabs", rep.dabs);
    let moved_near = |p: Vec3| {
        (mesh.positions.iter().zip(&after.positions)).any(|(x, y)| {
            x[2].to_bits() != y[2].to_bits() && (Vec3::from_array(*x) - p).len2().sqrt() < 0.08
        })
    };
    assert!(
        moved_near(a) && moved_near(b),
        "the dabs reach both ends of the jump"
    );
}
