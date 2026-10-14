//! Stroke Smoothing and spacing: from raw samples to dab positions, each dab carrying the
//! length of stroke it stands for. Every length lands in exactly one dab (or, with Directional,
//! none), so the sum over a stroke is its length whatever the spacing and the sample rate.

use super::{Stroke, StrokeSample};
use crate::brush::{BrushPreset, Pressure, Smoothing, StrokeSpec};
use crate::math::Vec3;

/// Where a dab stroke along `samples` lays its dabs, as `(sample, length)` in the order it lays
/// them, without a mesh: Stroke Smoothing, then spacing, as a stroke runs them. Empty for a
/// preset of another class.
pub fn placed(preset: &BrushPreset, samples: &[StrokeSample]) -> Vec<(StrokeSample, f32)> {
    let StrokeSpec::Dab {
        spacing,
        tap,
        smoothing,
        directional,
    } = preset.stroke
    else {
        return Vec::new();
    };
    let (mut smoother, mut spacer) = (Smoother::default(), Spacer::new(directional));
    let mut out = Vec::new();
    for &s in samples {
        if let Some(p) = smoother.next(s, smoothing) {
            out.extend(spacer.walk(p, spacing, tap, &preset.pressure));
        }
    }
    out.extend(spacer.rest());
    out
}

/// The sample `t` of the way from `a` to `b`.
fn lerp(a: StrokeSample, b: StrokeSample, t: f32) -> StrokeSample {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    StrokeSample {
        pos: a.pos + (b.pos - a.pos) * t,
        pressure: mix(a.pressure, b.pressure),
        tilt: [mix(a.tilt[0], b.tilt[0]), mix(a.tilt[1], b.tilt[1])],
        radius: mix(a.radius, b.radius),
    }
}

/// How many straight pieces each curved span of a curve path is cut into.
const PIECES: usize = 32;

/// The path a curve stroke runs along, through `points` (the press, each point tapped in and
/// the pen, each with whether it is sharp) in order (decision 0055, section 1). Between two
/// sharp points (the ends always are) it is the straight segment, kept as its two ends; a soft
/// point bends it as a centripetal Catmull-Rom spline, and a curved span ending at a sharp
/// point leaves it as a natural spline does. Each curved span is cut into [`PIECES`] straight
/// pieces, pressure, tilt and radius going linearly along it. A point on the same spot as the
/// one before it counts once, sharp if either is, and the two ends always stay. The stroke
/// runs along this path and the window draws it, so the path shown is the path cut.
pub fn curve_path(points: &[(StrokeSample, bool)]) -> Vec<StrokeSample> {
    let mut pts: Vec<(StrokeSample, bool)> = Vec::with_capacity(points.len());
    for (i, &(s, sharp)) in points.iter().enumerate() {
        let (kept, last) = (pts.len(), i + 1 == points.len());
        match pts.last_mut() {
            Some(q) if q.0.pos == s.pos && !last => q.1 |= sharp,
            Some(q) if q.0.pos == s.pos && kept > 1 => *q = (s, true),
            _ => pts.push((s, sharp || i == 0 || last)),
        }
    }
    let mut out = Vec::with_capacity(pts.len() * PIECES);
    for j in 0..pts.len().saturating_sub(1) {
        let ((a, sa), (b, sb)) = (pts[j], pts[j + 1]);
        out.push(a);
        // Per unit of the span's own parameter; an end or a sharp point has no neighbour past it.
        let d = gap(a.pos, b.pos);
        let ta = (!sa).then(|| tangent([pts[j - 1].0.pos, a.pos, b.pos]) * d);
        let tb = (!sb).then(|| tangent([a.pos, b.pos, pts[j + 2].0.pos]) * d);
        // The natural end: no bend where the span meets a sharp point.
        let natural = |m: Vec3| (b.pos - a.pos) * 1.5 - m * 0.5;
        let (ma, mb) = match (ta, tb) {
            (Some(ma), Some(mb)) => (ma, mb),
            (Some(ma), None) => (ma, natural(ma)),
            (None, Some(mb)) => (natural(mb), mb),
            (None, None) => continue,
        };
        for i in 1..PIECES {
            let s = i as f32 / PIECES as f32;
            let (s2, s3) = (s * s, s * s * s);
            let pos = a.pos * (2.0 * s3 - 3.0 * s2 + 1.0)
                + ma * (s3 - 2.0 * s2 + s)
                + b.pos * (3.0 * s2 - 2.0 * s3)
                + mb * (s3 - s2);
            out.push(StrokeSample {
                pos,
                ..lerp(a, b, s)
            });
        }
    }
    out.extend(pts.last().map(|q| q.0));
    out
}

/// The knot gap between two points of a centripetal spline: the square root of their distance.
fn gap(a: Vec3, b: Vec3) -> f32 {
    (b - a).len2().sqrt().sqrt()
}

/// The Catmull-Rom tangent at a soft point `q[1]` between its neighbours, per unit of the
/// centripetal knots (Yuksel, Schaefer and Keyser 2011).
fn tangent(q: [Vec3; 3]) -> Vec3 {
    let (d0, d1) = (gap(q[0], q[1]), gap(q[1], q[2]));
    (q[1] - q[0]) * (1.0 / d0) - (q[2] - q[0]) * (1.0 / (d0 + d1)) + (q[2] - q[1]) * (1.0 / d1)
}

/// A curve stroke's points as it is drawn, and Space (decision 0055).
#[derive(Default)]
pub(super) struct Points {
    /// Each point tapped in: the held sample it sits on, and whether it is sharp.
    marks: Vec<(usize, bool)>,
    /// The samples fed now move the path instead of extending it (Space held).
    moving: bool,
    /// The pen's latest sample, which a moving sample's step is measured from.
    pen: Option<Vec3>,
}

impl Stroke {
    /// Space for what comes next: while on, each sample fed moves the whole path (the press,
    /// the points and the pen), or Drag Rect's rectangle, by the pen's step instead of
    /// extending it. Only a stroke that moves ([`StrokeSpec::moves`]) does.
    pub fn set_moving(&mut self, space: bool) {
        self.points.moving = space && self.preset.stroke.moves();
    }

    /// The pen's step to `pos` while Space moves what is `drawn`, with the smoother's point
    /// moved by it; `None` while it does not, so `pos` draws on.
    pub(super) fn space_step(&mut self, pos: Vec3, drawn: bool) -> Option<Vec3> {
        let pen = self.points.pen.replace(pos)?;
        if !(self.points.moving && drawn) {
            return None;
        }
        let step = pos - pen;
        if let Some(at) = &mut self.smoother.at {
            at.pos += step;
        }
        Some(step)
    }

    /// An Alt tap: the latest sample held becomes a soft point the path bends through. Nothing
    /// if it already is one, if nothing is held yet, or for a stroke that takes no points.
    pub fn point(&mut self) {
        let Some(at) = self.held.len().checked_sub(1) else {
            return;
        };
        if self.preset.stroke.takes_points() && self.points.marks.last().map(|m| m.0) != Some(at) {
            self.points.marks.push((at, false));
        }
    }

    /// A double tap: the latest point becomes sharp, a corner of the path.
    pub fn corner(&mut self) {
        if let Some(m) = self.points.marks.last_mut() {
            m.1 = true;
        }
    }

    /// The points the stroke's path runs through so far, each with whether it is sharp: the
    /// press, each point tapped in, then the pen (the latest sample held). Empty before the
    /// first sample; [`curve_path`] makes the path.
    pub fn path(&self) -> Vec<(StrokeSample, bool)> {
        let Some(last) = self.held.len().checked_sub(1) else {
            return Vec::new();
        };
        let inner = (self.points.marks.iter())
            .filter(|m| (1..last).contains(&m.0))
            .map(|&(i, sharp)| (self.held[i], sharp));
        let pen = (last > 0).then(|| (self.held[last], true));
        std::iter::once((self.held[0], true))
            .chain(inner)
            .chain(pen)
            .collect()
    }

    /// Holds a curve or gesture stroke's samples, through Stroke Smoothing. A moving sample
    /// (Space) is not held: everything held, and the smoother's point, moves by its step.
    pub(super) fn hold(&mut self, samples: &[StrokeSample], k: Smoothing) {
        for &s in samples {
            if let Some(step) = self.space_step(s.pos, !self.held.is_empty()) {
                for h in &mut self.held {
                    h.pos += step;
                }
            } else {
                let pts = self.smoothed(&[s], k);
                self.held.extend(pts);
            }
        }
    }
}

/// The dab radius of a sample: its radius through the size pressure curve.
pub(super) fn radius(s: StrokeSample, pr: &Pressure) -> f32 {
    s.radius * pr.size.eval(s.pressure)
}

/// The paint point of Stroke Smoothing.
#[derive(Default)]
pub(super) struct Smoother {
    at: Option<StrokeSample>,
}

impl Smoother {
    /// The paint point after the pointer moved to `s`: it hangs on a string `radius` brush radii
    /// long and keeps `factor` of each move back. `None` while the pointer stays within the
    /// string (which also drops repeated samples).
    pub fn next(&mut self, s: StrokeSample, k: Smoothing) -> Option<StrokeSample> {
        let Some(at) = self.at else {
            self.at = Some(s);
            return Some(s);
        };
        let d = s.pos - at.pos;
        let (len, rope) = (d.len2().sqrt(), k.radius * s.radius);
        if len <= rope {
            return None;
        }
        // Measured back from the pointer, so with smoothing off the point is the sample exactly.
        let out = StrokeSample {
            pos: s.pos - d * (1.0 - (len - rope) / len * (1.0 - k.factor)),
            ..s
        };
        self.at = Some(out);
        Some(out)
    }
}

/// The most dabs one pen sample may place. A long jump with a tiny brush gets its cells
/// stretched to fit, so a glitching pen cannot stall a stroke; the length they stand for is
/// unchanged.
pub const MOST_DABS_PER_SAMPLE: usize = 1000;

/// Places dabs along the smoothed path. The path is cut into cells one spacing step long (the
/// step read at each cell's start), the first half a step: its dab sits on the press and every
/// later cell's in its middle, so dabs land a whole number of steps from the press (decision
/// 0036 section 4). Each stands for its cell's length, and so does the unfinished cell's, in
/// its middle, when the stroke ends. Dabs so sit in the middle of the length they stand for
/// (the first a quarter step off it), which keeps the result the same at any spacing. With
/// Directional, a cell whose end is no farther from the press than its start lays nothing.
#[derive(Default)]
pub(super) struct Spacer {
    last: Option<StrokeSample>,
    /// Where the open cell starts, and how much path it holds so far.
    start: Option<StrokeSample>,
    carry: f32,
    /// Where the stroke pressed.
    press: Option<Vec3>,
    /// The first cell, the half step from the press, is done.
    past_press: bool,
    directional: bool,
}

impl Spacer {
    /// A spacer for a stroke with Directional on or off.
    pub fn new(directional: bool) -> Spacer {
        Spacer {
            directional,
            ..Spacer::default()
        }
    }

    /// Whether a cell from `a` to `b` lays its dab: always, unless Directional is on and `b` is
    /// no farther from the press than `a`.
    fn counts(&self, a: StrokeSample, b: StrokeSample) -> bool {
        let from = |s: StrokeSample| self.press.map_or(0.0, |p| (s.pos - p).len2());
        !self.directional || from(b) > from(a)
    }

    /// Dabs on the way from the last point to `to`, as `(sample, length)`. The first point gets
    /// the press dab, standing for `tap` radii.
    pub fn walk(
        &mut self,
        to: StrokeSample,
        spacing: f32,
        tap: f32,
        pr: &Pressure,
    ) -> Vec<(StrokeSample, f32)> {
        let (Some(from), Some(mut start)) = (self.last.replace(to), self.start) else {
            (self.start, self.press) = (Some(to), Some(to.pos));
            return vec![(to, tap * radius(to, pr))];
        };
        let seg = (to.pos - from.pos).len2().sqrt();
        // No cell is shorter than this, so one sample places at most the most dabs allowed.
        let shortest = (self.carry + seg) / MOST_DABS_PER_SAMPLE as f32;
        // At least 1% of the diameter, even where the spacing curve falls to zero.
        let step = |s: StrokeSample| {
            let fraction = (spacing * pr.spacing.eval(s.pressure)).max(0.01);
            (fraction * 2.0 * radius(s, pr)).max(shortest)
        };
        // Steps change linearly between samples, so none is shorter than the shorter end's:
        // that bounds the cells (the half first cell adds at most one, the loop's last turn),
        // and the loop cannot run on whatever its arithmetic does.
        let cells = ((self.carry + seg) / step(start).min(step(to))) as usize;
        let (mut t, mut out) = (0.0, Vec::new());
        for _ in 0..=cells.min(MOST_DABS_PER_SAMPLE) {
            let first = !self.past_press;
            let h = step(start) * if first { 0.5 } else { 1.0 };
            let need = (h - self.carry).max(0.0);
            let end = t + need;
            let fits = end <= seg;
            if !fits {
                break;
            }
            let at = lerp(from, to, (end / seg).min(1.0));
            if self.counts(start, at) {
                let dab = if first { start } else { lerp(start, at, 0.5) };
                out.push((dab, h.max(self.carry)));
            }
            (start, t, self.carry, self.past_press) = (at, end, 0.0, true);
        }
        self.carry += seg - t;
        self.start = Some(start);
        out
    }

    /// The dab of the unfinished cell, if it holds any length.
    pub fn rest(&mut self) -> Option<(StrokeSample, f32)> {
        let carry = std::mem::take(&mut self.carry);
        let (start, last) = (self.start?, self.last?);
        (carry > 0.0 && self.counts(start, last)).then(|| (lerp(start, last, 0.5), carry))
    }
}
