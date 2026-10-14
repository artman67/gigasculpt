//! Vector math, rotations, boxes, face normals, the closest point of a triangle, Morton codes and
//! the cube bins of near-point searches.
//! Only IEEE-exact operations (add, multiply, divide, square root, min, max), plus libm's sine and
//! cosine where a rotation is made from an angle, so results are the same on every CPU
//! (ARCHITECTURE 8).

use std::ops::{Add, AddAssign, Mul, Neg, Sub};

/// The empty fourth slot of a triangle, and an empty face slot: gs-protocol's `NO_VERTEX`, the
/// one marker, under the short name the store's face code reads it by.
pub use gs_protocol::NO_VERTEX as NO;

/// A point or direction.
#[must_use]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    /// Position of a dead vertex slot. Every distance test against NaN is false, so kernels
    /// skip it, and `f32::min`/`max` ignore it, so bounds skip it too.
    pub const DEAD: Vec3 = Vec3::new(f32::NAN, f32::NAN, f32::NAN);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }
    #[inline]
    pub fn from_array(a: [f32; 3]) -> Vec3 {
        Vec3::new(a[0], a[1], a[2])
    }
    #[inline]
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
    #[inline]
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    #[inline]
    pub fn len2(self) -> f32 {
        self.dot(self)
    }
    /// Unit length, or zero for the zero vector.
    #[inline]
    pub fn normalized(self) -> Vec3 {
        let l = self.len2().sqrt();
        if l > 0.0 {
            self * (1.0 / l)
        } else {
            Vec3::ZERO
        }
    }
    #[inline]
    pub fn min(self, o: Vec3) -> Vec3 {
        self.zip(o, f32::min)
    }
    #[inline]
    pub fn max(self, o: Vec3) -> Vec3 {
        self.zip(o, f32::max)
    }
    /// `f` applied per axis.
    #[inline]
    fn zip(self, o: Vec3, f: impl Fn(f32, f32) -> f32) -> Vec3 {
        Vec3::new(f(self.x, o.x), f(self.y, o.y), f(self.z, o.z))
    }
    /// A dead vertex slot (see [`Vec3::DEAD`]).
    #[inline]
    pub fn is_dead(self) -> bool {
        self.x.is_nan()
    }
    /// The exact bit patterns, for hashing and bit-exact comparison.
    #[inline]
    pub fn bits(self) -> [u32; 3] {
        [self.x.to_bits(), self.y.to_bits(), self.z.to_bits()]
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    #[inline]
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    #[inline]
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

/// A rotation: the unit quaternion `x i + y j + z k + w`.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    /// No rotation.
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// `[x, y, z, w]`.
    pub fn from_array([x, y, z, w]: [f32; 4]) -> Quat {
        Quat { x, y, z, w }
    }

    /// `[x, y, z, w]`.
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// The turn by `angle` radians about the unit vector `axis`, counterclockwise when the
    /// axis points at the viewer (right-handed).
    pub fn about(axis: Vec3, angle: f32) -> Quat {
        let (s, c) = (libm::sinf(angle * 0.5), libm::cosf(angle * 0.5));
        Quat {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: c,
        }
    }

    /// The rotation taking the X, Y and Z axes to the orthonormal right-handed `x`, `y` and
    /// `z` (Shepperd's method: divide by the largest of the four candidates).
    pub fn from_axes(x: Vec3, y: Vec3, z: Vec3) -> Quat {
        let trace = x.x + y.y + z.z;
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [(y.z - z.y) / s, (z.x - x.z) / s, (x.y - y.x) / s, 0.25 * s]
        } else if x.x > y.y && x.x > z.z {
            let s = (1.0 + x.x - y.y - z.z).sqrt() * 2.0;
            [0.25 * s, (y.x + x.y) / s, (z.x + x.z) / s, (y.z - z.y) / s]
        } else if y.y > z.z {
            let s = (1.0 + y.y - x.x - z.z).sqrt() * 2.0;
            [(y.x + x.y) / s, 0.25 * s, (z.y + y.z) / s, (z.x - x.z) / s]
        } else {
            let s = (1.0 + z.z - x.x - y.y).sqrt() * 2.0;
            [(z.x + x.z) / s, (z.y + y.z) / s, 0.25 * s, (x.y - y.x) / s]
        };
        Quat::from_array(q).normalized()
    }

    /// The opposite rotation.
    pub fn inverse(self) -> Quat {
        Quat {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    /// Unit length; the identity for the zero quaternion.
    pub fn normalized(self) -> Quat {
        let n = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if n > 0.0 && n.is_finite() {
            let s = 1.0 / n;
            Quat {
                x: self.x * s,
                y: self.y * s,
                z: self.z * s,
                w: self.w * s,
            }
        } else {
            Quat::IDENTITY
        }
    }

    /// Where the X, Y and Z axes turn to: the columns of the rotation matrix. Divides by the
    /// squared length, so a quaternion a rounding away from unit still gives a rotation.
    pub fn axes(self) -> (Vec3, Vec3, Vec3) {
        let Quat { x, y, z, w } = self;
        let s = 2.0 / (x * x + y * y + z * z + w * w);
        let (xx, yy, zz) = (x * x * s, y * y * s, z * z * s);
        let (xy, xz, yz) = (x * y * s, x * z * s, y * z * s);
        let (wx, wy, wz) = (w * x * s, w * y * s, w * z * s);
        (
            Vec3::new(1.0 - (yy + zz), xy + wz, xz - wy),
            Vec3::new(xy - wz, 1.0 - (xx + zz), yz + wx),
            Vec3::new(xz + wy, yz - wx, 1.0 - (xx + yy)),
        )
    }

    /// `v` turned.
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let (ax, ay, az) = self.axes();
        ax * v.x + ay * v.y + az * v.z
    }
}

impl Mul for Quat {
    type Output = Quat;
    /// `a * b` turns by `b` first, then by `a` (the Hamilton product).
    #[inline]
    fn mul(self, b: Quat) -> Quat {
        let a = self;
        Quat {
            x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        }
    }
}

/// An axis-aligned box. [`Aabb::EMPTY`] contains nothing and is the identity of `union`.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Aabb::EMPTY
    }
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb {
        lo: Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
        hi: Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
    };

    #[inline]
    pub fn grow(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }
    #[inline]
    pub fn union(self, o: Aabb) -> Aabb {
        Aabb {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
        }
    }
    /// Squared distance from a point to the box: 0 inside, infinite for an empty box.
    #[inline]
    pub fn dist2(self, p: Vec3) -> f32 {
        let d = |lo: f32, hi: f32, v: f32| (lo - v).max(v - hi).max(0.0);
        let (dx, dy, dz) = (
            d(self.lo.x, self.hi.x, p.x),
            d(self.lo.y, self.hi.y, p.y),
            d(self.lo.z, self.hi.z, p.z),
        );
        dx * dx + dy * dy + dz * dz
    }
    /// Whether the box touches the sphere (`c`, `r`).
    #[inline]
    pub fn hits_sphere(self, c: Vec3, r: f32) -> bool {
        self.dist2(c) <= r * r
    }
    /// Where a ray from `o` along `d` is first inside the box, in lengths of `d` (0 when it
    /// starts inside); `None` when it misses, and always for an empty box.
    pub fn ray_entry(self, o: Vec3, d: Vec3) -> Option<f32> {
        if self.lo.x > self.hi.x {
            return None;
        }
        let (mut near, mut far) = (0.0f32, f32::INFINITY);
        let (lo, hi) = (self.lo.to_array(), self.hi.to_array());
        for ((lo, hi), (o, d)) in lo
            .into_iter()
            .zip(hi)
            .zip(o.to_array().into_iter().zip(d.to_array()))
        {
            if d == 0.0 {
                if !(lo <= o && o <= hi) {
                    return None;
                }
                continue;
            }
            let (a, b) = ((lo - o) / d, (hi - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far).then_some(near)
    }
}

/// Box around a set of points; dead slots are skipped.
pub fn bounds_of(points: &[Vec3]) -> Aabb {
    let mut b = Aabb::EMPTY;
    for &p in points {
        b.grow(p);
    }
    b
}

/// `a - b` on plain arrays, in `f32` or `f64`: the one copy gs-geo, gs-engine and the curve read.
pub fn sub<T: Copy + Sub<Output = T>>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    [ax - bx, ay - by, az - bz]
}

/// Dot product on plain arrays, in `f32` or `f64`.
pub fn dot<T: Copy + Add<Output = T> + Mul<Output = T>>(a: [T; 3], b: [T; 3]) -> T {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    ax * bx + ay * by + az * bz
}

/// Cross product `a x b` on plain arrays, in `f32` or `f64`.
pub fn cross<T: Copy + Sub<Output = T> + Mul<Output = T>>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

/// Corners in a face: 3 for a triangle (`NO` in the last slot), 4 for a quad.
pub use gs_protocol::arity;
/// The triangles a face is split into, by corner, wherever it is split (decision 0040, section 2).
pub use gs_protocol::face_triangles;

/// Whether faces `x` and `y` run through the same corners in the same cycle, from any corner:
/// the one test the mirror map's face images and weld's duplicate faces (gs-geo) both use.
pub fn same_cycle(x: [u32; 4], y: [u32; 4]) -> bool {
    let k = arity(x);
    k == arity(y) && (0..k).any(|r| (0..k).all(|i| x[(i + r) % k] == y[i]))
}

/// Points binned in cubes for a search near each one: the grid weld (gs-geo) and the mirror map
/// both search with. Each point's cube, sorted by cube and then point.
pub struct Bins(Vec<([i64; 3], u32)>);

impl Bins {
    /// Points `0..n` in the cubes `cube` puts them in.
    pub fn new(n: usize, cube: impl Fn(usize) -> [i64; 3] + Sync) -> Bins {
        let mut bins = gs_par::map(n, |i| (cube(i), i as u32));
        gs_par::sort(&mut bins);
        Bins(bins)
    }

    /// The points in the cubes from `lo` to `hi` (each axis inclusive), by cube, then point.
    pub fn within(&self, lo: [i64; 3], hi: [i64; 3]) -> impl Iterator<Item = u32> + '_ {
        let cubes = (lo[0]..=hi[0]).flat_map(move |x| {
            (lo[1]..=hi[1]).flat_map(move |y| (lo[2]..=hi[2]).map(move |z| [x, y, z]))
        });
        cubes.flat_map(move |c| {
            let start = self.0.partition_point(|b| b.0 < c);
            (self.0[start..].iter())
                .take_while(move |b| b.0 == c)
                .map(|b| b.1)
        })
    }
}

/// Twice the area-weighted face normal: `(b-a)x(c-a)` for a triangle, the cross product of the
/// diagonals for a quad.
#[inline]
pub fn face_cross(f: [u32; 4], mut get: impl FnMut(u32) -> Vec3) -> Vec3 {
    if f[3] == NO {
        let a = get(f[0]);
        (get(f[1]) - a).cross(get(f[2]) - a)
    } else {
        (get(f[2]) - get(f[0])).cross(get(f[3]) - get(f[1]))
    }
}

/// How well shaped a face is: twice its area (the length of [`face_cross`]) over its longest
/// side squared. An equilateral triangle has 0.87, a needle or a flat face nearly 0.
pub(crate) fn face_quality(f: [u32; 4], mut get: impl FnMut(u32) -> Vec3) -> f32 {
    let k = arity(f);
    let p: [Vec3; 4] = std::array::from_fn(|i| if i < k { get(f[i]) } else { Vec3::ZERO });
    let longest = (0..k)
        .map(|i| (p[i] - p[(i + 1) % k]).len2())
        .fold(0.0f32, f32::max);
    let cross = if k == 3 {
        (p[1] - p[0]).cross(p[2] - p[0])
    } else {
        (p[2] - p[0]).cross(p[3] - p[1])
    };
    if longest > 0.0 {
        cross.len2().sqrt() / longest
    } else {
        0.0
    }
}

/// Whether a face whose normal (any length) was `old` leans more than 60 degrees away from it
/// with normal `new`, or either is zero. Live Detail refuses any edit that does this: a sign
/// test alone lets a face stand on edge, the first step of a fold.
pub(crate) fn leans(old: Vec3, new: Vec3) -> bool {
    let d = new.dot(old);
    d <= 0.0 || 4.0 * d * d < old.len2() * new.len2()
}

/// The mean of a face's corners.
pub fn face_center(f: [u32; 4], mut get: impl FnMut(u32) -> Vec3) -> Vec3 {
    let corners = &f[..arity(f)];
    let sum = corners.iter().fold(Vec3::ZERO, |a, &u| a + get(u));
    sum * (1.0 / corners.len() as f32)
}

/// How thin a triangle may be and still have an inside: its height over its longest side (twice
/// its area over that side squared, as [`face_quality`] measures). Thinner, and rounding moves
/// the inside's answer further than the nearest edge lies from it. Measured in `f32` against
/// `f64` on the same corners, points within a side's length of the plane: at 1/256 the inside
/// is off by up to 0.0007 of the side and the nearest edge by 0.0019; at 1/512, 0.0035 and 0.0010.
const FLAT: f32 = 1.0 / 256.0;

/// The point of triangle `t` closest to `p`, and its weights, which rebuild it as
/// `t[0] * w[0] + t[1] * w[1] + t[2] * w[2]`. The one closest-point search: the projection
/// between surfaces, Live Detail's region and voxel remesh's distances all call it. Ericson,
/// Real-Time Collision Detection, 5.1.5: which of seven regions (three corners, three edges, the
/// inside) holds the point, by dot products only.
///
/// Any triangle with finite corners gives finite weights in [0, 1] (to rounding) that sum to 1,
/// and nothing is ever divided by 0. Degenerate triangles give their true closest point:
/// - an edge of no length (two corners in one place) has no region of its own: its two ends are
///   one corner, already tested, so the search goes on to the other edges;
/// - a triangle less than 1/256 as high as its longest side (`FLAT`; three corners on one line,
///   or nearly) has no inside to speak of, as rounding alone makes up the inside's shares there:
///   it takes the nearest point of its three edges, off by at most half its height.
#[inline]
pub fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> (Vec3, [f32; 3]) {
    search::<true>(p, t)
}

/// The weights alone of [`closest_on_triangle`]: the same search, compiled without building
/// the point, for a loop that measures by the weights (voxel remesh's distance field, once per
/// sample).
#[inline]
pub fn closest_weights(p: Vec3, t: &[Vec3; 3]) -> [f32; 3] {
    search::<false>(p, t).1
}

/// The search behind [`closest_on_triangle`] and [`closest_weights`]; it builds the point only
/// when `POINT` is set.
#[expect(
    clippy::inline_always,
    reason = "one search, two faces: inline, the face without the point drops the point's work"
)]
#[inline(always)]
fn search<const POINT: bool>(p: Vec3, t: &[Vec3; 3]) -> (Vec3, [f32; 3]) {
    // The point, or nothing to compute when the caller wants the weights alone.
    let point = |q: Vec3| if POINT { q } else { Vec3::ZERO };
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (point(a), [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (point(b), [0.0, 1.0, 0.0]);
    }
    // Each edge's length squared, from the dot products: its region's divisor.
    let ab2 = d1 - d3;
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 && ab2 > 0.0 {
        let v = d1 / ab2;
        return (point(a + ab * v), [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (point(c), [0.0, 0.0, 1.0]);
    }
    let ac2 = d2 - d6;
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 && ac2 > 0.0 {
        let w = d2 / ac2;
        return (point(a + ac * w), [1.0 - w, 0.0, w]);
    }
    let (from_b, from_c) = (d4 - d3, d5 - d6);
    let bc2 = from_b + from_c;
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && from_b >= 0.0 && from_c >= 0.0 && bc2 > 0.0 {
        let w = from_b / bc2;
        return (point(b + (c - b) * w), [0.0, 1.0 - w, w]);
    }
    // The inside, whose three shares sum to twice the area, squared. The longest side by plain
    // comparisons, one instruction each: `f32::max` also sorts out NaN, at five more apiece.
    let total = va + vb + vc;
    let longest = if ab2 > ac2 { ab2 } else { ac2 };
    let flat = FLAT * if longest > bc2 { longest } else { bc2 };
    if va >= 0.0 && vb >= 0.0 && vc >= 0.0 && total > flat * flat {
        let (v, w) = (vb / total, vc / total);
        return (point(a + ab * v + ac * w), [1.0 - v - w, v, w]);
    }
    // Only a degenerate triangle gets here: the nearest point of its three edges, the first on a
    // tie, an edge of no length being its first corner. All from the dot products above, so the
    // hot path keeps no more in hand for it: each edge's parameter, clamped to the edge, and the
    // squared distance to that point less |ap|², which all three share (|bp|² is |ap|² - 2 d1
    // + |ab|²).
    std::hint::cold_path();
    let along = |num: f32, len2: f32| {
        if len2 > 0.0 {
            (num / len2).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let (s, u, r) = (along(d1, ab2), along(d2, ac2), along(from_b, bc2));
    let on_ab = s * (s * ab2 - 2.0 * d1);
    let on_ac = u * (u * ac2 - 2.0 * d2);
    let on_bc = ab2 - 2.0 * d1 + r * (r * bc2 - 2.0 * from_b);
    if on_ab <= on_ac && on_ab <= on_bc {
        (point(a + ab * s), [1.0 - s, s, 0.0])
    } else if on_ac <= on_bc {
        (point(a + ac * u), [1.0 - u, 0.0, u])
    } else {
        (point(b + (c - b) * r), [0.0, 1.0 - r, r])
    }
}

/// Spread the low 21 bits of `x` so there are two zero bits between each. Each step adds a
/// shifted copy whose bits do not overlap the original, then masks.
fn part1by2(x: u64) -> u64 {
    let mut x = x & 0x1f_ffff;
    x = (x + (x << 32)) & 0x1f_0000_0000_ffff;
    x = (x + (x << 16)) & 0x1f_0000_ff00_00ff;
    x = (x + (x << 8)) & 0x100f_00f0_0f00_f00f;
    x = (x + (x << 4)) & 0x10c3_0c30_c30c_30c3;
    (x + (x << 2)) & 0x1249_2492_4924_9249
}

/// 63-bit Morton code of a point inside `b`, 21 bits per axis. Points outside are clamped.
#[inline]
pub fn morton(p: Vec3, b: Aabb) -> u64 {
    let q = |v: f32, lo: f32, hi: f32| {
        let s = 2_097_151.0 / (hi - lo).max(1e-30);
        (((v - lo) * s) as u64).min(0x1f_ffff)
    };
    part1by2(q(p.x, b.lo.x, b.hi.x))
        + (part1by2(q(p.y, b.lo.y, b.hi.y)) << 1)
        + (part1by2(q(p.z, b.lo.z, b.hi.z)) << 2)
}
