//! The curve (decision 0041, amended 2026-10-12, step DF3): the part along an axis laid along
//! a biarc spline through the call's points. Each span between two points is two circular arcs
//! meeting with one tangent, so a weight can turn every arc by part of its angle and keep its
//! length. A frame rides along the curve, turning with each arc, and carries the part's
//! cross-section, turned by the twist and scaled by the scale there. Points on the axis line in
//! order from the box's low face are that line, and then only the twist and scale move anything.

use super::{Ready, sinc};
use crate::math::{cross, dot, sub};

/// A rotation of frame coordinates, by rows.
type Turn = [[f64; 3]; 3];

/// The curve: the box's span along `axis` laid along a smooth curve through `points`.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    /// The frame axis the part runs along.
    pub axis: usize,
    /// 2 to 32 points in frame coordinates, in order. `None` for the axis line through the
    /// origin from the box's low face to its high one.
    pub points: Option<Vec<[f32; 3]>>,
    /// The turn about the curve at each point, in radians; `None` for none.
    pub twist: Option<Vec<f32>>,
    /// The cross-section's scale at each point; `None` for 1.
    pub scale: Option<Vec<f32>>,
}

impl Curve {
    /// The curve with its points' lengths mapped by `f`; twist and scale stay.
    #[must_use]
    pub(super) fn scaled(self, f: impl Fn(f32) -> f32) -> Curve {
        let points = (self.points.as_ref()).map(|p| p.iter().map(|q| q.map(&f)).collect());
        Curve { points, ..self }
    }

    /// What is wrong with the curve, if anything: the path of the argument at fault below the
    /// curve's own, and why. Points too few or too many, not finite, equal to the one before or
    /// where the curve turns straight back; a twist or scale without one finite value a point.
    pub fn fault(&self) -> Option<(String, &'static str)> {
        if self.axis > 2 {
            return Some(("axis".into(), "must be x, y or z"));
        }
        if let Some(points) = &self.points {
            if !(2..=32).contains(&points.len()) {
                return Some(("points".into(), "must be 2 to 32 points"));
            }
            let p: Vec<[f64; 3]> = points.iter().map(|q| q.map(f64::from)).collect();
            let at = |i: usize, why| Some((format!("points/{i}"), why));
            for (i, q) in p.iter().enumerate() {
                if !q.iter().all(|v| v.is_finite()) {
                    return at(i, "must be finite");
                }
                if i > 0 && (0..3).all(|k| q[k] - p[i - 1][k] == 0.0) {
                    return at(i, "must not be the point before it");
                }
            }
            for (i, w) in p.windows(3).enumerate() {
                let way = |a, b| unit(sub(b, a)).unwrap_or_default();
                if bisector(way(w[0], w[1]), way(w[1], w[2])).is_none() {
                    return at(i + 1, "must not turn the curve straight back");
                }
            }
        }
        let count = self.points.as_ref().map_or(2, Vec::len);
        for (name, values) in [("twist", &self.twist), ("scale", &self.scale)] {
            let Some(v) = values else { continue };
            if v.len() != count {
                return Some((name.into(), "must give one value a point"));
            }
            if let Some(i) = v.iter().position(|x| !x.is_finite()) {
                return Some((format!("{name}/{i}"), "must be finite"));
            }
        }
        None
    }
}

/// One arc of the spline: its length, its turn in radians, and the unit axis it turns about in
/// the frame riding along the curve (zero for a straight one).
#[derive(Clone, Copy, Debug)]
struct Arc {
    length: f64,
    turn: f64,
    about: [f64; 3],
}

/// The curve made ready on the box.
#[derive(Clone, Debug)]
pub(super) struct Path {
    axis: usize,
    /// The box's low face along the axis, where the part's distance along the curve is 0.
    lo: f64,
    /// The spline's arcs, two a span; none for points on the axis line from the low face.
    arcs: Vec<Arc>,
    /// The distance along the curve to each arc's start, and to the last one's end.
    starts: Vec<f64>,
    /// The distance along the curve to each point.
    knots: Vec<f64>,
    /// The first point less the box's low face on the axis line.
    start: [f64; 3],
    /// The least turn taking the axis to the start tangent: its angle and unit axis.
    first: (f64, [f64; 3]),
    /// Where each arc starts at full weight and the riding frame there, then where the last
    /// one ends.
    table: Vec<([f64; 3], Turn)>,
    twist: Vec<f64>,
    scale: Vec<f64>,
}

impl Path {
    /// The curve `c` on the box `r` holds; `None` when it changes nothing or is at fault.
    pub(super) fn new(r: &Ready, c: &Curve) -> Option<Path> {
        if c.fault().is_some() {
            return None;
        }
        let (axis, lo) = (c.axis, r.lo[c.axis]);
        let e = unit_axis(axis);
        let points: Vec<[f64; 3]> = match &c.points {
            Some(p) => p.iter().map(|q| q.map(f64::from)).collect(),
            None => vec![times(e, lo), times(e, r.hi[axis])],
        };
        let values = |v: &Option<Vec<f32>>| -> Vec<f64> {
            v.iter().flatten().map(|&x| f64::from(x)).collect()
        };
        let mut path = Path {
            axis,
            lo,
            arcs: Vec::new(),
            starts: vec![0.0],
            knots: points.iter().map(|p| p[axis] - lo).collect(),
            start: sub(points[0], times(e, lo)),
            first: (0.0, e),
            table: Vec::new(),
            twist: values(&c.twist),
            scale: values(&c.scale),
        };
        // Exact comparisons, written against zero: the line itself, read as no turn at all.
        let on_line = points[0][axis] - lo == 0.0
            && (points.iter()).all(|p| (0..3).all(|k| k == axis || p[k] == 0.0))
            && points.windows(2).all(|w| w[1][axis] >= w[0][axis]);
        let still = |t: &f64, rest: f64| t - rest == 0.0;
        if !on_line {
            path.spline(&points)?;
        } else if path.twist.iter().all(|t| still(t, 0.0))
            && path.scale.iter().all(|s| still(s, 1.0))
        {
            return None;
        }
        Some(path)
    }

    /// The arcs through `points`, each given in the frame riding along the curve, and the table
    /// of their starts at full weight.
    fn spline(&mut self, points: &[[f64; 3]]) -> Option<()> {
        let tangents = tangents(points)?;
        self.first = least(unit_axis(self.axis), tangents[0], self.axis);
        let mut at = self.origin(1.0);
        for (p, t) in points.windows(2).zip(tangents.windows(2)) {
            let (join, toward) = junction(p[0], t[0], p[1], t[1])?;
            for (from, tangent, to) in [(p[0], t[0], join), (join, toward, p[1])] {
                let (length, turn, about) = arc(from, tangent, to);
                if !(length > 0.0 && length.is_finite()) {
                    return None;
                }
                // The axis in the riding frame: the frame's transpose undoes it.
                let about = [0, 1, 2].map(|k| (0..3).map(|j| at.1[j][k] * about[j]).sum::<f64>());
                let arc = Arc {
                    length,
                    turn,
                    about,
                };
                self.table.push(at);
                at = self.step(at, &arc, 1.0, length);
                self.arcs.push(arc);
                self.starts
                    .push(self.starts[self.starts.len() - 1] + length);
            }
        }
        self.table.push(at);
        self.knots = (0..points.len()).map(|i| self.starts[2 * i]).collect();
        Some(())
    }

    /// The curve's start at weight `w`: `w` of the way from the box's low face on the axis line
    /// to the first point, with the frame turned by `w` of the least turn.
    fn origin(&self, w: f64) -> ([f64; 3], Turn) {
        let e = unit_axis(self.axis);
        let (angle, about) = self.first;
        (
            add(times(e, self.lo), times(self.start, w)),
            rotation(about, w * angle),
        )
    }

    /// From `at` on arc `arc`, turning `w` of its angle, `sigma` along it: where that is and
    /// the riding frame there. The chord in the frame is the start tangent turned by half the
    /// angle so far, times `sigma sinc(half)`.
    fn step(
        &self,
        (pos, frame): ([f64; 3], Turn),
        arc: &Arc,
        w: f64,
        sigma: f64,
    ) -> ([f64; 3], Turn) {
        let half = w * arc.turn * (sigma / arc.length) / 2.0;
        let (sin, cos) = libm::sincos(half);
        let e = unit_axis(self.axis);
        let across = cross(arc.about, e);
        // `sinc(half)` from the sine in hand, by the rule of [`sinc`].
        let length = sigma
            * if half.abs() < 1e-4 {
                1.0 - half * half / 6.0
            } else {
                sin / half
            };
        let chord = [0, 1, 2].map(|k| length * (cos * e[k] + sin * across[k]));
        let turn = turned(arc.about, 2.0 * sin * cos, 2.0 * sin * sin);
        (add(pos, apply(&frame, chord)), times_turn(&frame, &turn))
    }

    /// The point at distance `x` along the curve at weight `w` and the riding frame there:
    /// before the start straight back along the start tangent, past the end straight on.
    fn locate(&self, x: f64, w: f64) -> ([f64; 3], Turn) {
        let last = self.arcs.len();
        let on = |(pos, frame): ([f64; 3], Turn), dx: f64| {
            let tangent = frame.map(|row| row[self.axis]);
            (add(pos, times(tangent, dx)), frame)
        };
        if w.to_bits() == 1f64.to_bits() {
            return match self.starts.partition_point(|&a| a <= x) {
                0 => on(self.table[0], x),
                k if k > last => on(self.table[last], x - self.starts[last]),
                k => self.step(
                    self.table[k - 1],
                    &self.arcs[k - 1],
                    1.0,
                    x - self.starts[k - 1],
                ),
            };
        }
        let mut at = self.origin(w);
        if x < 0.0 {
            return on(at, x);
        }
        for (i, arc) in self.arcs.iter().enumerate() {
            if x < self.starts[i + 1] {
                return self.step(at, arc, w, x - self.starts[i]);
            }
            at = self.step(at, arc, w, arc.length);
        }
        on(at, x - self.starts[last])
    }
}

/// The path as the vertex stage reads it (gs-render `shaders/deform_tables.wgsl` `curve`, `DF9b`):
/// the low face, the arcs and the points; the start with the first turn's angle, then its axis;
/// for each arc and then the end, its axis and turn, where it starts and its length, and the
/// riding frame by rows, the first row ending in the distance to it; then each point's distance,
/// twist and scale. Zeros for no path.
pub(super) fn table(path: Option<&Path>) -> Vec<[f32; 4]> {
    let Some(p) = path else {
        return vec![[0.0; 4]];
    };
    let row = |v: [f64; 3], w: f64| [v[0], v[1], v[2], w].map(|x| x as f32);
    let (angle, about) = p.first;
    let mut out = vec![
        row([p.lo, p.arcs.len() as f64, p.knots.len() as f64], 0.0),
        row(p.start, angle),
        row(about, 0.0),
    ];
    let end = Arc {
        length: 0.0,
        turn: 0.0,
        about: [0.0; 3],
    };
    // The table holds one more than the arcs, or none for the line.
    for (i, (pos, frame)) in p.table.iter().enumerate() {
        let arc = p.arcs.get(i).unwrap_or(&end);
        out.extend([row(arc.about, arc.turn), row(*pos, arc.length)]);
        out.extend([
            row(frame[0], p.starts[i]),
            row(frame[1], 0.0),
            row(frame[2], 0.0),
        ]);
    }
    for (i, &knot) in p.knots.iter().enumerate() {
        let at = |v: &[f64], none| v.get(i).copied().unwrap_or(none);
        out.push(row([knot, at(&p.twist, 0.0), at(&p.scale, 1.0)], 0.0));
    }
    out
}

/// The curve's change of frame coordinates `c` at weight `w`: the cross-section turned by `w`
/// of the twist and scaled by `1 + w (scale - 1)`, carried by the riding frame to the curve's
/// point at the part's distance along it.
pub(super) fn curve(r: &Ready, c: [f64; 3], w: f64) -> [f64; 3] {
    let Some(path) = &r.path else {
        return [0.0; 3];
    };
    if w == 0.0 {
        return [0.0; 3];
    }
    let axis = path.axis;
    let x = c[axis] - path.lo;
    let twist = w * along(&path.knots, &path.twist, x, 0.0);
    let scale = 1.0 + w * (along(&path.knots, &path.scale, x, 1.0) - 1.0);
    let mut section = c;
    section[axis] = 0.0;
    let d = shaped(section, axis, twist, scale);
    if path.arcs.is_empty() {
        return d;
    }
    let (pos, frame) = path.locate(x, w);
    let carried = apply(&frame, add(section, d));
    [0, 1, 2].map(|k| pos[k] + carried[k] - c[k])
}

/// The change of cross-section point `b` turned by `twist` about `axis` and scaled by `scale`,
/// in half-angle form, so no twist and a scale of 1 change nothing.
fn shaped(b: [f64; 3], axis: usize, twist: f64, scale: f64) -> [f64; 3] {
    let (p, q) = ((axis + 1) % 3, (axis + 2) % 3);
    let (sin, half) = (libm::sin(twist), libm::sin(twist / 2.0));
    let turn_p = -2.0 * b[p] * half * half - b[q] * sin;
    let turn_q = b[p] * sin - 2.0 * b[q] * half * half;
    let mut d = [0.0; 3];
    d[p] = scale * turn_p + (scale - 1.0) * b[p];
    d[q] = scale * turn_q + (scale - 1.0) * b[q];
    d
}

/// `values` at distance `x`, linear between the knots and held past the ends; `none` when
/// there are no values.
fn along(knots: &[f64], values: &[f64], x: f64, none: f64) -> f64 {
    if values.is_empty() {
        return none;
    }
    match knots.partition_point(|&k| k <= x) {
        0 => values[0],
        i if i == knots.len() => values[i - 1],
        i => {
            let t = (x - knots[i - 1]) / (knots[i] - knots[i - 1]);
            values[i - 1] + t * (values[i] - values[i - 1])
        }
    }
}

/// The tangent at each point: at an inner one the bisector of the directions to it and from
/// it, at an end the next one mirrored across the end span; on two points, the span's.
fn tangents(points: &[[f64; 3]]) -> Option<Vec<[f64; 3]>> {
    let ways: Vec<[f64; 3]> = (points.windows(2))
        .map(|w| unit(sub(w[1], w[0])))
        .collect::<Option<_>>()?;
    let n = points.len();
    let mut t = vec![ways[0]; n];
    for i in 1..n - 1 {
        t[i] = bisector(ways[i - 1], ways[i])?;
    }
    if n > 2 {
        t[0] = mirror(t[1], ways[0]);
        t[n - 1] = mirror(t[n - 2], ways[n - 2]);
    }
    Some(t)
}

/// The unit bisector of unit directions `a` and `b`; `None` when they are within about a
/// millionth of a radian of opposite.
fn bisector(a: [f64; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let s = add(a, b);
    (norm(s) >= 1e-6).then(|| times(s, 1.0 / norm(s)))
}

/// Unit `t` mirrored across unit direction `u`.
fn mirror(t: [f64; 3], u: [f64; 3]) -> [f64; 3] {
    let d = 2.0 * dot(t, u);
    [0, 1, 2].map(|k| d * u[k] - t[k])
}

/// The equal-handle biarc's junction from `p0` along `t0` to `p1` along `t1`, and the tangent
/// there: the handle `d` makes the line from `p0 + d t0` to `p1 - d t1` `2 d` long, and the
/// junction is its middle.
fn junction(
    p0: [f64; 3],
    t0: [f64; 3],
    p1: [f64; 3],
    t1: [f64; 3],
) -> Option<([f64; 3], [f64; 3])> {
    let (v, t) = (sub(p1, p0), add(t0, t1));
    let a = 2.0 * (dot(t0, t1) - 1.0);
    let (vv, vt) = (dot(v, v), dot(v, t));
    let d = vv / (vt + (vt * vt - a * vv).max(0.0).sqrt());
    if !(d.is_finite() && d > 0.0) {
        return None;
    }
    let (q0, q1) = (add(p0, times(t0, d)), sub(p1, times(t1, d)));
    Some((times(add(q0, q1), 0.5), unit(sub(q1, q0))?))
}

/// The circular arc from `a` leaving along unit `tangent` to `b`: its length, its turn (twice
/// the angle from the tangent to the chord) and the unit axis it turns about.
fn arc(a: [f64; 3], tangent: [f64; 3], b: [f64; 3]) -> (f64, f64, [f64; 3]) {
    let c = sub(b, a);
    let across = cross(tangent, c);
    let s = norm(across);
    let half = libm::atan2(s, dot(tangent, c));
    let about = if s > 0.0 {
        times(across, 1.0 / s)
    } else {
        [0.0; 3]
    };
    (norm(c) / sinc(half), 2.0 * half, about)
}

/// The least turn taking unit `e` to unit `t`: its angle and unit axis, about the next axis
/// after `axis` when they are opposite.
fn least(e: [f64; 3], t: [f64; 3], axis: usize) -> (f64, [f64; 3]) {
    let across = cross(e, t);
    let s = norm(across);
    let angle = libm::atan2(s, dot(e, t));
    if s > 0.0 {
        (angle, times(across, 1.0 / s))
    } else {
        (angle, unit_axis((axis + 1) % 3))
    }
}

/// The turn by `angle` about unit `about`.
fn rotation(about: [f64; 3], angle: f64) -> Turn {
    let (sin, cos) = libm::sincos(angle / 2.0);
    turned(about, 2.0 * sin * cos, 2.0 * sin * sin)
}

/// The turn about unit `n` whose angle has sine `sin` and `1 - cos` of `versed`: Rodrigues'
/// `I + sin K + versed K²`, `K` the cross product with `n`.
fn turned([x, y, z]: [f64; 3], sin: f64, versed: f64) -> Turn {
    let v = versed;
    [
        [
            1.0 - v * (y * y + z * z),
            v * x * y - sin * z,
            v * x * z + sin * y,
        ],
        [
            v * x * y + sin * z,
            1.0 - v * (x * x + z * z),
            v * y * z - sin * x,
        ],
        [
            v * x * z - sin * y,
            v * y * z + sin * x,
            1.0 - v * (x * x + y * y),
        ],
    ]
}

fn apply(m: &Turn, v: [f64; 3]) -> [f64; 3] {
    m.map(|row| dot(row, v))
}

fn times_turn(a: &Turn, b: &Turn) -> Turn {
    a.map(|row| [0, 1, 2].map(|j| (0..3).map(|k| row[k] * b[k][j]).sum::<f64>()))
}

fn unit_axis(axis: usize) -> [f64; 3] {
    let mut e = [0.0; 3];
    e[axis] = 1.0;
    e
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let n = norm(v);
    (n > 0.0 && n.is_finite()).then(|| times(v, 1.0 / n))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|k| a[k] + b[k])
}

fn times(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|x| x * s)
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
