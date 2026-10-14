//! Cube maps of linear light on the CPU, with box-filtered mip levels: how environments are
//! stored for sampling. Face order and orientation follow the GPU convention (+X, -X, +Y, -Y,
//! +Z, -Z). Filtering is seamless: a bilinear tap that falls off its face reads the texel the
//! same direction finds on the neighboring face, so face edges never show as lines.

use super::Rgb;
use super::environment::BRIGHTEST;
use crate::math::{V3, normalize_or};

/// One square image per face, `size` texels on a side, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub size: usize,
    pub texels: Vec<Rgb>,
}

/// Six faces of one level.
pub type Level = [Face; 6];

/// The face a direction falls on, and its position there in 0..1 (u right, v down).
pub fn locate(d: V3) -> (usize, f32, f32) {
    let [x, y, z] = d;
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, sc, tc, ma) = if ax >= ay && ax >= az {
        if x > 0.0 {
            (0, -z, -y, ax)
        } else {
            (1, z, -y, ax)
        }
    } else if ay >= az {
        if y > 0.0 {
            (2, x, z, ay)
        } else {
            (3, x, -z, ay)
        }
    } else if z > 0.0 {
        (4, x, -y, az)
    } else {
        (5, -x, -y, az)
    };
    let ma = ma.max(1e-20);
    (
        face,
        f32::midpoint(sc / ma, 1.0),
        f32::midpoint(tc / ma, 1.0),
    )
}

/// The unit direction through face position (u, v) in 0..1.
pub fn direction(face: usize, u: f32, v: f32) -> V3 {
    let (s, t) = (2.0 * u - 1.0, 2.0 * v - 1.0);
    let d = match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    };
    normalize_or(d, [0.0, 0.0, 1.0])
}

/// The solid angle of the texel at column `i`, row `j` of a face `size` texels wide, exactly
/// (the area of a rectangle on the face plane projected onto the sphere).
pub fn texel_solid_angle(size: usize, i: usize, j: usize) -> f32 {
    let at = |k: usize| 2.0 * k as f32 / size as f32 - 1.0;
    let corner = |x: f32, y: f32| libm::atan2f(x * y, (x * x + y * y + 1.0).sqrt());
    let (x0, x1, y0, y1) = (at(i), at(i + 1), at(j), at(j + 1));
    corner(x0, y0) - corner(x0, y1) - corner(x1, y0) + corner(x1, y1)
}

impl Face {
    /// The texel at column `i`, row `j`.
    fn get(&self, i: usize, j: usize) -> Rgb {
        self.texels[j * self.size + i]
    }

    /// Half the size, each texel the mean of four.
    fn halve(&self) -> Face {
        let size = (self.size / 2).max(1);
        let texels = (0..size * size)
            .map(|t| {
                let (i, j) = (2 * (t % size), 2 * (t / size));
                let four = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)];
                let sum = four.iter().fold([0.0; 3], |s, &(x, y)| {
                    let c = self.get(x.min(self.size - 1), y.min(self.size - 1));
                    [s[0] + c[0], s[1] + c[1], s[2] + c[2]]
                });
                sum.map(|c| c * 0.25)
            })
            .collect();
        Face { size, texels }
    }
}

/// Face, column and row of the texel at column `i`, row `j` of `face` in a level `size` texels
/// wide, where `i` and `j` may lie one texel off the face: then the texel the same direction
/// finds on the neighboring face.
pub fn wrap(size: usize, face: usize, i: i32, j: i32) -> (usize, usize, usize) {
    if let (Ok(x), Ok(y)) = (usize::try_from(i), usize::try_from(j))
        && x < size
        && y < size
    {
        return (face, x, y);
    }
    let n = size as f32;
    let at = |k: i32| (k as f32 + 0.5) / n;
    let (other, u, v) = locate(direction(face, at(i), at(j)));
    let cell = |t: f32| ((t * n).max(0.0) as usize).min(size - 1);
    (other, cell(u), cell(v))
}

/// The texel at column `i`, row `j` of `face`, across face edges as [`wrap`] finds it.
fn texel(level: &Level, face: usize, i: i32, j: i32) -> Rgb {
    let (f, x, y) = wrap(level[face].size, face, i, j);
    level[f].get(x, y)
}

/// Bilinear light along unit direction `d` in one level, across face edges.
fn bilinear(level: &Level, d: V3) -> Rgb {
    let (face, u, v) = locate(d);
    let n = level[face].size as f32;
    let (x, y) = (u * n - 0.5, v * n - 0.5);
    let (x0, y0) = (libm::floorf(x), libm::floorf(y));
    let (fx, fy) = (x - x0, y - y0);
    let (i, j) = (x0 as i32, y0 as i32);
    let (a, b) = (texel(level, face, i, j), texel(level, face, i + 1, j));
    let (c, e) = (
        texel(level, face, i, j + 1),
        texel(level, face, i + 1, j + 1),
    );
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (e[k] - c[k]) * fx;
        top + (bottom - top) * fy
    })
}

/// A cube map with its mip levels, level 0 the sharpest.
#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    pub levels: Vec<Level>,
}

impl Cube {
    /// A cube of faces `size` texels wide, with mip levels down to one texel per face. Each
    /// texel is the mean of `light` over an `n` by `n` grid of directions across it.
    pub fn build(size: usize, n: usize, light: impl Fn(V3) -> Rgb + Sync) -> Cube {
        let base = std::array::from_fn(|face| {
            let texels = gs_par::map(size * size, |t| {
                let (i, j) = (t % size, t / size);
                let sum = (0..n * n).fold([0.0; 3], |sum, k| {
                    let at = |texel: usize, sub: usize| {
                        (texel as f32 + (sub as f32 + 0.5) / n as f32) / size as f32
                    };
                    let c = light(direction(face, at(i, k % n), at(j, k / n)));
                    [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]]
                });
                sum.map(|c| c / (n * n) as f32)
            });
            Face { size, texels }
        });
        Cube::from_base(base)
    }

    /// A cube with `base` as level 0 and box-filtered mip levels down to one texel per face.
    pub fn from_base(base: Level) -> Cube {
        let mut levels: Vec<Level> = vec![base];
        while levels.last().is_some_and(|l| l[0].size > 1) {
            let next = levels.last().map(|l| l.each_ref().map(Face::halve));
            levels.extend(next);
        }
        Cube { levels }
    }

    /// Texels per face side at level 0.
    pub fn size(&self) -> usize {
        self.levels[0][0].size
    }

    /// The light along unit direction `d` at mip `lod` (fractional levels blend two).
    pub fn sample(&self, d: V3, lod: f32) -> Rgb {
        let top = (self.levels.len() - 1) as f32;
        let lod = lod.clamp(0.0, top);
        let lo = lod as usize;
        let a = bilinear(&self.levels[lo], d);
        let t = lod - lo as f32;
        if t <= 0.0 {
            return a;
        }
        let b = bilinear(&self.levels[(lo + 1).min(self.levels.len() - 1)], d);
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }

    /// Every texel of level `lod` with its direction and solid angle, for integrals over the
    /// sphere.
    pub fn texels(&self, lod: usize) -> Vec<(V3, f32, Rgb)> {
        let level = &self.levels[lod.min(self.levels.len() - 1)];
        let size = level[0].size;
        let mut out = Vec::with_capacity(6 * size * size);
        for (face, f) in level.iter().enumerate() {
            for j in 0..size {
                for i in 0..size {
                    let (u, v) = (
                        (i as f32 + 0.5) / size as f32,
                        (j as f32 + 0.5) / size as f32,
                    );
                    let omega = texel_solid_angle(size, i, j);
                    out.push((direction(face, u, v), omega, f.get(i, j)));
                }
            }
        }
        out
    }

    /// The same cube with every texel times `k`, held at [`BRIGHTEST`]. A `k` that is an f32
    /// gives what the f32 product gives: two f32 values multiply exactly in f64 and round once.
    #[must_use]
    pub fn scaled(mut self, k: f64) -> Cube {
        let most = f64::from(BRIGHTEST);
        for level in &mut self.levels {
            for face in level.iter_mut() {
                for t in &mut face.texels {
                    *t = t.map(|c| (f64::from(c) * k).min(most) as f32);
                }
            }
        }
        self
    }
}
