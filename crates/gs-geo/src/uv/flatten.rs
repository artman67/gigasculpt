//! The flattener (decision 0061 sections 1 and 7): least squares conformal maps (LSCM) per
//! chart, each quad as the two triangles [`face_triangles`] splits it into, solved by
//! [`solve`](super::solve). The two border points farthest apart are held still and the solve
//! starts from [`Chart::project`]; the result is wound face up, scaled to the chart's surface
//! area, and turned so its long axis lies on u.
//!
//! Under symmetry a twin chart is its partner's mirror image `(-u, v)`. A chart across the plane
//! is flattened whole, held at the two points on the plane farthest apart (or, when its border
//! meets the plane at fewer than two points, at the partner pair farthest apart), averaged with
//! its mirror image so it is exactly its own, and kept with the plane on u = 0.

use std::cmp::Ordering;

use gs_protocol::{arity, face_triangles};

use super::cut::{Mirror, upright};
use super::solve::{Matrix, solve};
use super::{Chart, Refusal, Surface, Symmetry};
use crate::math::{cross, dot, sub};

/// No point.
const NONE: u32 = u32::MAX;

/// A chart laid flat.
#[derive(Clone, Debug, PartialEq)]
pub struct Flat {
    /// A UV per point of the chart.
    pub uvs: Vec<[f32; 2]>,
    /// The solve's iterations (a twin's are its partner's).
    pub iterations: usize,
    /// Whether the solve reached its tolerance (as a chart with nothing to solve has).
    pub converged: bool,
}

/// Flattens the charts of one surface.
pub struct Flattener<'a> {
    surface: Surface<'a>,
    mirror: Option<Mirror>,
}

/// How a chart is flattened among the others.
enum Plan {
    /// On its own.
    Own,
    /// On its own and made its own mirror image: each point's partner point.
    Across(Vec<u32>),
    /// As the mirror image of an earlier chart: that chart, and each point's point there.
    Twin(usize, Vec<u32>),
}

/// Two points held still, and where.
struct Pins {
    at: [usize; 2],
    to: [[f64; 2]; 2],
}

/// A triangle of a chart: its points, the edge opposite each corner in a plane frame of its own
/// (as a complex number), and its area.
struct Tri {
    w: [usize; 3],
    s: [[f64; 2]; 3],
    area: f64,
}

impl<'a> Flattener<'a> {
    /// A flattener for `surface`'s charts, which follow `symmetry`.
    ///
    /// # Errors
    /// [`Refusal::Unpaired`] when a point or face has no mirror partner.
    pub fn new(surface: Surface<'a>, symmetry: Option<Symmetry>) -> Result<Self, Refusal> {
        let mirror = symmetry.map(|s| Mirror::new(surface, s)).transpose()?;
        Ok(Flattener { surface, mirror })
    }

    /// `chart` laid flat on its own, a UV per point: what the cutter's check reads through
    /// [`cut_with`](super::cut_with). A chart across the plane comes back its own mirror image.
    pub fn chart(&self, chart: &Chart) -> Flat {
        let across = (self.mirror.as_ref()).and_then(|mirror| {
            let f = *chart.faces.first()? as usize;
            let g = mirror.corner(self.surface.faces, f, 0).0 as u32;
            chart.faces.binary_search(&g).ok()?;
            self.images(chart, chart)
        });
        self.flatten(chart, across.as_deref())
    }

    /// Every chart laid flat, in parallel; under symmetry each twin as its partner's mirror
    /// image.
    pub fn charts(&self, charts: &[Chart]) -> Vec<Flat> {
        let plans = self.plans(charts);
        let mut out = gs_par::map(charts.len(), |c| match &plans[c] {
            Plan::Own => Some(self.flatten(&charts[c], None)),
            Plan::Across(partner) => Some(self.flatten(&charts[c], Some(partner))),
            Plan::Twin(..) => None,
        });
        for (c, plan) in plans.iter().enumerate() {
            if let Plan::Twin(d, image) = plan {
                out[c] = Some(match &out[*d] {
                    Some(flat) => mirrored(flat, image),
                    None => self.flatten(&charts[c], None),
                });
            }
        }
        out.into_iter().flatten().collect()
    }

    /// Each chart's plan: a twin when its lowest face's mirror face is in an earlier chart that
    /// is its mirror image, across when it is in this one and this one is its own.
    fn plans(&self, charts: &[Chart]) -> Vec<Plan> {
        let Some(mirror) = &self.mirror else {
            return charts.iter().map(|_| Plan::Own).collect();
        };
        let mut owner = vec![NONE; self.surface.faces.len()];
        for (c, chart) in charts.iter().enumerate() {
            for &f in &chart.faces {
                owner[f as usize] = c as u32;
            }
        }
        gs_par::map(charts.len(), |c| {
            let Some(&f) = charts[c].faces.first() else {
                return Plan::Own;
            };
            let d = owner[mirror.corner(self.surface.faces, f as usize, 0).0] as usize;
            match d.cmp(&c) {
                Ordering::Equal => {
                    (self.images(&charts[c], &charts[c])).map_or(Plan::Own, Plan::Across)
                }
                Ordering::Less => (self.images(&charts[c], &charts[d]))
                    .map_or(Plan::Own, |image| Plan::Twin(d, image)),
                Ordering::Greater => Plan::Own,
            }
        })
    }

    /// Each point of `from` as the point of `to` at its corners' mirror corners; `None` when
    /// `to` is not `from`'s mirror image, face for face and point for point.
    fn images(&self, from: &Chart, to: &Chart) -> Option<Vec<u32>> {
        let mirror = self.mirror.as_ref()?;
        if from.faces.len() != to.faces.len() || from.points.len() != to.points.len() {
            return None;
        }
        let mut image = vec![NONE; from.points.len()];
        for (i, &f) in from.faces.iter().enumerate() {
            for k in 0..arity(from.corners[i]) {
                let (g, j) = mirror.corner(self.surface.faces, f as usize, k);
                let at = to.faces.binary_search(&(g as u32)).ok()?;
                let slot = &mut image[from.corners[i][k] as usize];
                if *slot != NONE && *slot != to.corners[at][j] {
                    return None;
                }
                *slot = to.corners[at][j];
            }
        }
        Some(image)
    }

    /// `chart` by LSCM; with `across`, each point's partner, made its own mirror image. A chart
    /// with no two points apart or no triangle with area comes back as its projection.
    fn flatten(&self, chart: &Chart, across: Option<&[u32]>) -> Flat {
        let projected = chart.project(self.surface);
        let tris = self.triangles(chart);
        let pins = (!tris.is_empty())
            .then(|| self.pins(chart, across))
            .flatten();
        let Some(pins) = pins else {
            return Flat {
                uvs: projected,
                iterations: 0,
                converged: true,
            };
        };
        let mut x = start(&projected, &pins);
        let (iterations, converged) = lscm(&tris, &pins, &mut x);
        if let Some(partner) = across {
            x = (x.iter().zip(partner))
                .map(|(p, &w)| {
                    let q = x[w as usize];
                    [(p[0] - q[0]) / 2.0, f64::midpoint(p[1], q[1])]
                })
                .collect();
        }
        Flat {
            uvs: finish(&tris, x, across.is_some()),
            iterations,
            converged,
        }
    }

    /// The chart's triangles with area.
    fn triangles(&self, chart: &Chart) -> Vec<Tri> {
        let mut out = Vec::new();
        for (i, &f) in chart.faces.iter().enumerate() {
            let face = self.surface.faces[f as usize];
            let at = |k: usize| self.surface.points[face[k] as usize];
            for t in face_triangles(face, at) {
                let p = t.map(|k| at(k).map(f64::from));
                let (e1, e2) = (sub(p[1], p[0]), sub(p[2], p[0]));
                let n = cross(e1, e2);
                let (l, twice) = (dot(e1, e1).sqrt(), dot(n, n).sqrt());
                if l <= 0.0 || twice <= 0.0 {
                    continue;
                }
                // Corners at 0, (l, 0) and (u, v); s[j] runs along the edge opposite corner j.
                let (u, v) = (dot(e1, e2) / l, twice / l);
                out.push(Tri {
                    w: t.map(|k| chart.corners[i][k] as usize),
                    s: [[u - l, v], [-u, -v], [l, 0.0]],
                    area: twice / 2.0,
                });
            }
        }
        out
    }

    /// The two points held still: the border points farthest apart, at `(-d / 2, 0)` and
    /// `(d / 2, 0)` for their distance `d`; across the plane the two on the plane, at
    /// `(0, -d / 2)` and `(0, d / 2)`, or else the partner pair farthest apart. A chart with no
    /// border reads two far points as its border. `None` when no two are apart.
    fn pins(&self, chart: &Chart, across: Option<&[u32]>) -> Option<Pins> {
        let at = |w: usize| self.surface.points[chart.points[w] as usize].map(f64::from);
        let far = |a: usize, b: usize| {
            let d = sub(at(a), at(b));
            dot(d, d)
        };
        let mut border = border(chart);
        if border.len() < 2 {
            // No border: the point farthest from the first, and the one farthest from that.
            let sweep = |from: usize| {
                (0..chart.points.len()).fold(from, |best, w| {
                    if far(from, w) > far(from, best) {
                        w
                    } else {
                        best
                    }
                })
            };
            let a = sweep(0);
            border = vec![a, sweep(a)];
            border.sort_unstable();
        }
        let pair = |[a, b]: [usize; 2], d: f64, along: usize| {
            let mut to = [[0.0; 2]; 2];
            (to[0][along], to[1][along]) = (-d / 2.0, d / 2.0);
            (d > 0.0).then_some(Pins { at: [a, b], to })
        };
        let Some(partner) = across else {
            let (a, b, d) = farthest(&border, far)?;
            return pair([a, b], d.sqrt(), 0);
        };
        let plane: Vec<usize> = (border.iter().copied())
            .filter(|&w| partner[w] as usize == w)
            .collect();
        if plane.len() >= 2 {
            let (a, b, d) = farthest(&plane, far)?;
            return pair([a, b], d.sqrt(), 1);
        }
        let (a, b, d) = (border.iter().copied()).fold((0, 0, 0.0), |best, w| {
            let p = partner[w] as usize;
            let d = far(w, p);
            if d > best.2 { (w, p, d) } else { best }
        });
        pair([a, b], d.sqrt(), 0)
    }
}

/// The points of `chart` on its border, lowest first: the start of every edge between its points
/// that no face runs the other way.
fn border(chart: &Chart) -> Vec<usize> {
    let mut edges: Vec<u64> = Vec::new();
    for c in &chart.corners {
        let n = arity(*c);
        for k in 0..n {
            edges.push((u64::from(c[k]) << 32) | u64::from(c[(k + 1) % n]));
        }
    }
    gs_par::sort_keys(&mut edges);
    let mut out: Vec<usize> = (edges.iter())
        .filter(|&&e| edges.binary_search(&e.rotate_left(32)).is_err())
        .map(|&e| (e >> 32) as usize)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The two of `points` farthest apart by `far` (the first such pair), and `far` of them.
fn farthest(
    points: &[usize],
    far: impl Fn(usize, usize) -> f64 + Sync,
) -> Option<(usize, usize, f64)> {
    let better = |a: Option<(usize, usize, f64)>, b: Option<(usize, usize, f64)>| match (a, b) {
        (Some(x), Some(y)) if y.2 > x.2 => b,
        (None, _) => b,
        _ => a,
    };
    gs_par::reduce(
        points.len(),
        || None,
        |best, i| {
            (i + 1..points.len()).fold(best, |best, j| {
                better(
                    best,
                    Some((points[i], points[j], far(points[i], points[j]))),
                )
            })
        },
        better,
    )
}

/// The projection moved by the similarity that takes its pins to where they are held, the pins
/// there.
fn start(projected: &[[f32; 2]], pins: &Pins) -> Vec<[f64; 2]> {
    let p = |w: usize| projected[w].map(f64::from);
    let ([a, b], [c, d]) = (pins.at.map(p), pins.to);
    let span = [b[0] - a[0], b[1] - a[1]];
    let length = span[0] * span[0] + span[1] * span[1];
    let ratio = if length > 0.0 {
        times([d[0] - c[0], d[1] - c[1]], [span[0], -span[1]]).map(|x| x / length)
    } else {
        [1.0, 0.0]
    };
    let mut x: Vec<[f64; 2]> = (projected.iter())
        .map(|q| {
            let r = times(ratio, [f64::from(q[0]) - a[0], f64::from(q[1]) - a[1]]);
            [c[0] + r[0], c[1] + r[1]]
        })
        .collect();
    (x[pins.at[0]], x[pins.at[1]]) = (c, d);
    x
}

/// Solves for every point but the pins, from `x`, and writes them into it: the iterations, and
/// whether the solve converged.
fn lscm(tris: &[Tri], pins: &Pins, x: &mut [[f64; 2]]) -> (usize, bool) {
    let mut index = vec![NONE; x.len()];
    let mut free = Vec::new();
    for (w, slot) in index.iter_mut().enumerate() {
        if !pins.at.contains(&w) {
            *slot = free.len() as u32;
            free.push(w);
        }
    }
    let (a, b) = system(tris, &index, x, free.len());
    let solution = solve(&a, &b, free.iter().map(|&w| x[w]).collect());
    for (&w, &z) in free.iter().zip(&solution.x) {
        x[w] = z;
    }
    (solution.iterations, solution.converged)
}

/// The conformal energy's matrix on the `count` free points (`index` their numbers, `NONE` for
/// a pin) and its right-hand side, the pins' pull at their places in `x`. Each triangle adds
/// `conj(s_j) s_k / (4 area)` between its corners `j` and `k`, summed in triangle order.
fn system(tris: &[Tri], index: &[u32], x: &[[f64; 2]], count: usize) -> (Matrix, Vec<[f64; 2]>) {
    let mut keys = Vec::with_capacity(9 * tris.len());
    for t in tris {
        for j in t.w.map(|w| index[w]).into_iter().filter(|&j| j != NONE) {
            for k in t.w.map(|w| index[w]).into_iter().filter(|&k| k != NONE) {
                keys.push((u64::from(j) << 32) | u64::from(k));
            }
        }
    }
    gs_par::sort_keys(&mut keys);
    keys.dedup();
    let mut start = vec![0; count + 1];
    for &key in &keys {
        start[(key >> 32) as usize + 1] += 1;
    }
    for i in 0..count {
        start[i + 1] += start[i];
    }
    let column: Vec<u32> = keys.iter().map(|&key| key as u32).collect();
    let mut value = vec![[0.0; 2]; keys.len()];
    let mut b = vec![[0.0; 2]; count];
    for t in tris {
        let weight = 1.0 / (4.0 * t.area);
        for j in 0..3 {
            let row = index[t.w[j]] as usize;
            if row == NONE as usize {
                continue;
            }
            for k in 0..3 {
                let h = times([t.s[j][0], -t.s[j][1]], t.s[k]).map(|v| v * weight);
                let col = index[t.w[k]];
                if col == NONE {
                    let pull = times(h, x[t.w[k]]);
                    b[row] = [b[row][0] - pull[0], b[row][1] - pull[1]];
                } else if let Ok(at) = column[start[row]..start[row + 1]].binary_search(&col) {
                    let e = start[row] + at;
                    value[e] = [value[e][0] + h[0], value[e][1] + h[1]];
                }
            }
        }
    }
    (
        Matrix {
            start,
            column,
            value,
        },
        b,
    )
}

/// `x` wound face up and scaled to the triangles' surface area; then centred and turned so its
/// long axis lies on u, or, when it is its own mirror image (`across`), only centred along v so
/// the plane stays on u = 0.
fn finish(tris: &[Tri], mut x: Vec<[f64; 2]>, across: bool) -> Vec<[f32; 2]> {
    let (area3, area2) = tris.iter().fold((0.0, 0.0), |(a3, a2), t| {
        let [a, b, c] = t.w.map(|w| x[w]);
        let twice = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        (a3 + t.area, a2 + twice / 2.0)
    });
    let scale = if area2.abs() > 0.0 {
        (area3 / area2.abs()).sqrt()
    } else {
        1.0
    };
    let flip = if area2 < 0.0 { -scale } else { scale };
    for p in &mut x {
        *p = [flip * p[0], scale * p[1]];
    }
    if !across {
        return upright(&x);
    }
    let mean = x.iter().map(|p| p[1]).sum::<f64>() / x.len().max(1) as f64;
    x.iter()
        .map(|p| [p[0] as f32, (p[1] - mean) as f32])
        .collect()
}

/// `flat` mirrored onto the chart whose points are `image` of its own.
fn mirrored(flat: &Flat, image: &[u32]) -> Flat {
    Flat {
        uvs: (image.iter())
            .map(|&w| {
                let [u, v] = flat.uvs[w as usize];
                [-u, v]
            })
            .collect(),
        iterations: flat.iterations,
        converged: flat.converged,
    }
}

/// `a b`, as complex numbers.
fn times(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}
