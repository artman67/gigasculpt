//! Differential tests (ARCHITECTURE 10.1, kind 3): one dab of every preset that claims
//! `reference`, against the slow `f64` reference in gs-testkit, which shares only the preset
//! file. And the falloff, symmetry passes and curves against plain formulas.

use gs_core::brush::{KernelSpec, StrokeSpec};
use gs_core::curve::Curve;
use gs_core::symmetry::Symmetry;
use gs_core::{BrushPreset, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::reference::stroke::{Dab, dab};

use crate::brush_kit::{at, floats, params, run, store};
use crate::common::{ok, sphere, surface_point};

/// Where a test dab lands and how the camera sees it.
#[derive(Clone, Copy)]
struct Shot {
    /// The dab sits on the mesh vertex nearest this direction from the origin.
    toward: Vec3,
    /// The camera direction; `None` looks at the dab from outside, toward the origin.
    view: Option<Vec3>,
    invert: bool,
}

/// The usual shot: the upper front of the mesh, seen from outside.
fn shot(invert: bool) -> Shot {
    Shot {
        toward: Vec3::new(0.4, 0.5, 0.8),
        view: None,
        invert,
    }
}

/// One dab of `text` through gs-core and through the reference, from the same mesh. Fails if
/// they differ by more than ten times the worst difference measured (2.8e-7, Mask Pen) or if
/// the dab does nothing.
fn compare(name: &str, text: &str, mesh: &PlainMesh, sh: Shot) {
    let change = measure(name, text, mesh, sh);
    assert!(
        change > 1e-3,
        "{name} (invert {}): the dab did something",
        sh.invert
    );
}

/// [`compare`] with and without Alt: both agree with the reference, and at least one does
/// something (a brush that only cuts may find nothing to fill on a convex mesh).
fn both_ways(name: &str, text: &str, mesh: &PlainMesh) {
    let change = [false, true].map(|invert| measure(name, text, mesh, shot(invert)));
    assert!(
        change.iter().any(|&c| c > 1e-3),
        "{name}: the dab did something one way or the other: {change:?}"
    );
}

/// One dab of `text` through gs-core and through the reference: fails if they differ by more
/// than the tolerance, and returns the largest change the reference made.
fn measure(name: &str, text: &str, mesh: &PlainMesh, sh: Shot) -> f64 {
    let invert = sh.invert;
    let p = ok(BrushPreset::from_toml(text));
    let mut m = store(mesh);
    let before = m.to_plain();
    let g = surface_point(&m, sh.toward);
    // A hook's second sample stays within its first cell, half a step (0.035 here) from the press.
    let hook = matches!(p.kernel, KernelSpec::Hook(_));
    let to = g + Vec3::new(-0.03, 0.04, 0.01) * if hook { 0.5 } else { 1.0 };
    let (radius, pressure) = (0.35, 0.8);
    let sample = |pos| gs_core::StrokeSample {
        pressure,
        ..at(pos, radius)
    };
    let two = matches!(p.stroke, StrokeSpec::Drag { .. }) || hook;
    let samples = if two {
        vec![sample(g), sample(to)]
    } else {
        vec![sample(g)]
    };
    let view = sh.view.unwrap_or((Vec3::ZERO - g).normalized());
    let mut prm = params(None);
    prm.view.dir = view;
    prm.invert = invert;
    prm.alpha = crate::brush_kit::shipped_alpha(&p);
    let levels = prm.alpha.as_deref().map(crate::brush_kit::levels_of);
    run(&mut m, &p, prm, &samples);
    let got = m.to_plain();
    let d = |v: Vec3| v.to_array().map(f64::from);
    let dab_in = Dab {
        alpha: levels.as_deref(),
        at: d(g),
        to: two.then(|| d(to)),
        radius: f64::from(radius),
        pressure: f64::from(pressure),
        view: d(view),
        strength: None,
        invert,
    };
    let want = ok(dab(text, &before, &dab_in));
    let far = |a: [f64; 3], b: [f32; 3]| {
        (0..3)
            .map(|k| (a[k] - f64::from(b[k])).abs())
            .fold(0.0, f64::max)
    };
    let (mut worst, mut change) = (0.0f64, 0.0f64);
    for ((g, w), b) in got
        .positions
        .iter()
        .zip(&want.positions)
        .zip(&before.positions)
    {
        worst = worst.max(far(*w, *g));
        change = change.max(far(*w, *b));
    }
    let (gm, bm) = (floats(&got, "mask"), floats(&before, "mask"));
    for ((g, w), b) in gm.iter().zip(&want.mask).zip(bm) {
        worst = worst.max((f64::from(*g) - w).abs());
        change = change.max((w - f64::from(b)).abs());
    }
    assert!(
        worst <= 3e-6,
        "{name} (invert {invert}): {worst} from the reference"
    );
    change
}

#[test]
fn one_dab_of_each_kernel_matches_the_slow_reference() {
    let mesh = sphere(10, 0.5, 77, true);
    let mut kernels = std::collections::BTreeSet::new();
    for s in crate::brush_kit::shipped() {
        if s.preset.claims.iter().any(|c| c == "reference") {
            kernels.insert(format!("{:?}", std::mem::discriminant(&s.preset.kernel)));
            both_ways(&s.stem, &s.text, &mesh);
        }
    }
    assert_eq!(
        kernels.len(),
        6,
        "every kernel kind has a preset checked: {kernels:?}"
    );
}

#[test]
fn kernel_options_no_shipped_preset_uses_match_the_slow_reference() {
    let mesh = sphere(10, 0.5, 78, true);
    let head = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.6\n[kernel]\n";
    let variants = [
        "type = \"smooth\"\nmode = \"tangent\"\ngain = 5.0",
        "type = \"smooth\"\nmode = \"relax\"\ngain = 5.0",
        "type = \"offset\"\nterms = [{ dir = \"view\", gain = 1.0 }, { dir = \"x\", gain = 0.5 }]",
        "type = \"offset\"\nterms = [{ dir = \"y\", gain = 1.0, exponent = 3.0 }, { dir = \"z\", gain = -0.5 }]",
        "type = \"offset\"\nterms = [{ dir = \"to_center\", gain = 1.5 }]",
        "type = \"plane\"\nside = \"above\"\ngain = 3.0\nheight = 0.2\n[depth]\nimbed = -0.1",
        "type = \"plane\"\ngain = 3.0\ndepth = 0.05\n[depth]\nimbed = 0.05\n[pressure]\nimbed = [[0, 0.2], [1, 0.9]]",
        "type = \"plane\"\nsource = \"stroke_normal\"\nside = \"below\"\ngain = 3.0\n[depth]\nimbed = 0.1",
        "type = \"smooth\"\nmode = \"uniform\"\ngain = 3.0\n[pressure]\nstrength = { points = [[0, 0], [0.4, 0.7], [0.8, 0.75], [1, 1]], smooth = true, sharp = [2] }",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[tip]\nprofile = \"custom\"\npoints = { points = [[0, 1], [0.4, 0.8], [1, 0]], smooth = true }",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[post]\nauto_smooth = 3.0\nauto_smooth_mode = \"relax\"",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[post]\nauto_smooth = 3.0\nauto_smooth_mode = \"tangent\"\nauto_smooth_max_faces = 1000000",
        "type = \"move\"\nselect = \"ball\"\n[stroke]\nclass = \"drag\"\n[post]\nauto_smooth = 2.0\nauto_smooth_mode = \"relax\"",
        // Alphas: a signed carve with tiles, an intensity on a plane, axes from a drag and a
        // hook's step, sizes in the post effects, the elastic reach, a connected move.
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[tip.alpha]\nimage = \"ring\"\nmid = 0.4\ntiles = [2, 3]",
        "type = \"plane\"\nside = \"below\"\ngain = 3.0\n[depth]\nimbed = 0.1\n[tip.alpha]\nimage = \"grainy-round\"\nmid = 0.3",
        "type = \"move\"\nselect = \"ball\"\n[stroke]\nclass = \"drag\"\n[tip.alpha]\nimage = \"ring\"",
        "type = \"hook\"\npinch = 0.3\n[tip.alpha]\nimage = \"ring\"\nmid = 0.2",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[tip.alpha]\nimage = \"ring\"\nmid = 0.5\n[post]\nauto_smooth = 3.0\ngravity = 0.6",
        "type = \"move\"\nselect = \"elastic\"\nsupport = 2.5\n[stroke]\nclass = \"drag\"\n[tip.alpha]\nimage = \"soft-round\"",
        "type = \"move\"\nselect = \"connected\"\n[stroke]\nclass = \"drag\"\n[tip.alpha]\nimage = \"square\"",
        "type = \"attribute\"\ncolumn = \"mask\"\nop = \"blur\"\ngain = 9.0",
        "type = \"attribute\"\ncolumn = \"mask\"\nop = \"sharpen\"\ngain = 9.0",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.2 }]\n[post]\ngravity = -0.7\ngravity_dir = [1, 2, 0]",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[post]\nauto_smooth = 4.0",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.3 }]\n[post]\nauto_smooth = 3.0\ngravity = 0.6",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[tip]\nprofile = \"custom\"\npoints = [[0, 1], [0.3, 0.9], [1, 0]]\nfocal = -40\nhardness = 0.2",
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n[sampling]\nnormal_radius = 0.2\nposition_radius = 0.9\nfront_only = false\n[pressure]\nsize = [[0, 0.3], [1, 1]]\nstrength = [[0, 1], [0.5, 0.4], [1, 0.2]]",
        "type = \"move\"\nselect = \"elastic\"\nsupport = 2.5\npoisson = 0.1\n[stroke]\nclass = \"drag\"",
        "type = \"hook\"\npinch = 0.3",
        "type = \"move\"\nselect = \"connected\"\n[stroke]\nclass = \"drag\"",
    ];
    for (i, v) in variants.iter().enumerate() {
        let text = format!("{head}{v}\n");
        for invert in [false, true] {
            compare(&format!("variant {i}"), &text, &mesh, shot(invert));
        }
    }
    // Settings above the kernel: the Brush Modifier both ways (through its pressure curve) on
    // a brush with a term of its own, and Subtract.
    let top = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.6\n";
    let draw = "[kernel]\ntype = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]";
    let curve = "[pressure]\nmodifier = [[0, 0.3], [1, 0.9]]";
    for more in [
        format!("modifier = 70\n{draw}\n{curve}"),
        format!("modifier = -45\n{draw}\n{curve}"),
        format!("mode = \"subtract\"\n{draw}"),
        "mode = \"subtract\"\n[kernel]\ntype = \"plane\"\ngain = 3.0\n[depth]\nimbed = 0.1".into(),
    ] {
        for invert in [false, true] {
            compare(&more, &format!("{top}{more}\n"), &mesh, shot(invert));
        }
    }
}

#[test]
fn every_tip_profile_follows_its_formula() {
    type Case<'a> = (&'a str, &'a str, &'a dyn Fn(f64) -> f64);
    let smooth = |d: f64| 3.0 * d * d * d * d - 4.0 * d * d * d + 1.0;
    let cases: [Case; 7] = [
        ("profile = \"smooth\"", "smooth", &smooth),
        ("profile = \"sphere\"", "sphere", &|d| (1.0 - d * d).sqrt()),
        ("profile = \"sharp\"", "sharp", &|d| (1.0 - d) * (1.0 - d)),
        ("profile = \"linear\"", "linear", &|d| 1.0 - d),
        ("profile = \"constant\"", "constant", &|_| 1.0),
        (
            "profile = \"custom\"\npoints = [[0, 1], [0.5, 0.2], [1, 0]]",
            "custom",
            &|d| {
                if d <= 0.5 {
                    1.0 - 1.6 * d
                } else {
                    0.4 - 0.4 * d
                }
            },
        ),
        ("hardness = 0.5\nfocal = 50", "hard, narrow", &|d| {
            let e = ((d - 0.5) / 0.5).max(0.0);
            smooth(e.sqrt())
        }),
    ];
    for (tip, what, want) in cases {
        let text = format!(
            "name = \"T\"\nid = \"t\"\nfamily = \"Build\"\nstrength = 0.5\n[kernel]\ntype = \"smooth\"\nmode = \"uniform\"\ngain = 1\n[tip]\n{tip}\n"
        );
        let tip = ok(BrushPreset::from_toml(&text)).tip.tip();
        for i in 0..=40 {
            let d = f64::from(i) / 32.0;
            let w = if d >= 1.0 { 0.0 } else { want(d) };
            let got = f64::from(tip.falloff(d as f32));
            assert!((got - w).abs() < 2e-6, "{what} at {d}: {got} not {w}");
        }
        assert!(
            tip.falloff(f32::NAN).abs() < f32::MIN_POSITIVE,
            "{what}: NaN is outside"
        );
    }
}

#[test]
fn mirror_and_radial_passes_are_the_plain_maps() {
    let p = Vec3::new(0.3, -0.7, 1.9);
    let sym = Symmetry {
        mirror: [true, false, true],
        radial: Some((1, 3)),
        ..Symmetry::default()
    };
    let got: Vec<[f32; 3]> = sym.passes().iter().map(|x| x.apply(p).to_array()).collect();
    let mut want = Vec::new();
    for k in 0..3 {
        let a = std::f64::consts::TAU * f64::from(k) / 3.0;
        for (fx, fz) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            let (x, y, z) = (f64::from(p.x) * fx, f64::from(p.y), f64::from(p.z) * fz);
            // Right-handed turn around Y: z' = z cos + x sin, x' = x cos - z sin.
            let (c, s) = (libm::cos(a), libm::sin(a));
            want.push([z * s + x * c, y, z * c - x * s]);
        }
    }
    assert_eq!(got.len(), want.len(), "pass count");
    for (g, w) in got.iter().zip(&want) {
        for k in 0..3 {
            assert!(
                (f64::from(g[k]) - w[k]).abs() < 1e-5,
                "pass {g:?} want {w:?}"
            );
        }
    }
    assert_eq!(
        got[0],
        p.to_array(),
        "the first pass is the identity, exactly"
    );
}

#[test]
fn curves_interpolate_between_their_points() {
    let c = ok(Curve::new(vec![[0.0, 0.0], [0.25, 1.0], [1.0, 0.5]]));
    for (x, want) in [
        (0.0, 0.0),
        (0.125, 0.5),
        (0.25, 1.0),
        (0.625, 0.75),
        (1.0, 0.5),
        (-1.0, 0.0),
        (2.0, 0.5),
        (f32::NAN, 0.0),
    ] {
        assert!(
            (c.eval(x) - want).abs() < 1e-6,
            "curve at {x}: {} not {want}",
            c.eval(x)
        );
    }
    assert!((Curve::linear().eval(0.3) - 0.3).abs() < 1e-7, "linear");
    assert!(
        (Curve::constant(0.7).eval(0.9) - 0.7).abs() < 1e-7,
        "constant"
    );
}

/// Random curve points: `n` points from x = 0 to x = 1, y in 0 to 2, rising when `rising`.
fn random_points(rng: &mut gs_testkit::rng::SplitMix64, n: usize, rising: bool) -> Vec<[f32; 2]> {
    let mut u = || (rng.next_u64() >> 40) as f32 / 16_777_216.0;
    let mut xs: Vec<f32> = (0..n - 2).map(|_| 0.02 + 0.96 * u()).collect();
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    let xs: Vec<f32> = [0.0].into_iter().chain(xs).chain([1.0]).collect();
    let mut ys: Vec<f32> = xs.iter().map(|_| 2.0 * u()).collect();
    if rising {
        ys.sort_by(f32::total_cmp);
    }
    xs.into_iter().zip(ys).map(|(x, y)| [x, y]).collect()
}

#[test]
fn sharp_curves_are_their_straight_segments_bit_for_bit() {
    #[derive(serde::Deserialize)]
    struct File {
        c: Curve,
    }
    let mut rng = gs_testkit::rng::SplitMix64::new(31);
    for round in 0..40 {
        let pts = random_points(&mut rng, 2 + round % 6, round % 2 == 0);
        let list = ok(Curve::new(pts.clone()));
        let kinds = ok(Curve::with_kinds(pts.clone(), vec![false; pts.len()]));
        // The same points written as a table with every point listed as sharp.
        let written = format!(
            "c = {{ points = {:?}, smooth = true, sharp = {:?} }}",
            pts,
            (0..pts.len()).collect::<Vec<_>>()
        );
        let table: Curve = ok(toml::from_str::<File>(&written)).c;
        for k in 0..=200 {
            let x = k as f32 / 200.0;
            // The plain formula: the segment holding x, interpolated in f32.
            let i = pts.partition_point(|p| p[0] <= x).clamp(1, pts.len() - 1);
            let ([x0, y0], [x1, y1]) = (pts[i - 1], pts[i]);
            let want = y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
            for (c, how) in [(&list, "list"), (&kinds, "kinds"), (&table, "table")] {
                assert_eq!(c.eval(x).to_bits(), want.to_bits(), "{how} at {x}");
            }
        }
    }
}

#[test]
fn smooth_curves_are_monotone_stay_in_range_and_match_the_slow_reference() {
    let mut rng = gs_testkit::rng::SplitMix64::new(32);
    for round in 0..60 {
        let rising = round % 3 != 0;
        let pts = random_points(&mut rng, 3 + round % 7, rising);
        let smooth: Vec<bool> = (0..pts.len()).map(|i| (i + round) % 4 != 0).collect();
        let c = ok(Curve::with_kinds(pts.clone(), smooth.clone()));
        let reference = gs_testkit::reference::stroke::Curve::new(
            pts.iter().map(|p| p.map(f64::from)).collect(),
            smooth.clone(),
        );
        let (lo, hi) = pts
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p[1]), h.max(p[1])));
        let mut last = f32::MIN;
        for k in 0..=1000 {
            let x = k as f32 / 1000.0;
            let y = c.eval(x);
            assert!(
                (lo - 1e-6..=hi + 1e-6).contains(&y),
                "round {round}: {y} at {x} outside {lo}..{hi}"
            );
            if rising {
                assert!(y >= last - 1e-6, "round {round}: falls at {x}");
            }
            last = y;
            let want = reference.at(f64::from(x));
            assert!(
                (f64::from(y) - want).abs() < 1e-5,
                "round {round}: {y} against {want} at {x}"
            );
        }
        // Through every point (to the table's resolution), and close to the exact cubic.
        for p in &pts {
            let exact = reference.cubic(f64::from(p[0]));
            assert!((exact - f64::from(p[1])).abs() < 1e-6, "through {p:?}");
        }
    }
}

#[test]
fn smoothing_at_an_open_edge_matches_the_slow_reference() {
    // A domed square facing up, its points nudged sideways so every mode has work to do;
    // the dab sits on the middle of one edge.
    let mut mesh = gs_testkit::fixtures::patch(12);
    for (i, p) in mesh.positions.iter_mut().enumerate() {
        let h = crate::common::mix(i as u64 + 5);
        p[0] += 0.03 * crate::common::unit(h);
        p[2] += 0.03 * crate::common::unit(crate::common::mix(h));
    }
    let edge = Shot {
        toward: Vec3::new(1.0, 0.0, 0.0),
        view: Some(Vec3::new(0.0, -1.0, 0.0)),
        invert: false,
    };
    for mode in ["uniform", "stronger", "tangent", "relax"] {
        let text = format!(
            "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.6\n[kernel]\ntype = \"smooth\"\nmode = \"{mode}\"\ngain = 5.0\n"
        );
        compare(mode, &text, &mesh, edge);
    }
}

#[test]
fn a_dab_seen_from_behind_falls_back_to_the_view_and_the_dab_point() {
    // Every vertex faces away, so nothing is sampled: the normal is the reverse of the view
    // and the plane passes through the dab point.
    let mesh = sphere(10, 0.5, 79, true);
    let behind = Shot {
        toward: Vec3::new(0.4, 0.5, 0.8),
        view: Some(Vec3::new(0.4, 0.5, 0.8).normalized()),
        invert: false,
    };
    let head = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.6\n[kernel]\n";
    for v in [
        "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]",
        "type = \"plane\"\ngain = 3.0\n[depth]\nimbed = 0.1",
    ] {
        compare(v, &format!("{head}{v}\n"), &mesh, behind);
    }
}

#[test]
fn one_dab_of_each_kernel_matches_the_slow_reference_beside_hidden_faces() {
    // The faces right of x = 0.55 hidden: the border runs through the dab, so the reference
    // checks hidden points, the visible-only frame and neighbours across visible faces only.
    let mut mesh = sphere(10, 0.5, 77, true);
    let hide = (mesh.faces.iter())
        .map(|f| {
            let c: Vec<f32> = (f.iter().filter(|&&v| v != gs_core::NO))
                .map(|&v| mesh.positions[v as usize][0])
                .collect();
            u32::from(c.iter().sum::<f32>() / c.len() as f32 > 0.55)
        })
        .collect();
    let column = gs_protocol::Attribute {
        domain: gs_protocol::AttrDomain::Face,
        values: gs_protocol::AttrValues::U32(hide),
    };
    mesh.attributes.insert("hide".into(), column);
    for s in crate::brush_kit::shipped() {
        if s.preset.claims.iter().any(|c| c == "reference") {
            both_ways(&s.stem, &s.text, &mesh);
        }
    }
}
