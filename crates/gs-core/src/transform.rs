//! Weighted transforms (decision 0027): move, rotate, scale and inflate a part about a frame,
//! each point weighted by visibility, the mask and a falloff from the frame's origin, with one
//! mirrored transform per symmetry region. The one implementation behind `transform.apply` and
//! the gizmo's drags. The deformers (decision 0041, [`crate::deform`]) are more operations of
//! the same kind, and the [`Scope`] every operation shares narrows any of them.
//!
//! A weight scales the transform, never the result, as Blender's Edit-mode transforms do
//! (`editors/transform/transform_mode_rotate.cc` turns by `angle * td->factor`): a point at
//! weight `w` moves `w` of the way, turns `w` of the angle about the same axis, and scales by
//! `1 + w (s - 1)`. Every change is a difference that is exactly zero for a zero amount, so a
//! drag that comes back to where it started leaves every bit as it was.
//!
//! Symmetry follows Blender's sculpt transform (`editors/sculpt_paint/sculpt_transform.cc`,
//! `calc_symm_area_transform_translations`): the region holding the frame's origin gets the
//! transform as given, every other region the transform mirrored across the planes where the
//! two differ, and a point on an active plane keeps its coordinate across it. A plane the
//! frame's origin lies on (within the clip) is left out: the gizmo and its mirror coincide, so
//! the part moves as one piece (catalog `gizmo.symmetry`). A Parallel plane (decision 0044,
//! section 5) splits the part the same way, but the far side gets the transform unreflected,
//! about the frame origin's reflection, and a point on it is not held: a region's origin is
//! reflected across every plane between, its directions across the Mirror planes alone.
//!
//! [`Similarity`] is a part's transform (decision 0029): where its points, kept in the part's
//! own space, are in the scene. It holds the only code that maps between the two spaces.

mod preview;

pub use preview::Prepared;

use serde::{Deserialize, Serialize};

use crate::camera::{Camera, M4};
use crate::columns::{AttrDef, AttrId, AttrVec};
use crate::config::VertId;
use crate::deform::{Deform, Ready, Scope};
use crate::falloff::{Profile, Tip};
use crate::kernel::{Kernel, Reach, Scratch, SculptSurface, StepReport};
use crate::kernels::flood;
use crate::math::{Aabb, Quat, Vec3};
use crate::store::MeshStore;
use crate::symmetry::Symmetry;
use crate::view::NodeView;

/// The Falloff value at which a transform is rigid: every weight is 1.
pub const RIGID: f32 = -100.0;

/// A part's transform (decision 0029): a uniform scale, then a turn, then a move. A part point
/// `p` is the scene point `rotation(p) * scale + translation`. Only a similarity is ever kept,
/// so a sphere in the scene is a sphere in the part, and normals turn by the rotation alone.
///
/// The identity maps every point and vector to itself bit for bit, so a part at the identity
/// behaves exactly as a part without a transform. Scene files store it as three plain fields.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "SimilarityFile", into = "SimilarityFile")]
pub struct Similarity {
    pub translation: Vec3,
    /// A unit quaternion.
    pub rotation: Quat,
    /// Above zero.
    pub scale: f32,
}

/// [`Similarity`] as scene files hold it.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SimilarityFile {
    translation: [f32; 3],
    /// `[x, y, z, w]`.
    rotation: [f32; 4],
    scale: f32,
}

impl From<SimilarityFile> for Similarity {
    fn from(f: SimilarityFile) -> Self {
        Similarity {
            translation: Vec3::from_array(f.translation),
            rotation: Quat::from_array(f.rotation),
            scale: f.scale,
        }
    }
}

impl From<Similarity> for SimilarityFile {
    fn from(s: Similarity) -> Self {
        SimilarityFile {
            translation: s.translation.to_array(),
            rotation: s.rotation.to_array(),
            scale: s.scale,
        }
    }
}

impl Default for Similarity {
    fn default() -> Self {
        Similarity::IDENTITY
    }
}

impl Similarity {
    /// No move, no turn, scale 1.
    pub const IDENTITY: Similarity = Similarity {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: 1.0,
    };

    /// A move by `v`.
    pub fn moving(v: Vec3) -> Similarity {
        Similarity {
            translation: v,
            ..Similarity::IDENTITY
        }
    }

    /// A turn by the unit quaternion `q` about the point `origin`.
    pub fn turning(origin: Vec3, q: Quat) -> Similarity {
        Similarity {
            translation: origin - q.rotate(origin),
            rotation: q,
            scale: 1.0,
        }
    }

    /// A scale by `k` (above zero) about the point `origin`.
    pub fn scaling(origin: Vec3, k: f32) -> Similarity {
        Similarity {
            translation: origin - origin * k,
            rotation: Quat::IDENTITY,
            scale: k,
        }
    }

    /// True for no move, no turn and scale 1 (a turn by a whole circle counts as no turn).
    pub fn is_identity(&self) -> bool {
        self.translation == Vec3::ZERO && self.turns_not() && self.unscaled()
    }

    /// True when the scale is exactly 1, so lengths map bit for bit.
    fn unscaled(&self) -> bool {
        self.scale.to_bits() == 1.0_f32.to_bits()
    }

    /// True when the transform turns: its rotation is not the identity.
    pub fn turns(&self) -> bool {
        !self.turns_not()
    }

    fn turns_not(&self) -> bool {
        let q = self.rotation;
        q.x == 0.0 && q.y == 0.0 && q.z == 0.0
    }

    /// True when every number is finite, the rotation is a unit quaternion within rounding and
    /// the scale is above zero: what a scene file must hold.
    pub fn is_valid(&self) -> bool {
        let q = self.rotation.to_array();
        let n2: f32 = q.iter().map(|c| c * c).sum();
        let finite = self
            .translation
            .to_array()
            .iter()
            .chain(&q)
            .all(|c| c.is_finite());
        finite && (n2 - 1.0).abs() < 1e-4 && self.scale > 0.0 && self.scale.is_finite()
    }

    /// `self`, then `after`: the transform that maps a point as `self` does and then as
    /// `after` does. The rotation is normalized, so a long chain stays a rotation.
    #[must_use]
    pub fn then(&self, after: &Similarity) -> Similarity {
        if self.is_identity() {
            return *after;
        }
        if after.is_identity() {
            return *self;
        }
        Similarity {
            translation: after.point(self.translation),
            rotation: (after.rotation * self.rotation).normalized(),
            scale: after.scale * self.scale,
        }
    }

    /// The transform that undoes this one.
    #[must_use]
    pub fn inverse(&self) -> Similarity {
        let rotation = self.rotation.inverse();
        Similarity {
            translation: rotation.rotate(-self.translation) * (1.0 / self.scale),
            rotation,
            scale: 1.0 / self.scale,
        }
    }

    /// The scene point of part point `p`: `rotation(p) * scale + translation`, in that order.
    pub fn point(&self, p: Vec3) -> Vec3 {
        if self.is_identity() {
            return p;
        }
        self.direction(p) * self.scale + self.translation
    }

    /// The scene direction of part direction `d`: turned, not scaled, so a unit vector stays
    /// one.
    pub fn direction(&self, d: Vec3) -> Vec3 {
        if self.turns_not() {
            return d;
        }
        self.rotation.rotate(d)
    }

    /// The scene length of part length `l`.
    pub fn length(&self, l: f32) -> f32 {
        if self.unscaled() { l } else { l * self.scale }
    }

    /// The part point of scene point `p`.
    pub fn part_point(&self, p: Vec3) -> Vec3 {
        if self.is_identity() {
            return p;
        }
        self.part_direction(p - self.translation) * (1.0 / self.scale)
    }

    /// The part direction of scene direction `d`.
    pub fn part_direction(&self, d: Vec3) -> Vec3 {
        if self.turns_not() {
            return d;
        }
        self.rotation.inverse().rotate(d)
    }

    /// The part displacement of scene displacement `v`: turned back and scaled down.
    pub fn part_offset(&self, v: Vec3) -> Vec3 {
        let d = self.part_direction(v);
        if self.unscaled() {
            d
        } else {
            d * (1.0 / self.scale)
        }
    }

    /// The part length of scene length `l`.
    pub fn part_length(&self, l: f32) -> f32 {
        if self.unscaled() { l } else { l / self.scale }
    }

    /// The scene box around part box `b`: its corners mapped and boxed. With a turn it is
    /// larger than the box of the points inside.
    pub fn bounds(&self, b: Aabb) -> Aabb {
        if self.is_identity() || b == Aabb::EMPTY {
            return b;
        }
        let mut out = Aabb::EMPTY;
        for c in 0..8 {
            let pick = |bit: u32, lo: f32, hi: f32| if c & bit == 0 { lo } else { hi };
            let corner = Vec3::new(
                pick(1, b.lo.x, b.hi.x),
                pick(2, b.lo.y, b.hi.y),
                pick(4, b.lo.z, b.hi.z),
            );
            out.grow(self.point(corner));
        }
        out
    }

    /// The part ray of the scene ray from `origin` along unit `direction`: a hit `t` along it
    /// is `scale * t` along the scene ray.
    pub fn part_ray(&self, origin: Vec3, direction: Vec3) -> (Vec3, Vec3) {
        (self.part_point(origin), self.part_direction(direction))
    }

    /// The camera as the part sees it: the same picture, every ray and projected point the
    /// part-space image of the scene camera's, because a similarity keeps angles.
    pub fn part_camera(&self, camera: &Camera) -> Camera {
        if self.is_identity() {
            return *camera;
        }
        Camera {
            orientation: (self.rotation.inverse() * camera.orientation).normalized(),
            target: self.part_point(Vec3::from_array(camera.target)).to_array(),
            distance: self.part_length(camera.distance),
            ..*camera
        }
    }

    /// `t`, given in the scene, as it acts on the part's own points (decision 0029, section 6):
    /// the frame's origin mapped as a point and its axes turned back, a move turned back and
    /// scaled down, a turn's axis turned back, and inflate's distance, a deformer's lengths, the
    /// box, the reach and the clip scaled down. Angles, scale factors and a deformer's axes are
    /// the frame's own and stay; the mirror planes are the part's (section 7). The identity
    /// gives `t` itself.
    pub fn part_transform(&self, t: &Transform) -> Transform {
        if self.is_identity() {
            return t.clone();
        }
        let op = match t.op {
            Op::Move(d) => Op::Move(self.part_offset(d)),
            Op::Rotate { axis, angle } => Op::Rotate {
                axis: self.part_direction(axis),
                angle,
            },
            Op::Scale(s) => Op::Scale(s),
            Op::Inflate(a) => Op::Inflate(self.part_length(a)),
            Op::Deform(ref d) => Op::Deform(d.clone().scaled(|l| self.part_length(l))),
        };
        let axes = if self.turns() {
            (self.rotation.inverse() * t.frame.axes).normalized()
        } else {
            t.frame.axes
        };
        Transform {
            frame: Frame {
                origin: self.part_point(t.frame.origin),
                axes,
            },
            op,
            weights: Weights {
                reach: self.part_length(t.weights.reach),
                ..t.weights
            },
            symmetry: Symmetry {
                clip: self.part_length(t.symmetry.clip),
                ..t.symmetry
            },
            scope: t.scope.scaled(|l| self.part_length(l)),
        }
    }

    /// Part to scene as a column-major 4 by 4 matrix, as the renderer takes it.
    pub fn matrix(&self) -> M4 {
        let (x, y, z) = self.rotation.axes();
        let (x, y, z) = if self.turns_not() {
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            )
        } else {
            (x, y, z)
        };
        let s = self.scale;
        let t = self.translation;
        [
            [x.x * s, x.y * s, x.z * s, 0.0],
            [y.x * s, y.y * s, y.z * s, 0.0],
            [z.x * s, z.y * s, z.z * s, 0.0],
            [t.x, t.y, t.z, 1.0],
        ]
    }
}

/// A point and three axes: what a transform turns and scales about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    /// A unit quaternion; its columns are the frame's X, Y and Z.
    pub axes: Quat,
}

/// What a transform does at full weight.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Move by a world vector.
    Move(Vec3),
    /// Turn by `angle` radians about the unit world vector `axis` through the origin.
    Rotate { axis: Vec3, angle: f32 },
    /// Scale about the origin by a factor along each frame axis. Flattening toward a frame
    /// plane is a factor below 1 along its axis.
    Scale([f32; 3]),
    /// Move along each point's normal by this many scene units.
    Inflate(f32),
    /// A deformer about the frame.
    Deform(Deform),
}

/// How a point's distance from the origin is measured for the falloff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distance {
    /// The straight line.
    Straight,
    /// Along edges of visible faces, from the vertex nearest the origin.
    Surface,
}

/// How much of the transform each point gets: visible × (1 - mask) × falloff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// The mask holds points back.
    pub mask: bool,
    /// The Falloff setting, -100 to 100; [`RIGID`] or below gives every point weight 1.
    pub falloff: f32,
    /// The distance where the falloff reaches 0, in scene units. Unused when rigid.
    pub reach: f32,
    pub distance: Distance,
}

/// One transform of a part's points.
#[derive(Clone, Debug, PartialEq)]
pub struct Transform {
    pub frame: Frame,
    pub op: Op,
    pub weights: Weights,
    /// Mirror and Parallel planes through the part's origin and their clip distance. Radial
    /// copies are not part of transforms and are ignored.
    pub symmetry: Symmetry,
    pub scope: Scope,
}

impl Op {
    /// The op about `origin` as a part transform, when one can hold it (decision 0029, section
    /// 6): a move, a turn, or a scale by three equal factors above zero. `None` for inflate, a
    /// deformer and a stretch, which only an edit of the points can make.
    pub fn similarity(&self, origin: Vec3) -> Option<Similarity> {
        match *self {
            Op::Move(d) => Some(Similarity::moving(d)),
            Op::Rotate { axis, angle } => {
                Some(Similarity::turning(origin, Quat::about(axis, angle)))
            }
            Op::Scale([a, b, c])
                if a > 0.0
                    && a.is_finite()
                    && a.to_bits() == b.to_bits()
                    && b.to_bits() == c.to_bits() =>
            {
                Some(Similarity::scaling(origin, a))
            }
            Op::Scale(_) | Op::Inflate(_) | Op::Deform(_) => None,
        }
    }
}

impl MeshStore {
    /// Applies `t` to the points, as one kernel step: positions and normals are written only
    /// where a point moves, so the open history step holds exactly what changed. A box or
    /// spherize radius `t` leaves out is measured first, as [`MeshStore::measured`] reports it.
    pub fn transform(&mut self, t: &Transform) -> StepReport {
        let kernel = Weighted::new(self, t).measured(self);
        self.run_kernel(&kernel, &mut Scratch::default())
    }
}

/// The transform as a kernel: every point's change read from its own position and normal.
pub(crate) struct Weighted {
    pub(crate) t: Transform,
    /// The deformer and the scope, ready for points.
    pub(crate) ready: Ready,
    /// The falloff curve; `None` when rigid.
    tip: Option<Tip>,
    /// The mask column when the mask holds points back and the part has one.
    mask: Option<AttrId>,
    /// The frame's axes as vectors.
    axes: [Vec3; 3],
    /// The full turn, for points at weight 1.
    turn: Quat,
    /// The splitting Mirror planes, as bits.
    mirrored: u32,
    /// For surface distances, per region: every vertex in reach with its distance, ascending.
    floods: Vec<Vec<(VertId, f32)>>,
}

impl Weighted {
    pub(crate) fn new(store: &MeshStore, t: &Transform) -> Weighted {
        let w = t.weights;
        let rigid = w.falloff <= RIGID;
        let tip = (!rigid).then(|| Tip::new(Profile::Smooth, 0.0, w.falloff));
        let mask = (w.mask)
            .then(|| store.attr_id(&AttrDef::mask().name))
            .flatten();
        let (x, y, z) = t.frame.axes.axes();
        let turn = match t.op {
            Op::Rotate { axis, angle } => Quat::about(axis, angle),
            _ => Quat::IDENTITY,
        };
        let mut k = Weighted {
            t: t.clone(),
            ready: Ready::new(t),
            tip,
            mask,
            axes: [x, y, z],
            turn,
            mirrored: 0,
            floods: Vec::new(),
        };
        k.mirrored = (0..3).filter(|&a| k.mirrors(a)).map(|a| 1 << a).sum();
        if !rigid && w.distance == Distance::Surface {
            k.floods = (0..8).map(|m| k.flood_of(store, m)).collect();
        }
        k
    }

    /// The node's mask values, when the mask holds points back and the node has any.
    pub(crate) fn mask_of<'n>(&self, node: &NodeView<'n>) -> Option<&'n [f32]> {
        match self.mask.and_then(|a| node.attr(a)) {
            Some(AttrVec::F32(m)) => Some(m.as_slice()),
            _ => None,
        }
    }

    /// Local slot `l`'s offset from its region's origin, direction-mapped, the Mirror planes
    /// it lies across, and its weight: visible × (1 - mask) × falloff. `None` for a hidden or
    /// dead point and one of weight 0. The offset is, across a Mirror plane, the point's mirror
    /// image less the origin; across a Parallel plane, the point less the reflected origin.
    pub(crate) fn weigh(
        &self,
        node: &NodeView<'_>,
        mask: Option<&[f32]>,
        l: usize,
    ) -> Option<(Vec3, u32, f32)> {
        let p = node.pos[l];
        if node.hidden(l) || p.is_dead() {
            return None;
        }
        let free = mask.map_or(1.0, |m| Self::free(m[l]));
        let m = self.region(p);
        let r = m & self.mirrored;
        let v = flip(p, r) - flip(self.t.frame.origin, m & !r);
        let w = free * self.fade(node.base + l as u32, v, m);
        (w != 0.0).then_some((v, r, w))
    }

    /// The flood from the vertex nearest region `m`'s origin, with the straight step to it
    /// added, out to the reach. Empty for a region the planes do not make.
    fn flood_of(&self, store: &MeshStore, m: u32) -> Vec<(VertId, f32)> {
        if m & !self.planes() != 0 {
            return Vec::new();
        }
        let o = flip(self.t.frame.origin, m);
        let Some((seed, gap)) = store.nearest_vertex(o).found else {
            return Vec::new();
        };
        let reach = self.t.weights.reach;
        if gap > reach {
            return Vec::new();
        }
        let mut f = flood(store, seed, reach - gap);
        for (_, d) in &mut f {
            *d += gap;
        }
        f
    }

    /// Whether the plane across axis `a` splits the part into regions: it is on, Mirror or
    /// Parallel, and the frame's origin is off it.
    fn splits(&self, a: usize) -> bool {
        let s = &self.t.symmetry;
        (s.mirror[a] || s.parallel[a]) && self.t.frame.origin.to_array()[a].abs() > s.clip
    }

    /// Whether the plane across axis `a` mirrors the transform: a Mirror plane that splits.
    fn mirrors(&self, a: usize) -> bool {
        self.t.symmetry.mirror[a] && self.splits(a)
    }

    /// The splitting planes, as bits.
    fn planes(&self) -> u32 {
        (0..3).filter(|&a| self.splits(a)).map(|a| 1 << a).sum()
    }

    /// The planes `p` lies across from the frame's origin: its region. A point within the clip
    /// of a plane counts as on the origin's side of it.
    fn region(&self, p: Vec3) -> u32 {
        let (p, o) = (p.to_array(), self.t.frame.origin.to_array());
        let clip = self.t.symmetry.clip;
        (0..3)
            .filter(|&a| self.splits(a) && p[a].abs() > clip && (p[a] < 0.0) != (o[a] < 0.0))
            .map(|a| 1 << a)
            .sum()
    }

    /// How much a point of the mask's slot value `m` is free to move: the rule brushes use
    /// (gs-core `kernels`, `Ctx::common`), where a value that is not a number masks fully.
    fn free(m: f32) -> f32 {
        if m.is_finite() {
            1.0 - m.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The falloff of vertex `id` in region `m`, at offset `v` from the region's origin.
    fn fade(&self, id: VertId, v: Vec3, m: u32) -> f32 {
        let Some(tip) = &self.tip else {
            return 1.0;
        };
        let w = self.t.weights;
        let d = match w.distance {
            Distance::Straight => v.len2().sqrt(),
            Distance::Surface => match self.floods[m as usize].binary_search_by_key(&id, |e| e.0) {
                Ok(i) => self.floods[m as usize][i].1,
                Err(_) => return 0.0,
            },
        };
        tip.falloff(d / w.reach)
    }

    /// The change of a point at offset `v` from the frame's origin with unit normal `n`, in the
    /// origin's region, at weight `w`, along the kept axes.
    fn change(&self, v: Vec3, n: Vec3, w: f32) -> Vec3 {
        self.ready.keep(match self.t.op {
            Op::Move(d) => d * w,
            Op::Rotate { axis, angle } => {
                // Full weight reuses the turn made once; the same angle gives the same bits.
                let turn = if w.to_bits() == 1f32.to_bits() {
                    self.turn
                } else {
                    Quat::about(axis, angle * w)
                };
                turn.rotate(v) - v
            }
            Op::Scale(s) => (self.axes.iter().zip(s)).fold(Vec3::ZERO, |sum, (&a, s)| {
                sum + a * (w * (s - 1.0) * v.dot(a))
            }),
            Op::Inflate(amount) => n.normalized() * (w * amount),
            Op::Deform(_) => return self.ready.change(v, w),
        })
    }
}

impl Kernel for Weighted {
    fn spheres(&self) -> Vec<(Vec3, f32)> {
        let o = self.t.frame.origin;
        if self.tip.is_none() {
            return vec![(o, f32::INFINITY)];
        }
        let planes = self.planes();
        (0..8)
            .filter(|m| m & !planes == 0)
            .map(|m| (flip(o, m), self.t.weights.reach))
            .collect()
    }

    fn reach(&self) -> Reach {
        if self.mask.is_some() {
            Reach::Unmasked
        } else {
            Reach::Visible
        }
    }

    fn own_only(&self) -> bool {
        true
    }

    fn deltas(&self, _: &SculptSurface<'_>, node: &NodeView<'_>, out: &mut [Vec3]) {
        let mask = self.mask_of(node);
        let clip = self.t.symmetry.clip;
        for (l, out) in out.iter_mut().enumerate() {
            // The region's origin is the frame's reflected across every plane the point lies
            // across, its directions across the Mirror planes `r` alone.
            let Some((v, r, w)) = self.weigh(node, mask, l) else {
                continue;
            };
            let w = self.ready.ramp(v, w);
            if w == 0.0 {
                continue;
            }
            let mut d = flip(self.change(v, flip(node.nrm[l], r), w), r).to_array();
            let at = node.pos[l].to_array();
            for a in 0..3 {
                if self.mirrors(a) && at[a].abs() <= clip {
                    d[a] = 0.0;
                }
            }
            *out = Vec3::from_array(d);
        }
    }
}

/// `v` mirrored across the planes in `m` (bit `a` mirrors coordinate `a`).
pub(crate) fn flip(v: Vec3, m: u32) -> Vec3 {
    let mut a = v.to_array();
    for (i, c) in a.iter_mut().enumerate() {
        if m & (1 << i) != 0 {
            *c = -*c;
        }
    }
    Vec3::from_array(a)
}
