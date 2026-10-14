//! How good a finished layout is, by research 28's measures (decision 0061 section 1, UV2.5):
//! stretch, flips, use of the square, overlap, the narrowest gap and texel density. Each quad is
//! read as its two triangles by `quad_diagonal`. Sums are fixed-chunk `gs_par` sums and the logs
//! `libm`'s, so every number is the same bits at any thread count.

use gs_protocol::face_triangles;

use super::{Corners, Surface};
use crate::math::{cross, dot, sub};

/// A finished layout: the part, a UV per face corner and a chart per face.
#[derive(Clone, Copy, Debug)]
pub struct Layout<'a> {
    pub surface: Surface<'a>,
    pub uvs: &'a [Corners],
    pub chart: &'a [u32],
}

/// Stretch, on the map from UV back to the surface, normalized by the layout's whole area ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stretch {
    /// Sander's L2, 1 when isometric.
    pub l2: f64,
    /// The 99th percentile of the larger singular value, by surface area.
    pub p99: f64,
    /// The largest singular value.
    pub worst: f64,
    /// The area-weighted mean ratio of the singular values, 1 when conformal.
    pub angle: f64,
    /// The area-weighted mean `|ln|` of each triangle's area ratio over the whole's.
    pub area_spread: f64,
}

/// Triangles wound against their chart, and charts wound against the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flips {
    pub flipped: usize,
    pub mirrored: usize,
}

/// The layout on a square raster of the unit square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cover {
    /// The share of pixels covered.
    pub used: f64,
    /// Pixels covered more than once.
    pub overlap: usize,
    /// The fewest pixels between two charts, `None` with fewer than two.
    pub gap: Option<u32>,
}

/// Texel density, each chart's UV length per surface length, by surface area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Density {
    pub p5: f64,
    pub p95: f64,
}

/// A triangle as the metrics and the cutter's check read it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tri {
    /// Its surface area.
    pub area3: f64,
    /// Its signed UV area, positive counterclockwise.
    pub area2: f64,
    /// The singular values of its map from UV to the surface, `sigma >= gamma`.
    pub sigma: f64,
    pub gamma: f64,
}

impl Tri {
    fn new(p: [[f64; 3]; 3], q: [[f64; 2]; 3]) -> Tri {
        let c = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        let area3 = dot(c, c).sqrt() / 2.0;
        let two =
            (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[2][0] - q[0][0]) * (q[1][1] - q[0][1]);
        if two == 0.0 {
            let sigma = if area3 > 0.0 { f64::INFINITY } else { 0.0 };
            return Tri {
                area3,
                area2: 0.0,
                sigma,
                gamma: 0.0,
            };
        }
        let along = |d: [f64; 3]| -> [f64; 3] {
            std::array::from_fn(|i| (p[0][i] * d[0] + p[1][i] * d[1] + p[2][i] * d[2]) / two)
        };
        let su = along([q[1][1] - q[2][1], q[2][1] - q[0][1], q[0][1] - q[1][1]]);
        let sv = along([q[2][0] - q[1][0], q[0][0] - q[2][0], q[1][0] - q[0][0]]);
        let (a, b, c) = (dot(su, su), dot(su, sv), dot(sv, sv));
        let root = ((a - c) * (a - c) + 4.0 * b * b).sqrt();
        Tri {
            area3,
            area2: two / 2.0,
            sigma: ((a + c + root) / 2.0).sqrt(),
            gamma: ((a + c - root) / 2.0).max(0.0).sqrt(),
        }
    }

    /// Its share of the area spread, given the whole layout's squared scale.
    pub(crate) fn spread(&self, scale2: f64) -> f64 {
        self.area3 * libm::fabs(libm::log(self.area2.abs() / self.area3 / scale2))
    }
}

/// The triangles of `face`, with `at(k)` the surface position and `uv(k)` the UV of corner k.
pub(crate) fn triangles(
    face: [u32; 4],
    at: impl Fn(usize) -> [f32; 3],
    uv: impl Fn(usize) -> [f32; 2],
) -> impl Iterator<Item = Tri> {
    (face_triangles(face, &at).iter()).map(move |t| {
        Tri::new(
            t.map(|k| at(k).map(f64::from)),
            t.map(|k| uv(k).map(f64::from)),
        )
    })
}

/// What stretch sums over triangles with surface area.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sums {
    pub area3: f64,
    pub area2: f64,
    l2: f64,
    angle: f64,
    pub worst: f64,
}

impl Sums {
    pub(crate) fn add(mut self, t: &Tri) -> Sums {
        if t.area3 > 0.0 {
            self.area3 += t.area3;
            self.area2 += t.area2.abs();
            self.l2 += t.area3 * (t.sigma * t.sigma + t.gamma * t.gamma) / 2.0;
            self.angle += t.area3 * t.sigma / t.gamma;
            self.worst = self.worst.max(t.sigma);
        }
        self
    }

    fn merge(self, o: Sums) -> Sums {
        Sums {
            area3: self.area3 + o.area3,
            area2: self.area2 + o.area2,
            l2: self.l2 + o.l2,
            angle: self.angle + o.angle,
            worst: self.worst.max(o.worst),
        }
    }

    /// The whole layout's scale, `sqrt(ΣA_uv / ΣA_3d)`.
    pub(crate) fn scale(&self) -> f64 {
        (self.area2 / self.area3).sqrt()
    }
}

/// The triangles of face `f` in `layout`.
fn layout_triangles<'a>(layout: &'a Layout, f: usize) -> impl Iterator<Item = Tri> + 'a {
    let face = layout.surface.faces[f];
    let points = layout.surface.points;
    let uvs = &layout.uvs[f];
    triangles(face, move |k| points[face[k] as usize], move |k| uvs[k])
}

/// Every triangle of the layout, face by face.
fn all(layout: &Layout) -> Vec<Tri> {
    let per = gs_par::map(layout.surface.faces.len(), |f| {
        layout_triangles(layout, f).collect::<Vec<_>>()
    });
    per.into_iter().flatten().collect()
}

/// The layout's stretch.
pub fn stretch(layout: &Layout) -> Stretch {
    let tris = all(layout);
    let sums = gs_par::reduce(
        tris.len(),
        Sums::default,
        |s, i| s.add(&tris[i]),
        Sums::merge,
    );
    if sums.area3 <= 0.0 {
        return Stretch {
            l2: 1.0,
            p99: 1.0,
            worst: 1.0,
            angle: 1.0,
            area_spread: 0.0,
        };
    }
    let s = sums.scale();
    let spread = gs_par::reduce(
        tris.len(),
        || 0.0,
        |acc, i| {
            let t = &tris[i];
            if t.area3 > 0.0 {
                acc + t.spread(s * s)
            } else {
                acc
            }
        },
        |a, b| a + b,
    );
    let sigmas: Vec<(f64, f64)> = (tris.iter())
        .filter(|t| t.area3 > 0.0)
        .map(|t| (t.sigma, t.area3))
        .collect();
    Stretch {
        l2: (sums.l2 / sums.area3).sqrt() * s,
        p99: percentile(&sigmas, &[0.99])[0] * s,
        worst: sums.worst * s,
        angle: sums.angle / sums.area3,
        area_spread: spread / sums.area3,
    }
}

/// The smallest value whose share of the weight, counted from the smallest, reaches each of
/// `shares`, for values not below 0.
fn percentile(values: &[(f64, f64)], shares: &[f64]) -> Vec<f64> {
    let mut keyed: Vec<(u64, u64)> = (values.iter())
        .map(|&(v, w)| (v.to_bits(), w.to_bits()))
        .collect();
    gs_par::sort(&mut keyed);
    let total: f64 = (keyed.iter()).map(|&(_, w)| f64::from_bits(w)).sum();
    (shares.iter())
        .map(|&p| {
            let mut sum = 0.0;
            for &(v, w) in &keyed {
                sum += f64::from_bits(w);
                if sum >= p * total {
                    return f64::from_bits(v);
                }
            }
            keyed.last().map_or(0.0, |&(v, _)| f64::from_bits(v))
        })
        .collect()
}

/// Each chart's surface area and UV area, signed when `signed`.
fn per_chart(layout: &Layout, signed: bool) -> Vec<(f64, f64)> {
    let count = layout.chart.iter().max().map_or(0, |&c| c as usize + 1);
    let mut out = vec![(0.0, 0.0); count];
    for f in 0..layout.surface.faces.len() {
        let c = layout.chart[f] as usize;
        for t in layout_triangles(layout, f) {
            out[c].0 += t.area3;
            out[c].1 += if signed { t.area2 } else { t.area2.abs() };
        }
    }
    out
}

/// The layout's flipped triangles and mirrored charts.
pub fn flips(layout: &Layout) -> Flips {
    let charts = per_chart(layout, true);
    let flipped = gs_par::reduce(
        layout.surface.faces.len(),
        || 0,
        |n, f| {
            let total = charts[layout.chart[f] as usize].1;
            n + layout_triangles(layout, f)
                .filter(|t| t.area2 * total <= 0.0)
                .count()
        },
        |a, b| a + b,
    );
    let mirrored = charts.iter().filter(|c| c.1 < 0.0).count();
    Flips { flipped, mirrored }
}

/// The layout's texel density.
pub fn density(layout: &Layout) -> Density {
    let charts: Vec<(f64, f64)> = (per_chart(layout, false).into_iter())
        .filter(|c| c.0 > 0.0)
        .map(|(a3, a2)| ((a2 / a3).sqrt(), a3))
        .collect();
    let p = percentile(&charts, &[0.05, 0.95]);
    Density {
        p5: p[0],
        p95: p[1],
    }
}

/// No chart.
const EMPTY: u32 = u32::MAX;

/// The layout on a `size` by `size` raster of the unit square.
pub fn cover(layout: &Layout, size: u32) -> Cover {
    let n = size as usize;
    let mut hits = vec![0_u8; n * n];
    let mut label = vec![EMPTY; n * n];
    let mut shared = false;
    for f in 0..layout.surface.faces.len() {
        let face = layout.surface.faces[f];
        let points = layout.surface.points;
        let uv = layout.uvs[f];
        for t in face_triangles(face, |k| points[face[k] as usize]) {
            raster(t.map(|k| uv[k].map(f64::from)), size, &mut |p| {
                hits[p] = hits[p].saturating_add(1);
                if label[p] == EMPTY {
                    label[p] = layout.chart[f];
                } else if label[p] != layout.chart[f] {
                    shared = true;
                }
            });
        }
    }
    let used = hits.iter().filter(|&&h| h > 0).count();
    let overlap = hits.iter().filter(|&&h| h > 1).count();
    let first = label.iter().find(|&&l| l != EMPTY);
    let two = first.is_some_and(|&a| label.iter().any(|&l| l != EMPTY && l != a));
    let gap = if shared {
        Some(0)
    } else if two {
        Some(narrowest(&mut label, n))
    } else {
        None
    };
    Cover {
        used: used as f64 / (n * n) as f64,
        overlap,
        gap,
    }
}

/// Calls `hit` with each pixel whose centre is inside triangle `q`, edges by the top-left rule.
fn raster(mut q: [[f64; 2]; 3], size: u32, hit: &mut impl FnMut(usize)) {
    let area =
        (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[2][0] - q[0][0]) * (q[1][1] - q[0][1]);
    if area == 0.0 {
        return;
    }
    if area < 0.0 {
        q.swap(1, 2);
    }
    let s = f64::from(size);
    let span = |i: usize| {
        let lo = q.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min);
        let hi = q.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max);
        let first = (lo * s - 0.5).ceil().max(0.0) as usize;
        let last = ((hi * s - 0.5).floor()).min(s - 1.0);
        (
            first,
            if last < 0.0 {
                None
            } else {
                Some(last as usize)
            },
        )
    };
    let ((x0, x1), (y0, y1)) = (span(0), span(1));
    let (Some(x1), Some(y1)) = (x1, y1) else {
        return;
    };
    let inside = |p: [f64; 2]| {
        (0..3).all(|e| {
            let (a, b) = (q[e], q[(e + 1) % 3]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let side = dx * (p[1] - a[1]) - dy * (p[0] - a[0]);
            side > 0.0 || side == 0.0 && (dy < 0.0 || dy == 0.0 && dx > 0.0)
        })
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let centre = [(x as f64 + 0.5) / s, (y as f64 + 0.5) / s];
            if inside(centre) {
                hit(y * size as usize + x);
            }
        }
    }
}

/// The fewest empty pixels between two charts in `label`, by a two-pass chessboard distance
/// transform that carries each pixel's nearest chart.
fn narrowest(label: &mut [u32], n: usize) -> u32 {
    let mut dist: Vec<u32> = label
        .iter()
        .map(|&l| if l == EMPTY { u32::MAX } else { 0 })
        .collect();
    let mut pass = |order: &mut dyn Iterator<Item = usize>, back: &[(isize, isize)]| {
        for p in order {
            let (x, y) = (p % n, p / n);
            for &(dx, dy) in back {
                let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy))
                else {
                    continue;
                };
                if nx >= n || ny >= n {
                    continue;
                }
                let q = ny * n + nx;
                if dist[q] != u32::MAX && dist[q] + 1 < dist[p] {
                    dist[p] = dist[q] + 1;
                    label[p] = label[q];
                }
            }
        }
    };
    pass(&mut (0..n * n), &[(-1, 0), (-1, -1), (0, -1), (1, -1)]);
    pass(&mut (0..n * n).rev(), &[(1, 0), (1, 1), (0, 1), (-1, 1)]);
    let mut best = u32::MAX;
    for p in 0..n * n {
        let (x, y) = (p % n, p / n);
        for (dx, dy) in [(1, 0), (0, 1), (1, 1), (usize::MAX, 1)] {
            let (nx, ny) = (x.wrapping_add(dx), y + dy);
            if nx >= n || ny >= n {
                continue;
            }
            let q = ny * n + nx;
            if label[p] != label[q] {
                best = best.min(dist[p] + dist[q] + 1);
            }
        }
    }
    best.saturating_sub(1)
}
