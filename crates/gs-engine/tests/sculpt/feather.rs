//! The symmetry feather under a mirrored Move just beside the plane (decision 0040, research 23
//! section 2.4). Move works from the stroke-start surface and adds its passes' drags, so a
//! mirrored Move is its two one-sided strokes added and scaled by the dab's feather, one factor
//! for the whole dab. A per-point feather takes, point by point, the larger of the two instead,
//! which leaves a notch on the plane between them.

use gs_core::math::Vec3;
use gs_core::symmetry::Symmetry;
use serde_json::json;

use crate::{line, sphere_session, stroke};

/// Brush radius: research 23's 70 pixel brush in its orthographic views.
const RADIUS: f32 = 0.26;

/// Where research 23's nose stroke grabs, at `x` on the front of the unit sphere.
fn grab(x: f32) -> [f32; 3] {
    [x, -0.15, (1.0 - x * x - 0.0225).sqrt()]
}

/// Each point's move under research 23's nose drag at `x`, mirrored across x if `mirror`.
fn moves(x: f32, mirror: bool) -> Vec<[f32; 3]> {
    let mut s = sphere_session(None);
    s.ok("mesh.symmetrize", json!({ "axis": "x" }));
    let before = s.mesh("sphere").positions;
    let mut extra = json!({ "size": RADIUS });
    if mirror {
        extra["symmetry"] = json!({ "mirror": { "x": true } });
    }
    let points = line(grab(x), [x, -0.3, 1.25], 12);
    s.ok("brush.stroke", stroke("Move", &points, &extra));
    let after = s.mesh("sphere").positions;
    (before.iter().zip(&after))
        .map(|(b, a)| std::array::from_fn(|k| a[k] - b[k]))
        .collect()
}

#[test]
fn a_mirrored_move_beside_the_plane_is_its_two_sides_added_and_feathered() {
    let x = 0.12;
    let both = moves(x, true);
    let (right, left) = (moves(x, false), moves(-x, false));
    let sym = Symmetry {
        mirror: [true, false, false],
        ..Symmetry::default()
    };
    let k = sym.feather_scale(Vec3::from_array(grab(x)), RADIUS);
    assert!(k > 0.6 && k < 0.7, "the passes overlap: {k}");
    let len = |v: [f32; 3]| v.iter().map(|c| c * c).sum::<f32>().sqrt();
    let most = both.iter().map(|&v| len(v)).fold(0.0, f32::max);
    let (worst, at) = (0..both.len())
        .map(|i| {
            let want: [f32; 3] = std::array::from_fn(|c| k * (right[i][c] + left[i][c]));
            (len(std::array::from_fn(|c| both[i][c] - want[c])), i)
        })
        .fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a });
    assert!(
        worst <= 0.01 * most,
        "point {at} moved {:?}, not {k} times its two sides' {:?} and {:?}",
        both[at],
        right[at],
        left[at]
    );
}
