//! Drag Rect and Drag Dot (decision 0051, sections 1, 2 and 5), through Clay stamping the
//! shipped checker alpha: a Drag Rect stamp is centered on the press, as large as the drag and
//! turned to it, a Drag Dot stamp is the brush's own tap at the pointer, the release keeps
//! exactly what was last shown, and the checker comes out as a height pattern with nothing
//! moved outside the alpha's footprint. Every expectation is computed here from plain data.

use gs_core::brush::BrushPreset;
use gs_core::{Stroke, StrokeSample, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::compare::canonical;

use crate::brush_kit::{at, grid, params, preset, shipped_alpha, store};
use crate::common::ok;

/// Clay stamping the checker alpha, with `stroke` as its stroke table's lines.
fn clay_checker(stroke: &str) -> BrushPreset {
    let text = preset("clay").text;
    let grainy = "image = \"grainy-round\"";
    assert!(text.contains(grainy), "Clay stamps the grainy round tip");
    let text = text.replace(grainy, "image = \"checker\"");
    let cut = text.find("[stroke]").unwrap_or(text.len());
    ok(BrushPreset::from_toml(&format!(
        "{}[stroke]\n{stroke}\n",
        &text[..cut]
    )))
}

/// The mesh while the pen is still down after `samples` (each fed as the pen sends it), and
/// after the stroke ends.
fn drawn(p: &BrushPreset, mesh: &PlainMesh, samples: &[StrokeSample]) -> (PlainMesh, PlainMesh) {
    let mut m = store(mesh);
    let mut prm = params(None);
    prm.alpha = shipped_alpha(p);
    let mut k = ok(Stroke::begin(&mut m, p, prm));
    for s in samples {
        ok(k.add(&mut m, std::slice::from_ref(s)));
    }
    let shown = m.to_plain();
    k.finish(&mut m);
    (shown, m.to_plain())
}

/// `after` is the flat `before` with the checker raised in a stamp centered at `c`, its picture's
/// columns along the angle `turn` (radians, about +z) and its square spanning `r` each way:
/// vertices well inside a white square rose, those well inside a black one stayed, nothing
/// moved but up, and nothing at `r` or beyond moved.
fn copies_checker(before: &PlainMesh, after: &PlainMesh, c: Vec3, turn: f32, r: f32, what: &str) {
    let (cos, sin) = (libm::cosf(turn), libm::sinf(turn));
    let (t, b) = (Vec3::new(cos, sin, 0.0), Vec3::new(-sin, cos, 0.0));
    let (mut white, mut black) = (0, 0);
    for (p, q) in before.positions.iter().zip(&after.positions) {
        let (p, q) = (Vec3::from_array(*p), Vec3::from_array(*q));
        let moved = p != q;
        assert!(
            !moved || ((q - p).x == 0.0 && (q - p).y == 0.0 && q.z > p.z),
            "{what}: {p:?} only rises, yet went to {q:?}"
        );
        let rel = p - c;
        let d = rel.len2().sqrt();
        if d >= r * 1.001 {
            assert!(!moved, "{what}: {p:?} is outside the footprint, yet moved");
            continue;
        }
        // Columns and rows of the 8 by 8 squares, the picture's top row first.
        let (col, row) = ((rel.dot(t) / r + 1.0) * 4.0, (1.0 - rel.dot(b) / r) * 4.0);
        let near_edge = |x: f32| (x - x.floor() - 0.5).abs() > 0.3;
        if near_edge(col) || near_edge(row) || d > 0.8 * r {
            continue;
        }
        if (col.floor() as i32 + row.floor() as i32) % 2 == 0 {
            assert!(moved, "{what}: {p:?} is on a white square, yet stayed");
            white += 1;
        } else {
            assert!(!moved, "{what}: {p:?} is on a black square, yet moved");
            black += 1;
        }
    }
    assert!(
        white > 50 && black > 50,
        "{what}: {white} white and {black} black vertices checked"
    );
}

#[test]
fn a_drag_rect_stamp_grows_and_turns_with_the_drag_and_its_release_keeps_the_last_preview() {
    let p = clay_checker("class = \"anchored\"\ntap = 0.5");
    let flat = store(&grid(128)).to_plain();
    let c = Vec3::new(0.1, -0.05, 0.0);
    let toward = |deg: f32, r: f32| {
        let a = deg.to_radians();
        c + Vec3::new(libm::cosf(a), libm::sinf(a), 0.0) * r
    };
    for (deg, r) in [(0.0f32, 0.5f32), (30.0, 0.6), (115.0, 0.4), (200.0, 0.45)] {
        // The pointer wanders before it settles: only where it is last counts.
        let path = [
            c,
            toward(deg + 90.0, r * 0.5),
            toward(deg - 40.0, r * 1.3),
            toward(deg, r),
        ];
        let (shown, done) = drawn(&p, &flat, &path.map(|x| at(x, 0.2)));
        let what = format!("dragged {r} at {deg} degrees");
        assert!(
            canonical(&shown) == canonical(&done),
            "{what}: the release keeps the preview"
        );
        copies_checker(&flat, &done, c, deg.to_radians(), r, &what);
    }
    // Until the pointer leaves the press nothing shows, and a release there changes nothing.
    let (shown, done) = drawn(&p, &flat, &[at(c, 0.2), at(c, 0.2)]);
    assert!(
        canonical(&shown) == canonical(&flat),
        "a press shows nothing"
    );
    assert!(
        canonical(&done) == canonical(&flat),
        "a press changes nothing"
    );
}

#[test]
fn a_drag_dot_stamp_follows_the_pointer_and_leaves_one_tap_where_it_is_let_go() {
    let dot = clay_checker("class = \"anchored\"\nfollow = true\ntap = 0.5");
    let tap = clay_checker("class = \"dab\"\ntap = 0.5");
    let flat = store(&grid(128)).to_plain();
    let path: Vec<StrokeSample> = (0..7)
        .map(|i| {
            let i = i as f32;
            at(
                Vec3::new(-0.4 + 0.1 * i, 0.15 * libm::sinf(i * 0.9), 0.0),
                0.4,
            )
        })
        .collect();
    let last = path[path.len() - 1];
    let (shown, done) = drawn(&dot, &flat, &path);
    assert!(
        canonical(&shown) == canonical(&done),
        "the release keeps the preview"
    );
    let (_, one) = drawn(&tap, &flat, &[last]);
    assert!(
        canonical(&done) == canonical(&one),
        "one stamp, the brush's own tap at the last point: nothing along the path"
    );
    copies_checker(&flat, &done, last.pos, 0.0, 0.4, "Drag Dot");
}
