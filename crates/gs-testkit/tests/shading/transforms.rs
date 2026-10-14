//! gs-render's view transforms against references outside it (decision 0019).

use gs_render::ViewTransform;
use gs_testkit::reference::tonemap;

/// Linear light from black past white, dark to bright and gray to fully saturated, the way
/// looks and highlights reach the display.
fn colors() -> Vec<[f32; 3]> {
    let levels = [0.0f32, 0.002, 0.02, 0.06, 0.18, 0.4, 0.8, 1.5, 4.0, 16.0];
    let mut out = Vec::new();
    for &r in &levels {
        for &g in &levels {
            for &b in &levels {
                out.push([r, g, b]);
            }
        }
    }
    out
}

/// `AgX` is three.js's `AgXToneMapping` (which follows Filament's operator and Blender's
/// matrices): every color within 0.001 of display-linear light, a quarter of an 8-bit level at
/// most. What is left is f32 against f64 and the Rec. 2020 matrices' last digits (three.js
/// rounds them to four decimals).
#[test]
fn agx_is_the_reference_agx() {
    let mut worst = (0.0f64, [0.0f32; 3]);
    for c in colors() {
        let ours = ViewTransform::Agx.apply(c);
        let want = tonemap::agx(c.map(f64::from));
        for k in 0..3 {
            let off = (f64::from(ours[k]) - want[k]).abs();
            if off > worst.0 {
                worst = (off, c);
            }
        }
    }
    assert!(
        worst.0 < 0.001,
        "AgX differs from three.js's by {} at {:?}",
        worst.0,
        worst.1
    );
}
