//! Deformers (decision 0041, amended 2026-10-12): bend, fold, twist, taper, shear, clip,
//! spherize, noise, the cage and the curve, each a map of a point's place to its new place,
//! run by [`MeshStore::transform`](crate::store::MeshStore) as `transform.apply`'s other
//! operations are, under the same weights and symmetry. And [`Scope`], the three arguments
//! every operation shares: which frame axes the change keeps, a gradient that fades it along
//! one, and the box the operations measure against.
//!
//! Everything here works in frame coordinates: a point's offset from the frame's origin along
//! the frame's three axes, in `f64`, so the box, every operation's axis and the gradient are
//! the frame's. A weight scales a turning operation's angle and a linear one's amount, formed
//! as one `f64` product before anything else, so a point at weight one half moves exactly as
//! at full weight with half the amount. Every operation's change is exactly zero at a zero
//! amount, so a point that is not moved keeps its bits.

mod bend;
mod clip;
mod curve;
mod lattice;
mod measure;
mod noise;
mod taper;
mod twist;

pub use curve::Curve;
pub use lattice::{Lattice, Offset};

use std::cmp::Ordering;

use crate::math::{Aabb, Quat, Vec3};
use crate::transform::{Op, Transform};

/// One deformer at full weight. `axis` and `toward` are frame axes, 0, 1 and 2 for X, Y and Z,
/// never the same one; angles are radians and lengths scene units.
#[derive(Clone, Debug, PartialEq)]
pub enum Deform {
    /// The box's span along `axis` bent into an arc turning by `angle` toward `toward`, about
    /// the frame's origin clamped into the box; beyond the box the part goes on straight.
    Bend {
        axis: usize,
        toward: usize,
        angle: f32,
    },
    /// Each side of the plane through the origin across `axis` turned rigidly by half `angle`,
    /// both rising toward `toward`; a point on the plane stays.
    Fold {
        axis: usize,
        toward: usize,
        angle: f32,
    },
    /// Each slice across `axis` turned about it, by `angle` over the box's span, the origin's
    /// slice not at all.
    Twist { axis: usize, angle: f32 },
    /// Each slice across `axis` scaled by `1 - amount * u^exponent`, `u` the slice's place
    /// along the box's span from the origin (negative on the other side, the power keeping its
    /// sign). `amount` is -1 to 1, `exponent` above zero.
    Taper {
        axis: usize,
        amount: f32,
        exponent: f32,
    },
    /// Each slice across `axis` moved along `toward` by `amount` times its place along the
    /// box's span from the origin, or by the smoothstep of it.
    Shear {
        axis: usize,
        toward: usize,
        amount: f32,
        smooth: bool,
    },
    /// Every point past the plane `depth` inside the box's top face along `axis` (the bottom
    /// face for a negative depth) put onto it; beyond the box a point moves as the face does.
    Clip { axis: usize, depth: f32 },
    /// Every point moved `amount` of the way to the sphere of `radius` about the origin, over
    /// the kept axes only. `amount` is -1 to 1; the radius is measured when not given.
    Spherize { amount: f32, radius: Option<f32> },
    /// Every point moved by up to `amount` along each axis, a smooth value noise of the point's
    /// place in cells `scale` wide, the same at the same place for the same `seed`.
    Noise { amount: f32, scale: f32, seed: u64 },
    /// A lattice of points across the box, some moved; every point moves as the lattice
    /// around it does (DF3).
    Lattice(Lattice),
    /// The box's span along an axis laid along a curve through points, twisted and scaled
    /// along it (DF3).
    Curve(Curve),
}

impl Deform {
    /// Whether the operation measures against the box.
    pub fn needs_box(&self) -> bool {
        !matches!(
            self,
            Deform::Fold { .. } | Deform::Spherize { .. } | Deform::Noise { .. }
        )
    }

    /// The operation with every length mapped by `f`: shear's amount, clip's depth, spherize's
    /// radius, noise's amount and cell size, the cage's offsets and the curve's points. Angles,
    /// fractions and indices stay.
    #[must_use]
    pub fn scaled(self, f: impl Fn(f32) -> f32) -> Deform {
        match self {
            Deform::Shear {
                axis,
                toward,
                amount,
                smooth,
            } => Deform::Shear {
                axis,
                toward,
                amount: f(amount),
                smooth,
            },
            Deform::Clip { axis, depth } => Deform::Clip {
                axis,
                depth: f(depth),
            },
            Deform::Spherize { amount, radius } => Deform::Spherize {
                amount,
                radius: radius.map(f),
            },
            Deform::Noise {
                amount,
                scale,
                seed,
            } => Deform::Noise {
                amount: f(amount),
                scale: f(scale),
                seed,
            },
            Deform::Lattice(l) => Deform::Lattice(l.scaled(f)),
            Deform::Curve(c) => Deform::Curve(c.scaled(f)),
            other => other,
        }
    }
}

/// A fade of the weight along frame axis `axis`: full where the point's place across the box
/// is `from` or before it, none at `to` and after, linear between. `from` and `to` are
/// fractions of the box, 0 to 1, either way round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub axis: usize,
    pub from: f32,
    pub to: f32,
}

/// The arguments every operation shares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scope {
    /// The frame axes whose part of the change is kept; an axis left out holds the points'
    /// place along it.
    pub axes: [bool; 3],
    pub gradient: Option<Gradient>,
    /// The box in frame coordinates. `None` until measured: then the box around every point
    /// that weighs more than zero.
    pub bounds: Option<Aabb>,
}

impl Default for Scope {
    /// Every axis kept, no gradient, the box measured.
    fn default() -> Scope {
        Scope {
            axes: [true; 3],
            gradient: None,
            bounds: None,
        }
    }
}

impl Scope {
    /// True when the scope changes nothing about a move, turn or scale: every axis kept and
    /// no gradient. Only then can a part's transform hold one.
    pub fn is_plain(&self) -> bool {
        self.axes == [true; 3] && self.gradient.is_none()
    }

    /// The scope with the box's lengths mapped by `f`.
    #[must_use]
    pub fn scaled(self, f: impl Fn(f32) -> f32) -> Scope {
        let map = |v: Vec3| Vec3::new(f(v.x), f(v.y), f(v.z));
        Scope {
            bounds: self.bounds.map(|b| Aabb {
                lo: map(b.lo),
                hi: map(b.hi),
            }),
            ..self
        }
    }
}

/// A transform's deformer and scope, made ready for points: the frame's axes in `f64`, the box
/// as numbers.
#[derive(Clone, Debug)]
pub(crate) struct Ready {
    op: Option<Deform>,
    /// The frame's X, Y and Z as scene vectors.
    axes: [[f64; 3]; 3],
    lo: [f64; 3],
    hi: [f64; 3],
    keep: [bool; 3],
    gradient: Option<Gradient>,
    /// How far the frame's origin is from the scene's: offsets are this rough.
    far: f64,
    /// The cage's offsets on its padded lattice, when it moves anything.
    cage: Option<lattice::Grid>,
    /// The curve on the box, when it changes anything.
    path: Option<curve::Path>,
}

impl Ready {
    pub(crate) fn new(t: &Transform) -> Ready {
        let op = match &t.op {
            Op::Deform(d) => Some(d.clone()),
            _ => None,
        };
        let o = t.frame.origin.to_array().map(f64::from);
        let b = t.scope.bounds.unwrap_or(Aabb {
            lo: Vec3::ZERO,
            hi: Vec3::ZERO,
        });
        let mut r = Ready {
            op,
            axes: frame_axes(t.frame.axes),
            lo: b.lo.to_array().map(f64::from),
            hi: b.hi.to_array().map(f64::from),
            keep: t.scope.axes,
            gradient: t.scope.gradient,
            far: (o[0] * o[0] + o[1] * o[1] + o[2] * o[2]).sqrt(),
            cage: None,
            path: None,
        };
        match &r.op {
            Some(Deform::Lattice(l)) => r.cage = lattice::Grid::new(l),
            Some(Deform::Curve(c)) => r.path = curve::Path::new(&r, c),
            _ => {}
        }
        r
    }

    /// The frame coordinates of offset `v`.
    pub(crate) fn coords(&self, v: Vec3) -> [f64; 3] {
        let v = v.to_array().map(f64::from);
        self.axes.map(|a| a[0] * v[0] + a[1] * v[1] + a[2] * v[2])
    }

    /// The scene offset of frame coordinates `c`.
    fn world(&self, c: [f64; 3]) -> Vec3 {
        let a = &self.axes;
        let at = |j: usize| (a[0][j] * c[0] + a[1][j] * c[1] + a[2][j] * c[2]) as f32;
        Vec3::new(at(0), at(1), at(2))
    }

    /// Weight `w` of the point at offset `v` faded by the gradient; `w` itself without one.
    pub(crate) fn ramp(&self, v: Vec3, w: f32) -> f32 {
        let Some(g) = self.gradient else {
            return w;
        };
        let (lo, hi) = (self.lo[g.axis], self.hi[g.axis]);
        let f = if hi > lo {
            (self.coords(v)[g.axis] - lo) / (hi - lo)
        } else {
            0.0
        };
        let (from, to) = (f64::from(g.from), f64::from(g.to));
        let span = to - from;
        let t = if span == 0.0 {
            f64::from(u8::from(f >= from))
        } else {
            ((f - from) / span).clamp(0.0, 1.0)
        };
        (f64::from(w) * (1.0 - t)) as f32
    }

    /// Scene change `d` with the parts along the axes left out taken away; `d` itself when
    /// every axis is kept.
    pub(crate) fn keep(&self, d: Vec3) -> Vec3 {
        if self.keep == [true; 3] {
            return d;
        }
        self.world(self.held(self.coords(d)))
    }

    fn held(&self, mut c: [f64; 3]) -> [f64; 3] {
        for (c, keep) in c.iter_mut().zip(self.keep) {
            if !keep {
                *c = 0.0;
            }
        }
        c
    }

    /// The deformer's change of the point at offset `v` at weight `w`.
    pub(crate) fn change(&self, v: Vec3, w: f32) -> Vec3 {
        let Some(op) = &self.op else {
            return Vec3::ZERO;
        };
        let (c, w) = (self.coords(v), f64::from(w));
        let at = |a: f32| f64::from(a) * w;
        let d = match *op {
            Deform::Bend {
                axis,
                toward,
                angle,
            } => bend::bend(self, c, [axis, toward], at(angle)),
            Deform::Fold {
                axis,
                toward,
                angle,
            } => bend::fold(self, c, [axis, toward], at(angle)),
            Deform::Twist { axis, angle } => twist::twist(self, c, axis, at(angle)),
            Deform::Taper {
                axis,
                amount,
                exponent,
            } => taper::taper(self, c, axis, at(amount), exponent),
            Deform::Shear {
                axis,
                toward,
                amount,
                smooth,
            } => taper::shear(self, c, [axis, toward], at(amount), smooth),
            Deform::Clip { axis, depth } => clip::clip(self, c, axis, depth, w),
            Deform::Spherize { amount, radius } => {
                clip::spherize(self, c, at(amount), radius.unwrap_or(0.0))
            }
            Deform::Noise {
                amount,
                scale,
                seed,
            } => noise::noise(c, at(amount), scale, seed),
            Deform::Lattice(_) => lattice::lattice(self, c, w),
            Deform::Curve(_) => curve::curve(self, c, w),
        };
        if d.iter().all(|&x| x == 0.0) {
            return Vec3::ZERO;
        }
        self.world(self.held(d))
    }

    /// The cage's grid or the curve's path as the vertex stage reads them (`DF9b`); empty for
    /// every other operation.
    pub(crate) fn table(&self) -> Vec<[f32; 4]> {
        match &self.op {
            Some(Deform::Lattice(_)) => lattice::table(self.cage.as_ref()),
            Some(Deform::Curve(_)) => curve::table(self.path.as_ref()),
            _ => Vec::new(),
        }
    }

    /// Where `s` lies along `axis` from the origin, as a fraction of the box's span ([`place`]).
    fn place(&self, axis: usize, s: f64) -> f64 {
        place(self.lo[axis], self.hi[axis], s)
    }

    /// How close to a line or point frame coordinates `c` must come to count as on it: what
    /// rounding of the point and the origin can move it.
    fn near(&self, c: [f64; 3]) -> f64 {
        1e-6 * ((c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() + self.far)
    }
}

/// Where `s` lies from the origin on an axis the box spans from `lo` to `hi`, as a fraction of
/// the span: both clamped into the box, so beyond it the place stays at the face's. 0 for a
/// flat box. The deformer handles measure their rates with it too.
pub(crate) fn place(lo: f64, hi: f64, s: f64) -> f64 {
    if hi.partial_cmp(&lo).is_none_or(Ordering::is_le) {
        return 0.0;
    }
    (s.clamp(lo, hi) - 0.0_f64.clamp(lo, hi)) / (hi - lo)
}

/// The columns of `q`'s rotation in `f64`, as [`Quat::axes`] makes them in `f32`.
fn frame_axes(q: Quat) -> [[f64; 3]; 3] {
    let [x, y, z, w] = q.to_array().map(f64::from);
    let s = 2.0 / (x * x + y * y + z * z + w * w);
    let (xx, yy, zz) = (x * x * s, y * y * s, z * z * s);
    let (xy, xz, yz) = (x * y * s, x * z * s, y * z * s);
    let (wx, wy, wz) = (w * x * s, w * y * s, w * z * s);
    [
        [1.0 - (yy + zz), xy + wz, xz - wy],
        [xy - wz, 1.0 - (xx + zz), yz + wx],
        [xz + wy, yz - wx, 1.0 - (xx + yy)],
    ]
}

/// `sin a / a`, 1 at 0, without the division losing precision near it.
fn sinc(a: f64) -> f64 {
    if a.abs() < 1e-4 {
        1.0 - a * a / 6.0
    } else {
        libm::sin(a) / a
    }
}
