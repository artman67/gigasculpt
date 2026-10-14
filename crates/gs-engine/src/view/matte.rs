//! A look with no background (`view.snapshot`'s `transparent`). The renderer draws every pixel
//! opaque, so the look is drawn twice, over flat black and over flat white, and each pixel's
//! alpha is how much of the background does not show through it: a pixel a part covers is the
//! same in both, one no part covers is black in one and white in the other, and a pixel on an
//! anti-aliased edge is between. The renderer is left as it is.

use gs_core::MeshStore;
use gs_core::columns::{linear_to_srgb, srgb_to_linear};
use gs_protocol::CommandError;

use super::{RenderRequest, render};
use crate::display::Background;
use crate::image::{Picture, decode, encode};
use crate::{Ctx, Screenshot};

/// `parts` drawn as `request` asks with its background left out: a PNG of 8-bit RGBA, its
/// color not multiplied by its alpha.
pub(super) fn transparent(
    cx: &Ctx,
    parts: &[&MeshStore],
    mut request: RenderRequest,
) -> Result<Screenshot, CommandError> {
    let mut over = |color: [u8; 3]| {
        let flat = Background {
            color,
            gradient: 0.0,
            ..request.display.background
        };
        request.display.background = flat;
        decode(&render(cx, parts, &request)?.png, "the render")
    };
    let black = over([0; 3])?;
    let white = over([255; 3])?;
    let pairs = (black.rgba.as_chunks::<4>().0.iter()).zip(white.rgba.as_chunks::<4>().0);
    let rgba = pairs.flat_map(|(b, w)| matte(*b, *w)).collect();
    let picture = Picture { rgba, ..black };
    Ok(Screenshot {
        width: picture.width,
        height: picture.height,
        png: encode(&picture)?,
    })
}

/// One pixel from the pixel over black, `b`, and over white, `w`. The renderer blends a partly
/// covered pixel with the background in linear light: over black each channel is the part's
/// color times the alpha, and over white that plus one minus the alpha.
fn matte(b: [u8; 4], w: [u8; 4]) -> [u8; 4] {
    if b == w {
        return [b[0], b[1], b[2], 255];
    }
    let linear = |v: u8| srgb_to_linear(f32::from(v) / 255.0);
    let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let shows = (0..3).map(|k| linear(w[k]) - linear(b[k])).sum::<f32>() / 3.0;
    let alpha = 1.0 - shows;
    if alpha < 0.5 / 255.0 {
        return [0; 4];
    }
    let color = |k: usize| level(linear_to_srgb(linear(b[k]) / alpha));
    [color(0), color(1), color(2), level(alpha)]
}
