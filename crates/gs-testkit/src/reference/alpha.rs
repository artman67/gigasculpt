//! A slow, obvious reference for reading an alpha, in `f64`, written from the specification in
//! the doc comment of gs-core's `brush` module (decision 0018). It takes the processed picture's
//! values as plain numbers and shares nothing else with the engine.
//!
//! Its levels are box filters of the picture without rounding: each texel of a level is the
//! mean of 2 by 2 texels of the level before, reading the last row or column again past the
//! edge. The engine rounds every level to 16 bits, so the two differ by at most half a step
//! per level.

/// A level: width, height, values 0 to 1 row by row from the top.
#[derive(Clone, Debug)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f64>,
}

impl Level {
    fn at(&self, x: usize, y: usize) -> f64 {
        self.values[y.min(self.height - 1) * self.width + x.min(self.width - 1)]
    }
}

/// `count` levels read from another pyramid as plain data: level `k` is `size(k)` texels with
/// the 16-bit values `texel(k, x, y)`.
pub fn read(
    count: usize,
    size: impl Fn(usize) -> [u32; 2],
    texel: impl Fn(usize, u32, u32) -> u16,
) -> Vec<Level> {
    (0..count)
        .map(|k| {
            let [w, h] = size(k);
            let values = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| f64::from(texel(k, x, y)) / 65535.0)
                .collect();
            Level {
                width: w as usize,
                height: h as usize,
                values,
            }
        })
        .collect()
}

/// Every level of a picture `width` by `height` with 16-bit `values`, finest first, down to 1
/// by 1.
pub fn levels(width: usize, height: usize, values: &[u16]) -> Vec<Level> {
    let mut out = vec![Level {
        width,
        height,
        values: values.iter().map(|&v| f64::from(v) / 65535.0).collect(),
    }];
    while let Some(l) = out.last().filter(|l| l.width > 1 || l.height > 1) {
        let (w, h) = (l.width.div_ceil(2), l.height.div_ceil(2));
        let mut values = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (2 * x, 2 * y);
                let sum = l.at(a, b) + l.at(a + 1, b) + l.at(a, b + 1) + l.at(a + 1, b + 1);
                values.push(sum / 4.0);
            }
        }
        out.push(Level {
            width: w,
            height: h,
            values,
        });
    }
    out
}

/// The value at `(u, v)` (each -1 to 1 across the dab; the top row at `v = 1`) on `level`,
/// repeated `tiles` times: the four texels around the point blended bilinearly, held at the
/// edges or wrapped across tiles; 0 outside the square.
pub fn sample(level: &Level, u: f64, v: f64, tiles: [u32; 2]) -> f64 {
    if !(u.abs() <= 1.0 && v.abs() <= 1.0) {
        return 0.0;
    }
    let around = |s: f64, n: usize, t: u32| {
        let s = if t > 1 {
            (s * f64::from(t)).rem_euclid(1.0)
        } else {
            s
        };
        let x = s * n as f64 - 0.5;
        let x0 = x.floor();
        let f = x - x0;
        let n = i64::try_from(n).unwrap_or(i64::MAX);
        let pick = |i: i64| {
            if t > 1 {
                i.rem_euclid(n) as usize
            } else {
                i.clamp(0, n - 1) as usize
            }
        };
        (pick(x0 as i64), pick(x0 as i64 + 1), f)
    };
    let (x0, x1, fx) = around(f64::midpoint(u, 1.0), level.width, tiles[0]);
    let (y0, y1, fy) = around(f64::midpoint(1.0, -v), level.height, tiles[1]);
    let at = |x: usize, y: usize| level.values[y * level.width + x];
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}
