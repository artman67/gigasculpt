//! Alphas (decision 0018): pictures are named by their pixels, the pyramid and its sampler match
//! a slow box-filtered reference, tone settings follow their formulas, everything is the same
//! at any thread count, the starter recipes pass the catalog's checks, and through a stroke an
//! alpha is a factor on the influence with the sign rule of its kernel.

use std::sync::Arc;

use gs_core::alpha::{Picture, Pyramid, Recipe};
use gs_core::brush::{AlphaSpec, Flip, Patch};
use gs_core::curve::Curve;
use gs_core::{BrushPreset, StrokeParams, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::reference::alpha as reference;
use gs_testkit::rng::SplitMix64;

use crate::brush_kit::{after, at, grid, ico, params, preset, recipe_picture, run, shipped, store};
use crate::common::ok;

/// A random picture `w` by `h`.
fn random(w: u32, h: u32, seed: u64) -> Picture {
    let mut rng = SplitMix64::new(seed);
    let pixels = (0..w * h).map(|_| (rng.next_u64() >> 48) as u16).collect();
    ok(Picture::new(w, h, pixels))
}

/// Level 0 of `spec` applied to `p`, as values.
fn processed(p: &Picture, spec: &AlphaSpec) -> (Pyramid, Vec<u16>) {
    let pyr = Pyramid::build(p, spec);
    let [w, h] = pyr.size(0);
    let values = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| pyr.texel(0, x, y))
        .collect();
    (pyr, values)
}

fn spec(edit: impl FnOnce(&mut AlphaSpec)) -> AlphaSpec {
    let mut s = AlphaSpec::plain("x");
    edit(&mut s);
    s
}

#[test]
fn pictures_are_checked_and_named_by_their_pixels() {
    assert!(Picture::new(1, 2, vec![0; 2]).is_err(), "too narrow");
    assert!(Picture::new(2, 1, vec![0; 2]).is_err(), "too short");
    assert!(Picture::new(2, 2, vec![0; 3]).is_err(), "wrong count");
    assert!(Picture::new(8193, 2, vec![0; 16386]).is_err(), "too wide");
    let a = ok(Picture::new(3, 2, vec![0, 1, 2, 3, 4, 5]));
    let same = ok(Picture::new(3, 2, vec![0, 1, 2, 3, 4, 5]));
    let changed = ok(Picture::new(3, 2, vec![0, 1, 2, 3, 4, 6]));
    let turned = ok(Picture::new(2, 3, vec![0, 1, 2, 3, 4, 5]));
    assert_eq!(a.hash(), same.hash(), "the same pixels, the same name");
    assert_ne!(a.hash(), changed.hash(), "one pixel changes the name");
    assert_ne!(a.hash(), turned.hash(), "so does the shape");
    assert!(
        a.hash().len() == 64 && a.hash().bytes().all(|b| b.is_ascii_hexdigit()),
        "64 hex digits"
    );
    let unit = ok(Picture::from_unit(
        3,
        2,
        &[0.0, 1.0, 0.5, -1.0, 2.0, f32::NAN],
    ));
    assert_eq!(
        unit.pixels(),
        [0, 65535, 32768, 0, 65535, 0],
        "rounded and held"
    );
}

#[test]
fn the_sampler_matches_a_slow_box_filtered_reference() {
    let p = random(37, 23, 3);
    let pyr = Pyramid::build(&p, &AlphaSpec::plain("x"));
    let want = reference::levels(37, 23, p.pixels());
    assert_eq!(pyr.levels(), want.len(), "down to 1 by 1");
    let mut rng = SplitMix64::new(4);
    let mut u = || 2.2 * ((rng.next_u64() >> 40) as f64 / 16_777_216.0) - 1.1;
    for (k, level) in want.iter().enumerate() {
        assert_eq!(
            pyr.size(k),
            [level.width as u32, level.height as u32],
            "level {k}"
        );
        // Each level rounds to 16 bits once more, at most half a step per level; the engine
        // places the point in f32, which moves it by up to about 1e-5 texels.
        let tolerance = (0.5 * k as f64 + 1.0) / 65535.0 + 1e-5;
        for tiles in [[1, 1], [3, 2]] {
            for _ in 0..300 {
                let (x, y) = (u(), u());
                let got = f64::from(pyr.sample(k, x as f32, y as f32, tiles));
                let exp = reference::sample(level, f64::from(x as f32), f64::from(y as f32), tiles);
                assert!(
                    (got - exp).abs() <= tolerance,
                    "level {k}, tiles {tiles:?}, at {x}, {y}: {got} against {exp}"
                );
            }
        }
    }
}

#[test]
fn tone_settings_follow_their_formulas() {
    let p = random(4, 3, 5);
    let px = |x: u32, y: u32| p.at(x, y);
    let (_, plain) = processed(&p, &AlphaSpec::plain("x"));
    assert_eq!(
        plain,
        p.pixels(),
        "neutral settings pass the picture through"
    );
    let (_, inv) = processed(&p, &spec(|s| s.invert = true));
    assert!(
        inv.iter().zip(p.pixels()).all(|(a, b)| *a == 65535 - b),
        "invert"
    );
    let (_, h) = processed(&p, &spec(|s| s.flip = Flip::Horizontal));
    let (_, v) = processed(&p, &spec(|s| s.flip = Flip::Vertical));
    assert_eq!((h[0], v[0]), (px(3, 0), px(0, 2)), "flips");
    let (turned, t1) = processed(&p, &spec(|s| s.turn = 1));
    assert_eq!(turned.size(0), [3, 4], "a quarter turn swaps the sides");
    // Clockwise: the bottom-left pixel comes to the top left, the top-left to the top right.
    assert_eq!((t1[0], t1[2]), (px(0, 2), px(0, 0)), "clockwise");
    let (_, t2) = processed(&p, &spec(|s| s.turn = 2));
    let (_, both) = processed(&p, &spec(|s| s.flip = Flip::Both));
    assert_eq!(t2, both, "a half turn is both flips");
    let low = ok(Picture::new(2, 2, vec![1000, 2000, 3000, 1500]));
    let (_, max) = processed(&low, &spec(|s| s.maximize = true));
    assert_eq!(max, [0, 32768, 65535, 16384], "stretched to the full range");
    let curve = ok(Curve::with_kinds(
        vec![[0.0, 0.0], [0.3, 0.6], [1.0, 1.0]],
        vec![false, true, false],
    ));
    let (_, contrast) = processed(&p, &spec(|s| s.contrast = 2.0));
    let (_, curved) = processed(&p, &spec(|s| s.curve = curve.clone()));
    for (i, &v) in p.pixels().iter().enumerate() {
        let x = f32::from(v) / 65535.0;
        let near = |got: u16, want: f32| (f32::from(got) - want * 65535.0).abs() <= 1.0;
        assert!(
            near(contrast[i], ((x - 0.5) * 2.0 + 0.5).clamp(0.0, 1.0)),
            "contrast"
        );
        assert!(near(curved[i], curve.eval(x)), "curve");
    }
    let white = ok(Picture::new(64, 64, vec![65535; 64 * 64]));
    let (_, faded) = processed(&white, &spec(|s| s.radial_fade = 1.0));
    let at = |x: usize, y: usize| f32::from(faded[y * 64 + x]) / 65535.0;
    let d = |x: usize, y: usize| {
        let c = |i: usize| (i as f32 + 0.5) / 32.0 - 1.0;
        (c(x) * c(x) + c(y) * c(y)).sqrt()
    };
    for (x, y) in [(31, 31), (40, 20), (63, 32)] {
        assert!(
            (at(x, y) - (1.0 - d(x, y))).abs() < 1e-4,
            "linear fade at {x}, {y}"
        );
    }
    for y in 0..64 {
        for x in 0..64 {
            assert!(
                d(x, y) < 1.0 || at(x, y) == 0.0,
                "0 from the rim out: {x}, {y}"
            );
        }
    }
}

#[test]
fn blur_spreads_a_line_like_a_gaussian() {
    // A bright column in a 129 by 9 picture; sigma 4 pixels: 4 * 256 / 129 of a 256-wide copy.
    // Three boxes of odd widths reach the Gaussian's variance as closely as odd widths can
    // (here 14.7 for 16) and its shape within a few thousandths.
    let mut pixels = vec![0u16; 129 * 9];
    for y in 0..9 {
        pixels[y * 129 + 64] = 65535;
    }
    let p = ok(Picture::new(129, 9, pixels));
    let (_, b) = processed(&p, &spec(|s| s.blur = 4.0 * 256.0 / 129.0));
    let row: Vec<f64> = (0..129)
        .map(|x| f64::from(b[4 * 129 + x]) / 65535.0)
        .collect();
    let mass: f64 = row.iter().sum();
    let variance: f64 = (row.iter().enumerate())
        .map(|(x, v)| v * (x as f64 - 64.0) * (x as f64 - 64.0))
        .sum::<f64>()
        / mass;
    assert!((mass - 1.0).abs() < 2e-3, "keeps the total: {mass}");
    assert!(
        (variance - 16.0).abs() < 1.6,
        "sigma 4: variance {variance}"
    );
    for x in 0..129 {
        let dx = x as f64 - 64.0;
        let g = libm::exp(-dx * dx / 32.0) / (4.0 * std::f64::consts::TAU.sqrt());
        assert!((row[x] - g).abs() < 8e-3, "at {x}: {} against {g}", row[x]);
        assert_eq!(b[4 * 129 + x], b[4 * 129 + 128 - x], "symmetric");
    }
}

#[test]
fn noise_stays_in_range_and_tiles() {
    let grey = ok(Picture::new(96, 96, vec![32768; 96 * 96]));
    let (_, n) = processed(&grey, &spec(|s| (s.noise, s.noise_scale) = (0.4, 6)));
    let v: Vec<f32> = n.iter().map(|&x| f32::from(x) / 65535.0).collect();
    assert!(
        v.iter().all(|x| (0.1..=0.9).contains(x)),
        "within the amount"
    );
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    assert!((mean - 0.5).abs() < 0.08, "around the grey: {mean}");
    let step = |a: usize, b: usize| (v[a] - v[b]).abs();
    let inner = (0..96)
        .flat_map(|y| (0..95).map(move |x| (y * 96 + x, y * 96 + x + 1)))
        .map(|(a, b)| step(a, b))
        .fold(0.0, f32::max);
    let seam = (0..96)
        .map(|y| step(y * 96, y * 96 + 95))
        .fold(0.0, f32::max);
    assert!(inner > 0.0, "not flat");
    assert!(
        seam <= 2.0 * inner,
        "no seam where it repeats: {seam} against {inner}"
    );
}

#[test]
fn pyramids_and_recipes_are_the_same_at_any_thread_count() {
    let p = random(300, 200, 6);
    let full = spec(|s| {
        s.flip = Flip::Vertical;
        s.turn = 1;
        s.invert = true;
        s.maximize = true;
        s.blur = 3.0;
        s.noise = 0.2;
        s.noise_seed = 9;
        s.contrast = 1.5;
        s.curve = Curve::linear();
        s.radial_fade = 0.5;
    });
    let one = gs_par::with_threads(1, || Pyramid::build(&p, &full));
    let many = gs_par::with_threads(24, || Pyramid::build(&p, &full));
    assert_eq!(one, many, "the same pyramid on 1 and 24 threads");
    assert_eq!(one, Pyramid::build(&p, &full), "and every time");
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/alphas/grainy-round.toml"),
    );
    let recipe = ok(Recipe::from_toml(&ok(text)));
    let a = gs_par::with_threads(1, || ok(recipe.draw()));
    let b = gs_par::with_threads(24, || ok(recipe.draw()));
    assert_eq!(a.hash(), b.hash(), "the same picture on 1 and 24 threads");
}

/// The value of a 1024-wide recipe picture at `(x, y)`, each -1 to 1 (y down).
fn value(p: &Picture, x: f32, y: f32) -> f32 {
    let n = p.width() as f32;
    let i = ((f32::midpoint(x, 1.0) * n) as u32).min(p.width() - 1);
    let j = ((f32::midpoint(y, 1.0) * n) as u32).min(p.height() - 1);
    f32::from(p.at(i, j)) / 65535.0
}

#[test]
fn the_starter_alphas_pass_the_catalog_checks() {
    // alpha.starter-tips: soft round is 1 at the center, 0 at the rim, falling all the way;
    // white is 1 everywhere; the ring peaks at 0.75 radius.
    let soft = recipe_picture("soft-round");
    assert!(value(&soft, 0.0, 0.0) > 0.999, "1 at the center");
    let along: Vec<f32> = (0..512)
        .map(|i| value(&soft, i as f32 / 512.0, 0.0))
        .collect();
    assert!(
        along.windows(2).all(|w| w[1] <= w[0]),
        "falls monotonically"
    );
    assert!(value(&soft, 0.999, 0.0) < 1e-3, "0 at the rim");
    let white = recipe_picture("white");
    assert!(
        white.pixels().iter().all(|&v| v == 65535),
        "white everywhere"
    );
    let ring = recipe_picture("ring");
    let peak = (0..512)
        .max_by_key(|&i| (value(&ring, i as f32 / 512.0, 0.0) * 1e6) as u64)
        .map(|i| i as f32 / 512.0);
    assert!(
        peak.is_some_and(|d| (d - 0.75).abs() < 0.01),
        "ring peak at {peak:?}"
    );
    assert!(value(&ring, 0.0, 0.0) == 0.0, "a hole in the middle");
    let square = recipe_picture("square");
    assert!(
        value(&square, 0.6, -0.6) > 0.999,
        "flat top out to its corners"
    );
    assert!(value(&square, 0.75, 0.0) == 0.0, "a sharp edge at 0.7");
    let hard = recipe_picture("hard-round");
    let hard_edge = value(&hard, 0.85, 0.0) > 0.999 && value(&hard, 0.999, 0.0) < 0.02;
    assert!(
        hard_edge && value(&hard, 1.0, 1.0) == 0.0,
        "flat top, short edge"
    );
    let grainy = recipe_picture("grainy-round");
    let grain = (soft.pixels().iter().zip(grainy.pixels()))
        .filter(|(s, g)| s != g)
        .count();
    assert!(grain > 1000, "grain");
    assert!(
        (soft.pixels().iter().zip(grainy.pixels())).all(|(s, g)| g <= s),
        "the grain only darkens the soft round"
    );
}

const HEAD: &str = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.5\n";

/// A constant-tip push along the area normal with `alpha` settings for `picture`.
fn pusher(picture: &Picture, alpha: &str, stroke: &str) -> (BrushPreset, StrokeParams) {
    let text = format!(
        "{HEAD}[kernel]\ntype = \"offset\"\nterms = [{{ dir = \"area_normal\", gain = 0.5 }}]\n\
         [tip]\nprofile = \"constant\"\n[tip.alpha]\nimage = \"blake3:{}\"\n{alpha}\n{stroke}\n",
        picture.hash()
    );
    let p = ok(BrushPreset::from_toml(&text));
    let mut prm = params(None);
    prm.alpha = p
        .tip
        .alpha
        .as_ref()
        .map(|a| Arc::new(Pyramid::build(picture, a)));
    (p, prm)
}

/// A position's bits, with -0 read as 0, to find a vertex at an exact place.
fn key(p: [f32; 3]) -> [u32; 3] {
    p.map(|c| (c + 0.0).to_bits())
}

fn bits(m: &PlainMesh) -> Vec<[u32; 3]> {
    m.positions.iter().map(|p| p.map(f32::to_bits)).collect()
}

#[test]
fn white_is_no_alpha_and_black_at_mid_one_is_alt_on_a_signed_kernel() {
    let mesh = ico(4);
    let path = crate::brush_kit::arc(Vec3::new(0.3, 0.1, 1.0), Vec3::new(-0.2, 0.3, 1.0), 6, 0.3);
    let white = ok(toml::from_str::<Patch>("[tip.alpha]\nimage = \"white\""));
    for s in shipped() {
        let mut bare = s.preset.clone();
        bare.tip.alpha = None;
        let stamped = ok(BrushPreset::resolve(&s.text, &white));
        let samples = crate::brush_kit::samples(&s.preset, 0.3);
        assert_eq!(
            bits(&after(&mesh, &stamped, params(None), &samples)),
            bits(&after(&mesh, &bare, params(None), &samples)),
            "{}: white gives the bits of no alpha",
            s.stem
        );
    }
    let black = ok(Picture::new(2, 2, vec![0; 4]));
    let draw = preset("draw").text;
    let patch = format!("[tip.alpha]\nimage = \"blake3:{}\"\nmid = 1", black.hash());
    let carve = ok(BrushPreset::resolve(
        &draw,
        &ok(toml::from_str::<Patch>(&patch)),
    ));
    let mut prm = params(None);
    prm.alpha = carve
        .tip
        .alpha
        .as_ref()
        .map(|a| Arc::new(Pyramid::build(&black, a)));
    let mut alt = params(None);
    alt.invert = true;
    let plain = ok(BrushPreset::from_toml(&draw));
    assert_eq!(
        bits(&after(&mesh, &carve, prm, &path)),
        bits(&after(&mesh, &plain, alt, &path)),
        "black below a mid value of 1 carves exactly as Alt does"
    );
}

#[test]
fn a_mid_value_of_half_on_a_checker_raises_and_carves_equally() {
    // alpha.modify-tone's check, on a flat grid with one constant dab at its center.
    let mesh = store(&grid(40)).to_plain();
    let checker = ok(Picture::new(2, 2, vec![0, 65535, 65535, 0]));
    let (p, prm) = pusher(&checker, "mid = 0.5", "");
    let out = after(&mesh, &p, prm, &[at(Vec3::ZERO, 0.6)]);
    let rise = |i: usize| out.positions[i][2] - mesh.positions[i][2];
    let (mut up, mut down) = (0.0f32, 0.0f32);
    for (i, q) in mesh.positions.iter().enumerate() {
        up = up.max(rise(i));
        down = down.min(rise(i));
        // Mirroring across x = 0 swaps black and white: equal and opposite.
        let j = mesh
            .positions
            .iter()
            .position(|r| key(*r) == key([-q[0], q[1], q[2]]));
        if let Some(j) = j {
            assert!((rise(i) + rise(j)).abs() <= 1e-6, "at {q:?}");
        }
    }
    assert!(
        up > 1e-3 && (up + down).abs() <= 1e-6,
        "raises {up}, carves {down}"
    );
}

#[test]
fn turning_the_stroke_turns_the_print() {
    // One dab at the center of a square grid, made by a stroke along +x and one along +y: the
    // second print is the first turned a quarter about +z.
    let mesh = store(&grid(40)).to_plain();
    let picture = random(8, 8, 7);
    let one_dab = "[stroke]\nclass = \"dab\"\nspacing = 4.0\ntap = 0.0";
    let (p, prm) = pusher(&picture, "", one_dab);
    let along_x = after(
        &mesh,
        &p,
        prm.clone(),
        &[
            at(Vec3::new(-0.2, 0.0, 0.0), 0.6),
            at(Vec3::new(0.2, 0.0, 0.0), 0.6),
        ],
    );
    let along_y = after(
        &mesh,
        &p,
        prm,
        &[
            at(Vec3::new(0.0, -0.2, 0.0), 0.6),
            at(Vec3::new(0.0, 0.2, 0.0), 0.6),
        ],
    );
    let mut moved = 0;
    for (i, q) in mesh.positions.iter().enumerate() {
        let turned = [-q[1], q[0], q[2]];
        let j = mesh.positions.iter().position(|r| key(*r) == key(turned));
        let Some(j) = j else { continue };
        let (a, b) = (along_x.positions[i][2], along_y.positions[j][2]);
        assert!((a - b).abs() <= 1e-6, "{q:?}: {a} against {b}");
        moved += usize::from(a != 0.0);
    }
    assert!(moved > 100, "the dab printed: {moved} vertices");
    let still = |m: &PlainMesh| m.positions == mesh.positions;
    assert!(!still(&along_x), "and something moved");
}

#[test]
fn a_dab_reads_the_level_its_vertex_spacing_needs() {
    let big = random(4096, 4096, 8);
    let (p, prm) = pusher(&big, "", "");
    let mut small = store(&ico(4));
    let rep = run(
        &mut small,
        &p,
        prm.clone(),
        &[at(Vec3::new(0.0, 0.0, 1.0), 0.15)],
    );
    // A few vertices under the dab: a level of at most 16 texels across, never the picture.
    let small_level = rep.alpha_finest.unwrap_or(0);
    assert!(
        small_level >= 8,
        "a small dab never reads the 4096 picture itself: level {small_level}"
    );
    let mut dense = store(&ico(6));
    let rep = run(&mut dense, &p, prm, &[at(Vec3::new(0.0, 0.0, 1.0), 0.5)]);
    let dense_level = rep.alpha_finest.unwrap_or(usize::MAX);
    assert!(
        dense_level < small_level && dense_level > 0,
        "a wide dab over a dense mesh reads finer: {dense_level} against {small_level}"
    );
    // Its texels are no finer than half the vertex spacing (about 0.017 on this mesh).
    let pyr = Pyramid::build(&big, &AlphaSpec::plain("x"));
    let spacing = 2.0 * 0.5 / pyr.size(dense_level)[0] as f32;
    assert!(spacing >= 0.5 * 0.012, "texel spacing {spacing}");
}

#[test]
fn strength_per_stroke_length_holds_with_an_alpha() {
    // A grainy, tiled alpha following a stroke: halving the spacing changes the result by no
    // more than the claims allow without one (5%).
    let mesh = ico(4);
    let grainy = recipe_picture("grainy-round");
    let smp = crate::brush_kit::arc(Vec3::new(0.3, 0.1, 1.0), Vec3::new(-0.3, 0.2, 1.0), 6, 0.25);
    let run_at = |spacing: f32| {
        let stroke = format!("[stroke]\nclass = \"dab\"\nspacing = {spacing}");
        let (p, prm) = pusher(&grainy, "tiles = [2, 2]", &stroke);
        after(&mesh, &p, prm, &smp)
    };
    let (a, b) = (run_at(0.1), run_at(0.05));
    let size = crate::brush_kit::max_diff(&a, &mesh);
    let off = crate::brush_kit::max_diff(&a, &b);
    assert!(size > 1e-3 && off <= 0.05 * size, "{off} of {size}");
}
