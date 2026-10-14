//! Select Rect's rectangle (decision 0051, sections 3 and 4): drawn from the press to the
//! release corner to corner, whatever the pointer did between; Square keeps its sides equal and
//! Center grows it from the press; a rectangle with no area does nothing; its sides are square
//! to the screen. Each is checked against a polygon copy of Select Rect fed the four corners
//! worked out here, or against the faces wholly inside the corners.

use gs_core::Vec3;
use gs_core::brush::BrushPreset;
use gs_core::stroke::rectangle;
use gs_protocol::PlainMesh;
use gs_testkit::compare::canonical;

use crate::brush_kit::{grid, hidden_faces, lasso, params, preset, run, store};
use crate::common::ok;
use crate::gestures::outline;

/// Select Rect with `extra` lines added to its stroke table (its last).
fn select_rect(extra: &str) -> BrushPreset {
    let text = preset("select-rect").text;
    let stroke = text.find("[stroke]").unwrap_or_default();
    assert!(
        !text[stroke + 1..].contains('['),
        "the stroke table comes last"
    );
    ok(BrushPreset::from_toml(&format!("{text}{extra}\n")))
}

/// The grid after a stroke of `p` through `pts`.
fn after(p: &BrushPreset, pts: &[(f32, f32)]) -> PlainMesh {
    let mut m = store(&grid(32));
    run(&mut m, p, params(None), &outline(pts));
    m.to_plain()
}

/// The grid after the polygon through the corners of the rectangle from `a` to `b`, in the
/// order the rectangle has them: `a`, then along x, `b`, then back along y.
fn corner_to_corner(a: (f32, f32), b: (f32, f32)) -> PlainMesh {
    after(&lasso(), &[a, (b.0, a.1), b, (a.0, b.1)])
}

/// Some faces hidden and some shown: the rectangle cut the grid.
fn cut(m: &PlainMesh) -> bool {
    let hidden = hidden_faces(m);
    hidden.iter().any(|&h| h) && hidden.iter().any(|&h| !h)
}

#[test]
fn select_rect_runs_from_the_press_to_the_release_whatever_the_pointer_did_between() {
    let rect = select_rect("");
    let drawn = after(&rect, &[(-0.5, -0.4), (0.9, 0.7), (-0.8, 0.2), (0.6, 0.5)]);
    assert!(
        cut(&drawn),
        "the rectangle shows some faces and hides the rest"
    );
    assert!(
        canonical(&drawn) == canonical(&corner_to_corner((-0.5, -0.4), (0.6, 0.5))),
        "the corners are the press and the release"
    );
    let flat = store(&grid(32)).to_plain();
    for no_area in [
        [(0.1, 0.2), (0.5, 0.6), (0.1, 0.7)],
        [(0.1, 0.2), (-0.5, 0.6), (0.4, 0.2)],
    ] {
        assert!(
            canonical(&after(&rect, &no_area)) == canonical(&flat),
            "a rectangle with a side of zero does nothing: {no_area:?}"
        );
    }
}

#[test]
fn square_keeps_the_sides_equal_and_center_grows_the_rectangle_from_the_press() {
    let pts = |a: (f32, f32), b: (f32, f32)| [a, (0.9, -0.8), b];
    let square = after(
        &select_rect("square = true"),
        &pts((-0.5, -0.375), (0.625, 0.125)),
    );
    assert!(cut(&square), "the square cuts the grid");
    assert!(
        canonical(&square) == canonical(&corner_to_corner((-0.5, -0.375), (0.625, 0.75))),
        "Square: both sides take the longer part of the drag, the corner on the release's side"
    );
    let center = after(
        &select_rect("center = true"),
        &pts((0.125, 0.0), (0.5, 0.25)),
    );
    assert!(
        canonical(&center) == canonical(&corner_to_corner((-0.25, -0.25), (0.5, 0.25))),
        "Center: the press is the middle and the release a corner"
    );
    let both = after(
        &select_rect("square = true\ncenter = true"),
        &pts((0.125, 0.0), (0.5, 0.25)),
    );
    assert!(
        canonical(&both) == canonical(&corner_to_corner((-0.25, -0.375), (0.5, 0.375))),
        "Center with Square: a square centered on the press"
    );
}

#[test]
fn the_rectangle_from_two_points_in_two_dimensions() {
    let (a, b) = ([1.0, 2.0], [4.0, 0.0]);
    let cases = [
        (
            false,
            false,
            [[1.0, 2.0], [4.0, 2.0], [4.0, 0.0], [1.0, 0.0]],
        ),
        (
            true,
            false,
            [[1.0, 2.0], [4.0, 2.0], [4.0, -1.0], [1.0, -1.0]],
        ),
        (
            false,
            true,
            [[-2.0, 4.0], [4.0, 4.0], [4.0, 0.0], [-2.0, 0.0]],
        ),
        (
            true,
            true,
            [[-2.0, 5.0], [4.0, 5.0], [4.0, -1.0], [-2.0, -1.0]],
        ),
    ];
    for (square, center, corners) in cases {
        assert_eq!(
            rectangle(a, b, square, center),
            corners,
            "square {square}, center {center}"
        );
    }
}

#[test]
fn the_rectangle_is_square_to_the_screen_whose_up_the_stroke_is_given() {
    // The camera turned 45 degrees about its view: a drag along x is the diagonal of a square
    // standing on its corner, (0, 0), (0.25, -0.25), (0.5, 0) and (0.25, 0.25).
    let a = std::f32::consts::FRAC_1_SQRT_2;
    let mut prm = params(None);
    prm.up = Some(Vec3::new(-a, a, 0.0));
    let mut m = store(&grid(32));
    run(
        &mut m,
        &select_rect(""),
        prm,
        &outline(&[(0.0, 0.0), (0.5, 0.0)]),
    );
    let m = m.to_plain();
    // Above 0 inside the square, below 0 outside.
    let depth = |v: u32| {
        let p = m.positions[v as usize];
        0.25 - (p[0] - 0.25).abs() - p[1].abs()
    };
    let mut checked = [0; 2];
    for (face, hidden) in m.faces.iter().zip(hidden_faces(&m)) {
        let corners: Vec<f32> = face
            .iter()
            .filter(|&&v| v != u32::MAX)
            .map(|&v| depth(v))
            .collect();
        // A corner on the outline could round either way.
        if corners.iter().any(|d| d.abs() < 1e-3) {
            continue;
        }
        let within = corners.iter().all(|&d| d > 0.0);
        assert_eq!(
            hidden, !within,
            "{face:?}: the faces wholly inside show, the rest hide"
        );
        checked[usize::from(within)] += 1;
    }
    assert!(
        checked[0] > 0 && checked[1] > 0,
        "{checked:?} faces outside and inside"
    );
}

#[test]
fn square_and_center_are_refused_on_any_shape_but_the_rectangle() {
    let text = preset("select-rect")
        .text
        .replace("shape = \"rect\"", "shape = \"polygon\"");
    for key in ["square", "center"] {
        let e = BrushPreset::from_toml(&format!("{text}{key} = true\n")).err();
        let message = e.map(|e| e.message).unwrap_or_default();
        assert!(
            message.contains(key) && message.contains("rect"),
            "{key} on a polygon: {message}"
        );
    }
}
