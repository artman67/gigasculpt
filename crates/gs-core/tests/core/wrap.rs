//! Catalog `brush-settings.curve`, its Wrap Mode clause (decision 0054): "Wrap Mode 3 on a plane
//! produces a 3 by 3 repeat of the dab pattern whose borders match", for a stroke inside a tile
//! and one hanging over the plane's corner, and for drags and stamps; Wrap Mode 1 is no Wrap
//! Mode at all; and every brush or stroke that cannot wrap is refused with its reason.

use gs_core::live::{self, LiveDetail, NotRefined, Target};
use gs_core::{BrushPreset, Stroke, StrokeSample, Vec3, stroke};
use gs_testkit::compare::canonical;

use crate::brush_kit::{at, grid, mirror_x, params, preset, run, shipped, store};
use crate::common::ok;

/// Quads along each side of the test plane (2 by 2 at z = 0): 32 to a tile at Wrap Mode 3.
const N: usize = 96;
/// Grid steps along one tile's side at Wrap Mode 3.
const TILE: usize = 32;

/// How far a vertex's move may stray from its image's one tile over. The two are 2/3 apart,
/// and a vertex's place, its image's, and the copy's center are each rounded to f32 (about
/// 6e-8 at these sizes), so the two see the dab from points up to about 2e-7 apart. The smooth
/// falloff's slope at a radius of 0.15 is under 15 per unit, so each dab's weight differs by
/// under 3e-6 of itself, and the moves, summed over the stroke, by under 3e-6 of the ridge's
/// height (0.1 at most here): 3e-7. Measured: 1.8e-7 at most (2026-10-12). This is 30 times
/// the estimate, and a ten-thousandth of the ridge.
const CLOSE: f32 = 1e-5;

/// The shipped preset `stem`'s text with `wrap = {n}` in its `[tip]` table.
fn wrapped_text(stem: &str, n: i64) -> String {
    let text = preset(stem).text;
    if text.contains("[tip]\n") {
        text.replacen("[tip]\n", &format!("[tip]\nwrap = {n}\n"), 1)
    } else {
        format!("{text}\n[tip]\nwrap = {n}\n")
    }
}

fn wrapped(stem: &str, n: i64) -> BrushPreset {
    ok(BrushPreset::from_toml(&wrapped_text(stem, n)))
}

/// Vertex `(i, j)` of the plane: at x = -1 + 2i/N, y = -1 + 2j/N.
fn idx(i: usize, j: usize) -> usize {
    i * (N + 1) + j
}

/// Each vertex's move when `p` strokes `samples` on the plane, by grid place (the store keeps
/// its own vertex order, so each is placed by where it started).
fn moves(p: &BrushPreset, samples: &[StrokeSample]) -> Vec<[f32; 3]> {
    let mut s = store(&grid(N as u32));
    let flat = s.to_plain();
    run(&mut s, p, params(None), samples);
    let out = s.to_plain();
    let place = |x: f32| ((x + 1.0) * N as f32 / 2.0).round() as usize;
    let mut d = vec![[0.0; 3]; (N + 1) * (N + 1)];
    for (a, b) in flat.positions.iter().zip(&out.positions) {
        d[idx(place(a[0]), place(a[1]))] = std::array::from_fn(|k| b[k] - a[k]);
    }
    d
}

fn gap(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).abs()).fold(0.0, f32::max)
}

/// The most any vertex's move strays from its image's one tile over along x or y, and from
/// the one on the opposite border.
fn worst_repeat(d: &[[f32; 3]]) -> f32 {
    let mut worst = 0.0f32;
    for i in 0..=N {
        for j in 0..=N {
            let mut pair = |k: usize| worst = worst.max(gap(d[idx(i, j)], d[k]));
            if i + TILE <= N {
                pair(idx(i + TILE, j));
            }
            if j + TILE <= N {
                pair(idx(i, j + TILE));
            }
            if i == 0 {
                pair(idx(N, j));
            }
            if j == 0 {
                pair(idx(i, N));
            }
        }
    }
    worst
}

/// `n + 1` full-pressure samples of radius `r` from `a` to `b`.
fn line(a: Vec3, b: Vec3, n: usize, r: f32) -> Vec<StrokeSample> {
    (0..=n)
        .map(|k| at(a + (b - a) * (k as f32 / n as f32), r))
        .collect()
}

/// The Check: a Draw stroke inside the middle tile, at Wrap Mode 3, is the same stroke without
/// Wrap Mode repeated in all nine tiles, and the borders match.
#[test]
fn wrap_mode_3_repeats_a_stroke_inside_a_tile_3_by_3_with_matching_borders() {
    // Reaching 0.27 from the middle at most, short of the middle tile's edges at 1/3.
    let inside = line(
        Vec3::new(-0.12, -0.06, 0.0),
        Vec3::new(0.12, 0.08, 0.0),
        8,
        0.15,
    );
    let plain = moves(&preset("draw").preset, &inside);
    let height = plain.iter().map(|m| m[2]).fold(0.0, f32::max);
    assert!(height > 0.01, "the stroke raises a ridge: {height}");
    let d = moves(&wrapped("draw", 3), &inside);
    let mut worst = 0.0f32;
    for i in 0..=N {
        for j in 0..=N {
            let home = idx(TILE + i % TILE, TILE + j % TILE);
            worst = worst.max(gap(d[idx(i, j)], plain[home]));
        }
    }
    assert!(
        worst <= CLOSE,
        "each tile holds the stroke's pattern: {worst}"
    );
    let repeat = worst_repeat(&d);
    assert!(
        repeat <= CLOSE,
        "tile for tile and border to border: {repeat}"
    );
}

/// A stroke hanging over the plane's corner comes back in at the other three corners, and the
/// pattern still repeats tile for tile with matching borders.
#[test]
fn a_stroke_over_the_corner_re_enters_at_the_opposite_edges() {
    let corner = line(
        Vec3::new(0.82, 0.88, 0.0),
        Vec3::new(0.96, 0.97, 0.0),
        6,
        0.15,
    );
    let d = moves(&wrapped("draw", 3), &corner);
    for (i, j) in [(0, 0), (0, N), (N, 0), (N, N)] {
        assert!(
            d[idx(i, j)][2] > 1e-3,
            "corner ({i}, {j}) rose: {:?}",
            d[idx(i, j)]
        );
    }
    let repeat = worst_repeat(&d);
    assert!(
        repeat <= CLOSE,
        "tile for tile and border to border: {repeat}"
    );
}

/// A drag (Move) and an anchored stamp (Drag Rect) repeat across the tiles as dabs do.
#[test]
fn drags_and_stamps_repeat_across_the_tiles() {
    let text = wrapped_text("draw", 3);
    let cut = text.find("[stroke]").unwrap_or(text.len());
    let stamp = format!(
        "{}[stroke]\nclass = \"anchored\"\ntap = 0.5\n",
        &text[..cut]
    );
    let c = Vec3::new(0.1, -0.2, 0.0);
    for (what, p, samples) in [
        (
            "Move",
            wrapped("move", 3),
            line(c, c + Vec3::new(0.0, 0.0, 0.1), 4, 0.2),
        ),
        (
            "Drag Rect",
            ok(BrushPreset::from_toml(&stamp)),
            line(c, c + Vec3::new(0.2, 0.0, 0.0), 4, 0.2),
        ),
    ] {
        let d = moves(&p, &samples);
        let (i, j) = (N / 2 + 3, N / 2 - 10);
        for (a, b) in [(0, 0), (TILE, 0), (0, TILE), (TILE, TILE)] {
            let m = d[idx(i - a, j + b)];
            assert!(m[2] > 1e-3, "{what}: the tile {a}, {b} over moved: {m:?}");
        }
        let repeat = worst_repeat(&d);
        assert!(repeat <= CLOSE, "{what}: {repeat}");
    }
}

/// Wrap Mode 1 is the default and is no Wrap Mode: written out, every shipped preset is the
/// same preset with the same hash, and its canonical form leaves it out, so every stroke log
/// and golden stroke keeps its bits.
#[test]
fn wrap_mode_1_is_no_wrap_mode_and_keeps_every_hash() {
    for s in shipped() {
        let one = ok(BrushPreset::from_toml(&wrapped_text(&s.stem, 1)));
        assert_eq!(one, s.preset, "{}", s.stem);
        assert_eq!(one.hash(), s.preset.hash(), "{}", s.stem);
        assert!(!s.preset.canonical().contains("wrap"), "{}", s.stem);
    }
    let three = wrapped("draw", 3);
    assert!(
        three.canonical().contains("wrap = 3"),
        "{}",
        three.canonical()
    );
    assert_ne!(three.hash(), preset("draw").preset.hash());
}

/// Each preset that cannot wrap is refused at `tip.wrap`, saying why; the edges of the range and
/// every other stroke class wrap.
#[test]
fn brushes_that_cannot_wrap_are_refused_saying_why() {
    let draw = |n: i64, extra: &str| format!("{}\n{extra}\n", wrapped_text("draw", n));
    for (what, text, says) in [
        ("0", draw(0, ""), "from 1 to 16, got 0"),
        ("17", draw(17, ""), "from 1 to 16, got 17"),
        ("a gesture", wrapped_text("select-rect", 3), "gesture"),
        (
            "Move Connected",
            wrapped_text("move-connected", 2),
            "connected",
        ),
        (
            "topology",
            draw(2, "[automask]\ntopology = 1.0"),
            "topology automask",
        ),
        (
            "face group",
            draw(2, "[automask]\nface_group = 0.5"),
            "face group automask",
        ),
    ] {
        let Err(e) = BrushPreset::from_toml(&text) else {
            panic!("{what}: refused");
        };
        assert!(e.message.starts_with("tip.wrap"), "{what}: {}", e.message);
        assert!(e.message.contains(says), "{what}: {}", e.message);
    }
    for stem in [
        "draw",
        "move",
        "move-elastic",
        "clay-line",
        "mask-pen",
        "smooth",
    ] {
        for n in [2, 16] {
            assert_eq!(wrapped(stem, n).tip.wrap, n as u32, "{stem}");
        }
    }
}

/// A stroke with Wrap Mode and symmetry is refused when it begins, naming both.
#[test]
fn a_wrapping_stroke_with_symmetry_is_refused_saying_why() {
    let mut s = store(&grid(12));
    let before = s.to_plain();
    let Err(e) = stroke(
        &mut s,
        &wrapped("draw", 3),
        params(Some(mirror_x())),
        &[at(Vec3::ZERO, 0.2)],
    ) else {
        panic!("refused");
    };
    assert!(e.message.contains("symmetry"), "{}", e.message);
    assert!(e.message.contains("tip.wrap"), "{}", e.message);
    assert!(
        canonical(&s.to_plain()) == canonical(&before),
        "nothing changed"
    );
}

/// A brush with Wrap Mode keeps Live Detail off, saying why (decision 0054, section 6): over
/// coarse quads a refining stroke would split everywhere, and this one adds no triangle and
/// still sculpts.
#[test]
fn a_wrapping_brush_keeps_live_detail_off() {
    let p = wrapped("draw", 3);
    assert_eq!(live::refuses(&p), Some(NotRefined::Wrap));
    let mut s = store(&grid(6));
    let before = (s.vertex_count(), s.face_count(), s.to_plain());
    let mut prm = params(None);
    prm.live_detail = Some(LiveDetail {
        target: Target::Constant(0.02),
        ..LiveDetail::default()
    });
    let mut st = ok(Stroke::begin(&mut s, &p, prm));
    ok(st.add(
        &mut s,
        &line(Vec3::new(-0.3, 0.0, 0.0), Vec3::new(0.3, 0.1, 0.0), 6, 0.3),
    ));
    let out = st.finish(&mut s);
    assert!(out.live.is_none(), "{:?}", out.live);
    assert_eq!((s.vertex_count(), s.face_count()), (before.0, before.1));
    let sculpted = canonical(&s.to_plain()) != canonical(&before.2);
    assert!(sculpted, "the stroke still sculpts");
}
