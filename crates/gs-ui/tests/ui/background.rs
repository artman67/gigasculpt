//! The viewport's background (catalog item `canvas.background`) through gs-ui's render host:
//! `view.background`'s color fills what no part covers, a gradient of 0 draws it flat, and the
//! gradient runs from white above to black below, half white at its center.

use serde_json::{Value, json};

use crate::looks::{run, snapshot};
use crate::materials::empty;

/// The left column of a sphere seen from the front, 64 pixels square, after `view.background`
/// with `args`: background only, top to bottom.
fn column(args: Value) -> Vec<[i32; 3]> {
    let mut e = empty();
    run(&mut e, "primitive.sphere", json!({}));
    run(&mut e, "view.background", args);
    let shot = snapshot(
        &mut e,
        json!({ "view": "front", "size": 64, "antialias": false }),
    );
    (0..shot.0)
        .map(|y| {
            let i = (y * shot.0 * 4) as usize;
            [0, 1, 2].map(|k| i32::from(shot.1[i + k]))
        })
        .collect()
}

/// Whether every pixel of `pixels` is `want`, within `levels` per channel.
fn all_near(pixels: &[[i32; 3]], want: [i32; 3], levels: i32) -> bool {
    (pixels.iter()).all(|p| (0..3).all(|k| (p[k] - want[k]).abs() <= levels))
}

/// The color fills what no part covers: today's dark gray (40, 42, 46) until it is set, then
/// the color given, flat while the gradient is 0 wherever its center and softness are.
#[test]
fn the_back_color_fills_what_no_part_covers_flat_without_a_gradient() {
    let start = column(json!({}));
    assert!(all_near(&start, [40, 42, 46], 0), "{:?}", start[0]);
    let gray = column(json!({ "color": "#3a3a3a" }));
    assert!(all_near(&gray, [58, 58, 58], 0), "{:?}", gray[0]);
    let flat = column(json!({ "color": "#3a3a3a", "gradient": 0, "center": 0.2, "softness": 0 }));
    assert!(all_near(&flat, [58; 3], 0), "gradient 0: {:?}", flat[0]);
}

/// At gradient 1 the gradient alone shows: near white at the top, near black at the bottom,
/// gray all the way, darker on every row down and half white (sRGB 128) at the center. At 0.5
/// with softness 0 it is a hard edge at its center (a quarter down): above it the color halfway
/// to white, below it the color halfway to black.
#[test]
fn a_gradient_runs_white_to_black_down_the_view_half_white_at_its_center() {
    let soft = column(json!({ "gradient": 1, "center": 0.5, "softness": 1 }));
    let (top, bottom) = (soft[0][0], soft[soft.len() - 1][0]);
    assert!(top > 245 && bottom < 10, "{top} to {bottom}");
    assert!(soft.iter().all(|p| p[0] == p[1] && p[1] == p[2]), "gray");
    assert!(soft.windows(2).all(|w| w[1][0] < w[0][0]), "{soft:?}");
    let middle = f64::from(soft[31][0] + soft[32][0]) / 2.0;
    assert!((middle - 128.0).abs() <= 2.0, "half white: {middle}");
    let edge = json!({ "color": "#3a3a3a", "gradient": 0.5, "center": 0.25, "softness": 0 });
    let hard = column(edge);
    let light = i32::midpoint(58, 255);
    assert!(all_near(&hard[..16], [light; 3], 2), "{:?}", &hard[..16]);
    assert!(all_near(&hard[16..], [29; 3], 2), "{:?}", &hard[16..]);
}
