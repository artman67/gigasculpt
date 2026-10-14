//! Catalog `brush-settings.modifiers` and `brush-settings.alpha-and-texture`, their Check
//! clauses (decision 0058): the Brush Modifier pinches or bulges a flat plane, Constant Tilt
//! keeps the alpha turned alike from a curved stroke's start to its end, Align To Path 100
//! follows a spiral with no lag, Alpha Tile 4 repeats the picture 16 times, and Adaptive Map
//! Size 0 reads the full picture where more reads a reduced level. Also Tilt Brush's lean, the
//! settings' ranges, and that a preset leaving them at their defaults keeps its hash. Buildup's
//! clause is `stroke_pipeline`'s.

use std::sync::Arc;

use gs_core::alpha::{Picture, Pyramid};
use gs_core::brush::Patch;
use gs_core::stroke::placed;
use gs_core::symmetry::Symmetry;
use gs_core::{BrushPreset, StrokeParams, StrokeSample, Vec3};
use gs_protocol::PlainMesh;

use crate::brush_kit::{after, at, grid, ico, params, preset, run, shipped, store};
use crate::common::{mix, ok};

const HEAD: &str = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.5\n[kernel]\n\
                    type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n";

/// A constant-tip push along the area normal with `tip` lines, stamping `picture` (with its
/// `alpha` lines) when given, its dabs `spacing` diameters apart and its press laying nothing.
fn pusher(
    tip: &str,
    picture: Option<(&Picture, &str)>,
    spacing: f32,
) -> (BrushPreset, StrokeParams) {
    let alpha = picture.map_or(String::new(), |(p, lines)| {
        format!("[tip.alpha]\nimage = \"blake3:{}\"\n{lines}\n", p.hash())
    });
    let text = format!(
        "{HEAD}[tip]\nprofile = \"constant\"\n{tip}\n{alpha}[stroke]\nclass = \"dab\"\n\
         spacing = {spacing}\ntap = 0.0\n"
    );
    let p = ok(BrushPreset::from_toml(&text));
    let mut prm = params(None);
    prm.alpha = picture.and_then(|(picture, _)| {
        let spec = p.tip.alpha.as_ref()?;
        Some(Arc::new(Pyramid::build(picture, spec)))
    });
    (p, prm)
}

/// Black to the left, white to the right: a print leans to the side its first axis points.
/// Stamped with `mid = 0.5`, so the print carves as much as it raises and the round edge of
/// its footprint on the grid weighs nothing in where it leans.
fn ramp() -> Picture {
    ok(Picture::new(2, 2, vec![0, 65535, 0, 65535]))
}

/// `mesh` in the store's own vertex order, which a stroke's result keeps.
fn flat(mesh: &PlainMesh) -> PlainMesh {
    store(mesh).to_plain()
}

/// The angle of `(x, y)`, in degrees.
fn degrees(x: f32, y: f32) -> f32 {
    libm::atan2f(y, x).to_degrees()
}

/// `a - b` in degrees, from -180 to 180.
fn turn(a: f32, b: f32) -> f32 {
    (a - b + 540.0).rem_euclid(360.0) - 180.0
}

/// Where the print within `r` of `c` on the flat `before` leans, in degrees: the direction of
/// its rise-weighted first moment about `c`.
fn lean(before: &PlainMesh, after: &PlainMesh, c: Vec3, r: f32) -> f32 {
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for (a, b) in after.positions.iter().zip(&before.positions) {
        let (dx, dy) = (b[0] - c.x, b[1] - c.y);
        if dx * dx + dy * dy <= r * r {
            let rise = f64::from(a[2] - b[2]);
            x += rise * f64::from(dx);
            y += rise * f64::from(dy);
        }
    }
    degrees(x as f32, y as f32)
}

/// The prints a stroke of `p` along `s` leaves on the flat `mesh` that no other print overlaps,
/// past the press (which lays nothing) and the first dab, laid on the press with no step, in
/// stroke order: where each leans and the direction of its dab's step from the dab before, in
/// degrees.
fn prints(
    mesh: &PlainMesh,
    p: &BrushPreset,
    prm: StrokeParams,
    s: &[StrokeSample],
) -> Vec<(f32, f32)> {
    let (base, out) = (flat(mesh), after(mesh, p, prm, s));
    let dabs = placed(p, s);
    let r = s[0].radius;
    let alone = |i: usize| {
        (1..dabs.len()).all(|j| j == i || (dabs[j].0.pos - dabs[i].0.pos).len2() >= 4.0 * r * r)
    };
    (2..dabs.len())
        .filter(|&i| alone(i))
        .map(|i| {
            let c = dabs[i].0.pos;
            let step = c - dabs[i - 1].0.pos;
            (lean(&base, &out, c, r), degrees(step.x, step.y))
        })
        .collect()
}

/// `n + 1` samples of radius `r` on the plane z = 0 at `f(t)`, `t` from 0 to 1.
fn path(n: usize, r: f32, f: impl Fn(f32) -> [f32; 2]) -> Vec<StrokeSample> {
    (0..=n)
        .map(|i| {
            let [x, y] = f(i as f32 / n as f32);
            at(Vec3::new(x, y, 0.0), r)
        })
        .collect()
}

#[test]
fn the_brush_modifier_pinches_or_bulges_a_flat_plane() {
    let mesh = grid(60);
    let base = flat(&mesh);
    let line = path(10, 0.25, |t| [t - 0.5, 0.0]);
    let draw = preset("draw").text;
    let drawn = |m: f32| {
        let p = ok(BrushPreset::from_toml(&format!("modifier = {m}\n{draw}")));
        after(&mesh, &p, params(None), &line)
    };
    // How far the surface moved away from the stroke's line, over the whole plane, and its
    // highest rise.
    let spread = |m: &PlainMesh| {
        let pairs = m.positions.iter().zip(&base.positions);
        let out: f32 = (pairs.clone())
            .map(|(a, b)| (a[1] - b[1]) * b[1].signum())
            .sum();
        let top = pairs.map(|(a, b)| a[2] - b[2]).fold(0.0, f32::max);
        (out, top)
    };
    let (plain, pinch, bulge) = (
        spread(&drawn(0.0)),
        spread(&drawn(50.0)),
        spread(&drawn(-50.0)),
    );
    assert!(pinch.0 < -1e-3, "+50 draws the plane in: {pinch:?}");
    assert!(
        bulge.0 > 1e-3 && bulge.1 > plain.1,
        "-50 swells it out and up: {bulge:?} against plain Draw's {plain:?}"
    );
}

#[test]
fn tilt_brush_leans_the_push_toward_the_stroke_on_every_copy() {
    // One dab at x = 0.5 stepping along +x: its push leans toward +x by the tilt. A Mirror copy
    // steps along -x and leans that way; a Parallel copy steps along +x as the dab does.
    let mesh = grid(40);
    let base = flat(&mesh);
    let s = [
        at(Vec3::new(0.3, 0.0, 0.0), 0.2),
        at(Vec3::new(0.7, 0.0, 0.0), 0.2),
    ];
    let mirror = Symmetry {
        mirror: [true, false, false],
        ..Symmetry::default()
    };
    let parallel = Symmetry {
        parallel: [true, false, false],
        ..Symmetry::default()
    };
    for (tilt, sym, left) in [
        (45.0f32, Symmetry::default(), 0.0),
        (-30.0, Symmetry::default(), 0.0),
        (30.0, mirror, -1.0),
        (30.0, parallel, 1.0),
    ] {
        let (p, mut prm) = pusher(&format!("tilt = {tilt}"), None, 4.0);
        prm.symmetry = sym;
        let out = after(&mesh, &p, prm, &s);
        let tan = libm::tanf(tilt.to_radians());
        let mut moved = [0, 0];
        for (a, b) in out.positions.iter().zip(&base.positions) {
            let d: [f32; 3] = std::array::from_fn(|k| a[k] - b[k]);
            if d.iter().all(|&x| x == 0.0) {
                continue;
            }
            let side = if b[0] < 0.0 { left } else { 1.0 };
            assert!(
                d[1] == 0.0 && (d[0] - side * tan * d[2]).abs() <= 1e-6 + 1e-4 * d[2].abs(),
                "tilt {tilt} at {b:?} moved {d:?}"
            );
            moved[usize::from(b[0] < 0.0)] += 1;
        }
        assert!(moved[0] > 10, "tilt {tilt}: the dab moved {moved:?}");
        assert_eq!(
            moved[1] > 0,
            left != 0.0,
            "tilt {tilt}: the copy moved {moved:?}"
        );
    }
}

#[test]
fn constant_tilt_keeps_the_alpha_turned_alike_from_start_to_end() {
    // A half circle of separate prints: with Constant Tilt the first and the last lean alike;
    // without it the alpha turns as the stroke does, most of a half turn.
    let mesh = grid(480);
    let half = path(60, 0.08, |t| {
        let (s, c) = libm::sincosf(std::f32::consts::PI * t);
        [0.7 * c, 0.7 * s]
    });
    let ramp = ramp();
    let ends = |tip: &str| {
        let (p, prm) = pusher(tip, Some((&ramp, "mid = 0.5")), 1.5);
        let all = prints(&mesh, &p, prm, &half);
        assert!(all.len() >= 6, "{tip}: {all:?}");
        (all[0], all[all.len() - 1])
    };
    let ((start, _), (end, _)) = ends("constant_tilt = true");
    assert!(
        turn(end, start).abs() <= 1.0,
        "Constant Tilt: {start} at the start, {end} at the end"
    );
    let ((start, from), (end, to)) = ends("");
    let (leaned, stepped) = (turn(end, start), turn(to, from));
    assert!(
        stepped.abs() >= 90.0 && turn(leaned, stepped).abs() <= 1.0,
        "without it the alpha turns {leaned} as the stroke turns {stepped}"
    );
}

#[test]
fn align_to_path_100_follows_a_spiral_with_no_lag() {
    // Each separate print along a spiral turning ever tighter, against its dab's step: at 100
    // the alpha leans along the step; at 0 it lags behind the turn after the first.
    let mesh = grid(480);
    let spiral = path(120, 0.07, |t| {
        let a = 1.5 * std::f32::consts::PI * t;
        let (s, c) = libm::sincosf(a);
        let rho = 0.85 - 0.1 * a;
        [rho * c, rho * s]
    });
    let ramp = ramp();
    let lags = |align: f32| {
        let (p, prm) = pusher(&format!("align = {align}"), Some((&ramp, "mid = 0.5")), 1.5);
        let all = prints(&mesh, &p, prm, &spiral);
        assert!(all.len() >= 8, "{align}: {all:?}");
        all.iter()
            .map(|&(lean, step)| turn(step, lean))
            .collect::<Vec<f32>>()
    };
    let strict = lags(100.0);
    assert!(
        strict.iter().all(|l| l.abs() <= 1.0),
        "Align To Path 100 lags {strict:?}"
    );
    // Against the same print at 100, which cancels what the grid adds to the measure.
    let behind: Vec<f32> = (lags(0.0).iter().zip(&strict))
        .map(|(a, b)| a - b)
        .collect();
    assert!(
        behind[0].abs() <= 1.0 && behind[1..].iter().all(|&l| l >= 2.0),
        "Align To Path 0 lags behind the turn after the first print, past the same print at \
         100: {behind:?}"
    );
}

#[test]
fn alpha_tile_4_repeats_the_picture_16_times() {
    // A white square in a black border: tiled 4 by 4 under one dab, black lines part the 16
    // copies (the round tip clips the corner ones but reaches each).
    let n = 80;
    let mesh = grid(n);
    let framed = (0..64)
        .map(|i| {
            let (x, y) = (i % 8, i / 8);
            if [x, y].iter().any(|&k| k == 0 || k == 7) {
                0
            } else {
                65535
            }
        })
        .collect();
    let framed = ok(Picture::new(8, 8, framed));
    let (p, prm) = pusher("", Some((&framed, "tiles = [4, 4]")), 4.0);
    let one_dab = [
        at(Vec3::new(-0.2, 0.0, 0.0), 0.6),
        at(Vec3::new(0.2, 0.0, 0.0), 0.6),
    ];
    let (base, out) = (flat(&mesh), after(&mesh, &p, prm, &one_dab));
    // Whether each vertex moved, by its place on the grid.
    let side = n as usize + 1;
    let place = |x: f32| ((x + 1.0) * n as f32 / 2.0).round() as usize;
    let mut moved = vec![false; side * side];
    for (a, b) in out.positions.iter().zip(&base.positions) {
        moved[place(b[0]) * side + place(b[1])] |= a.map(f32::to_bits) != b.map(f32::to_bits);
    }
    // Groups of moved vertices joined along grid lines, found by flooding from each.
    let mut seen = vec![false; moved.len()];
    let mut groups = 0;
    for start in 0..moved.len() {
        if !moved[start] || seen[start] {
            continue;
        }
        groups += 1;
        let mut todo = vec![start];
        seen[start] = true;
        while let Some(v) = todo.pop() {
            let (i, j) = (v / side, v % side);
            let near = [
                (i > 0).then(|| v - side),
                (i + 1 < side).then(|| v + side),
                (j > 0).then(|| v - 1),
                (j + 1 < side).then(|| v + 1),
            ];
            for u in near.into_iter().flatten() {
                if moved[u] && !seen[u] {
                    seen[u] = true;
                    todo.push(u);
                }
            }
        }
    }
    assert_eq!(groups, 16, "the dab printed 16 copies");
}

/// A picture of noise, `size` by `size`.
fn noise(size: u32) -> Picture {
    let pixels = (0..u64::from(size * size))
        .map(|i| (mix(i) >> 48) as u16)
        .collect();
    ok(Picture::new(size, size, pixels))
}

#[test]
fn adaptive_map_size_0_reads_the_full_picture_and_above_0_a_reduced_level() {
    // A small dab over a coarse sphere: by its vertex spacing it reads a level a few texels
    // across.
    let big = noise(512);
    let level = |lines: &str| {
        let (p, prm) = pusher("", Some((&big, lines)), 0.1);
        let mut m = store(&ico(4));
        let report = run(&mut m, &p, prm, &[at(Vec3::new(0.0, 0.0, 1.0), 0.15)]);
        report.alpha_finest
    };
    assert_eq!(level("adaptive = 0"), Some(0), "0 reads the full picture");
    let (half, full) = (level("adaptive = 0.5"), level(""));
    assert!(
        half > Some(0) && half < full,
        "0.5 reads a reduced level finer than 1's: {half:?} against {full:?}"
    );
}

/// Why `patch` on the shipped preset `stem` is refused.
fn refused(stem: &str, patch: &str) -> String {
    let patch = ok(toml::from_str::<Patch>(patch));
    match BrushPreset::resolve(&preset(stem).text, &patch) {
        Ok(_) => panic!("{stem} took {patch:?}"),
        Err(e) => e.message,
    }
}

#[test]
fn tilt_align_and_map_size_are_checked_and_their_defaults_keep_every_hash() {
    for (patch, key) in [
        ("[tip]\ntilt = 91", "tip.tilt must be from -90 to 90"),
        ("[tip]\ntilt = -90.5", "tip.tilt must be from -90 to 90"),
        ("[tip]\ntilt = nan", "tip.tilt"),
        ("[tip]\nalign = 101", "tip.align must be from 0 to 100"),
        ("[tip]\nalign = -1", "tip.align must be from 0 to 100"),
        (
            "[tip.alpha]\nadaptive = 1.5",
            "tip.alpha.adaptive must be from 0 to 1",
        ),
        (
            "[tip.alpha]\nadaptive = -0.1",
            "tip.alpha.adaptive must be from 0 to 1",
        ),
    ] {
        let why = refused("clay-build", patch);
        assert!(why.contains(key), "{patch}: {why}");
    }
    for patch in [
        "[tip]\ntilt = 10",
        "[tip]\nconstant_tilt = true",
        "[tip]\nalign = 50",
    ] {
        let why = refused("clip-curve", patch);
        assert!(why.contains("gesture"), "{patch}: {why}");
    }
    // Every shipped preset leaves them out, and writing them at their defaults changes no hash.
    let tip = "[tip]\ntilt = 0\nconstant_tilt = false\nalign = 100\n";
    for s in shipped() {
        let text = s.preset.canonical();
        let new = ["tilt", "align", "adaptive"];
        assert!(new.iter().all(|k| !text.contains(k)), "{}: {text}", s.stem);
        let alpha = s.preset.tip.alpha.is_some();
        let patch = format!(
            "{tip}{}",
            if alpha {
                "[tip.alpha]\nadaptive = 1\n"
            } else {
                ""
            }
        );
        let same = ok(BrushPreset::resolve(
            &s.text,
            &ok(toml::from_str::<Patch>(&patch)),
        ));
        assert_eq!(same.hash(), s.preset.hash(), "{}", s.stem);
    }
    let set = "[tip]\ntilt = 20\nconstant_tilt = true\nalign = 40\n[tip.alpha]\nadaptive = 0.25";
    let set = ok(BrushPreset::resolve(
        &preset("clay-build").text,
        &ok(toml::from_str::<Patch>(set)),
    ));
    let text = set.canonical();
    let written = [
        "tilt = 20",
        "constant_tilt = true",
        "align = 40",
        "adaptive = 0.25",
    ];
    assert!(written.iter().all(|k| text.contains(k)), "{text}");
}
