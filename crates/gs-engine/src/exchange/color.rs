//! Colors back from Blender (decision 0032, section 8's amendment): the active color attribute
//! as the add-on copied it, at the point or corner domain, as bytes or floats, made into the
//! store's opaque sRGB words. A point byte color with alpha 255, which is how GigaSculpt's own
//! colors come back, is taken as it is. Anything else is read as Blender stores it (linear
//! light, premultiplied alpha), averaged to points, laid over white and rounded to the nearest
//! byte.

use gs_core::columns::{linear_to_srgb, srgb_to_linear};

use super::names::{Column, Domain, Elem};

/// Opaque white, the color column's empty value (decision 0039, section 2).
const WHITE: u32 = u32::MAX;

/// The store's color word for each of `points` points, from color column `row`'s file `bytes`.
/// A corner column needs `corner_vert`, each corner's point (checked by the caller). Refuses a
/// float that is not a finite number, saying so.
pub fn points(
    row: Column,
    bytes: &[u8],
    corner_vert: &[i32],
    points: usize,
) -> Result<Vec<u32>, String> {
    let elements = || -> Result<Vec<[f32; 4]>, String> {
        match row.elem {
            Elem::U8 => Ok(bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&b| linear(b))
                .collect()),
            _ => floats(bytes),
        }
    };
    match (row.domain, row.elem) {
        (Domain::Point, Elem::U8) => {
            Ok(bytes.as_chunks::<4>().0.iter().map(|&b| word(b)).collect())
        }
        (Domain::Point, _) => Ok(elements()?.into_iter().map(opaque).collect()),
        (Domain::Corner, _) => {
            let mut sum = vec![([0.0f32; 4], 0u32); points];
            for (&v, c) in corner_vert.iter().zip(elements()?) {
                let (s, n) = &mut sum[v.unsigned_abs() as usize];
                for (s, c) in s.iter_mut().zip(c) {
                    *s += c;
                }
                *n += 1;
            }
            let mean = |(s, n): ([f32; 4], u32)| {
                if n == 0 {
                    WHITE
                } else {
                    opaque(s.map(|x| x / n as f32))
                }
            };
            Ok(sum.into_iter().map(mean).collect())
        }
        (Domain::Face, _) => Err("has a face color, which the table does not have".into()),
    }
}

/// A point's byte color: as it is when opaque, else laid over white.
fn word(b: [u8; 4]) -> u32 {
    if b[3] == 255 {
        u32::from_le_bytes(b)
    } else {
        opaque(linear(b))
    }
}

/// A byte color as Blender stores it, in linear light with premultiplied alpha: red, green and
/// blue through the sRGB curve, alpha divided by 255.
fn linear(b: [u8; 4]) -> [f32; 4] {
    let [r, g, bl, a] = b.map(|x| f32::from(x) / 255.0);
    [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(bl), a]
}

/// Four-float colors from their file, every value a finite number.
fn floats(bytes: &[u8]) -> Result<Vec<[f32; 4]>, String> {
    let values: Vec<f32> = (bytes.as_chunks::<4>().0.iter())
        .map(|&b| f32::from_le_bytes(b))
        .collect();
    if values.iter().any(|x| !x.is_finite()) {
        return Err("has a color that is not a finite number".into());
    }
    Ok(values.as_chunks::<4>().0.to_vec())
}

/// A linear premultiplied color laid over white, `c + (1 - a)` with alpha clamped to 0 to 1
/// first and each channel after, then encoded to sRGB bytes rounded to the nearest, opaque.
fn opaque(c: [f32; 4]) -> u32 {
    let a = c[3].clamp(0.0, 1.0);
    let byte = |v: f32| {
        let s = linear_to_srgb((v + (1.0 - a)).clamp(0.0, 1.0));
        (255.0 * s + 0.5).floor() as u8
    };
    u32::from_le_bytes([byte(c[0]), byte(c[1]), byte(c[2]), 255])
}
