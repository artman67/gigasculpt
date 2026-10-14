//! Symmetry: the one home of mirroring (CLAUDE.md). A dab is repeated once per pass: mirrored
//! across any of the X, Y and Z planes through the part's origin, and rotated around one axis
//! for radial symmetry. Every pass reads the same surface and their deltas are summed in one
//! kernel step, so a symmetric stroke on a symmetric mesh stays symmetric up to rounding.
//!
//! A plane is Mirror or Parallel (decision 0044). Both put the copy at the mirrored place; a
//! Mirror plane also mirrors the dab's directions (its step, a Move's grab, its alpha's axes),
//! and a Parallel plane leaves them as they are, so both copies go the same way. Each pass
//! has a place map ([`Symmetry::passes`]) and a direction map ([`Symmetry::directions`]),
//! which differ only by its Parallel planes.
//!
//! Near a mirror plane two passes overlap. With feathering on, all of a dab's passes are scaled
//! by one factor, one over their summed overlap ([`Symmetry::feather_scale`], decision 0040), so
//! a dab on the plane counts once and the passes' summed shape is kept; without it, strokes on
//! the plane count twice.

use crate::math::Vec3;

/// Which symmetry a stroke uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symmetry {
    /// Mirror across the planes `x = 0`, `y = 0` and `z = 0`.
    pub mirror: [bool; 3],
    /// Repeat at the place mirrored across these planes, directions unmirrored. A plane is
    /// Mirror or Parallel, not both.
    pub parallel: [bool; 3],
    /// Radial copies around an axis (0 = X, 1 = Y, 2 = Z): `(axis, count)`, count 2 or more.
    pub radial: Option<(usize, u32)>,
    /// Scale a dab's passes down by how much they overlap.
    pub feather: bool,
    /// Vertices this close to an active mirror plane, and those exactly on it, stay on it.
    pub clip: f32,
}

impl Default for Symmetry {
    /// No symmetry, feathering on, no plane clamp.
    fn default() -> Symmetry {
        Symmetry {
            mirror: [false; 3],
            parallel: [false; 3],
            radial: None,
            feather: true,
            clip: 0.0,
        }
    }
}

/// An orthogonal linear map (a mirror, a rotation, or both), as its three rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    rows: [Vec3; 3],
}

impl Xform {
    pub const IDENTITY: Xform = Xform {
        rows: [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
    };

    pub fn apply(self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.rows[0].dot(v),
            self.rows[1].dot(v),
            self.rows[2].dot(v),
        )
    }

    /// The map that undoes this one. Its rows are orthonormal, so it is the transpose.
    #[must_use]
    pub fn inverse(self) -> Xform {
        let [a, b, c] = self.rows;
        Xform {
            rows: [
                Vec3::new(a.x, b.x, c.x),
                Vec3::new(a.y, b.y, c.y),
                Vec3::new(a.z, b.z, c.z),
            ],
        }
    }

    /// The determinant: -1 for a map that mirrors, 1 for one that only turns.
    pub fn det(self) -> f32 {
        let [a, b, c] = self.rows;
        a.dot(b.cross(c))
    }

    /// Negate the axes whose bits are set in `mask` (bit 0 = X).
    fn mirror(mask: u32) -> Xform {
        let s = |a: u32| if mask >> a & 1 == 1 { -1.0 } else { 1.0 };
        Xform {
            rows: [
                Vec3::new(s(0), 0.0, 0.0),
                Vec3::new(0.0, s(1), 0.0),
                Vec3::new(0.0, 0.0, s(2)),
            ],
        }
    }

    /// Rotate by `k / n` of a turn around axis `axis`, right-handed.
    fn turn(axis: usize, k: u32, n: u32) -> Xform {
        let (s, c) = libm::sincosf(core::f32::consts::TAU * k as f32 / n as f32);
        let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut m = [[0.0f32; 3]; 3];
        m[axis][axis] = 1.0;
        (m[i][i], m[i][j], m[j][i], m[j][j]) = (c, -s, s, c);
        Xform {
            rows: m.map(Vec3::from_array),
        }
    }

    /// `o` first, then `self`.
    fn after(self, o: Xform) -> Xform {
        let [a, b, c] = o.rows;
        Xform {
            rows: self.rows.map(|r| a * r.x + b * r.y + c * r.z),
        }
    }
}

impl Symmetry {
    /// The map of every pass, the identity first: each radial copy of each combination of the
    /// Mirror and Parallel planes. It is where the pass's copy of a dab goes.
    pub fn passes(&self) -> Vec<Xform> {
        self.maps(|_| true)
    }

    /// The map each pass turns a dab's directions by, in [`Symmetry::passes`]'s order: the
    /// pass's own map without its Parallel planes.
    pub fn directions(&self) -> Vec<Xform> {
        self.maps(|a| self.mirror[a])
    }

    /// Each pass's turn after its reflections, across only the planes `keep` takes.
    fn maps(&self, keep: impl Fn(usize) -> bool) -> Vec<Xform> {
        let (axis, count) = self.radial.unwrap_or((0, 1));
        let planes = |a: u32| self.mirror[a as usize] || self.parallel[a as usize];
        let axes: Vec<u32> = (0..3).filter(|&a| planes(a)).collect();
        let mut out = Vec::new();
        for k in 0..count {
            for combo in 0..1u32 << axes.len() {
                let mask: u32 = (axes.iter().enumerate())
                    .filter(|&(b, &a)| combo >> b & 1 == 1 && keep(a as usize))
                    .map(|(_, &a)| 1 << a)
                    .sum();
                out.push(Xform::turn(axis, k, count).after(Xform::mirror(mask)));
            }
        }
        out
    }

    /// The delta `d` of a vertex at `p` after the plane clamp and the axis lock: on a locked
    /// axis nothing moves, and a vertex within `clip` of an active Mirror plane lands on it. A
    /// Parallel plane holds nothing: what it repeats is not mirror-symmetric.
    pub fn clamp(&self, p: Vec3, d: Vec3, lock: [bool; 3]) -> Vec3 {
        let (mut d, p) = (d.to_array(), p.to_array());
        for a in 0..3 {
            if lock[a] {
                d[a] = 0.0;
            } else if self.mirror[a] && p[a].abs() <= self.clip {
                d[a] = -p[a];
            }
        }
        Vec3::from_array(d)
    }

    /// The feather of one dab at `center` reaching `reach`, the factor every one of its passes'
    /// weights is scaled by (decision 0040, Blender's rule): one over the passes' summed overlap,
    /// where a pass whose center lies `d` from the dab's overlaps it by `1 - d / (2 reach)`, and
    /// not at all from `2 reach` apart, where their spheres stop meeting. One factor for the
    /// whole dab keeps the shape of the passes' sum: on a plane two passes count as one, far
    /// apart each keeps its strength, and between them no notch opens. 1 with the feather off.
    pub fn feather_scale(&self, center: Vec3, reach: f32) -> f32 {
        if !self.feather {
            return 1.0;
        }
        let overlap: f32 = (self.passes().iter())
            .map(|x| {
                let d = (x.apply(center) - center).len2().sqrt();
                if d < 2.0 * reach {
                    1.0 - d / (2.0 * reach)
                } else {
                    0.0
                }
            })
            .sum();
        // The identity overlaps itself by 1, unless the dab reaches nothing.
        1.0 / overlap.max(1.0)
    }
}
