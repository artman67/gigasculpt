//! The influence stage every brush shares: the one home of hide, mask, automask, alpha and the
//! symmetry feather, which turn a point's place into its weight in each symmetry pass.

use std::cell::RefCell;

use super::{Frame, PassStart, moves};
use crate::alpha::AlphaRead;
use crate::brush::{BrushPreset, KernelSpec, Select};
use crate::cavity::{self, Walk};
use crate::columns::{AttrId, AttrVec};
use crate::config::VertId;
use crate::falloff::Tip;
use crate::kernel::{Free, Reach, SculptSurface};
use crate::math::Vec3;
use crate::symmetry::Symmetry;
use crate::view::{NodeView, TopoRead};

/// The attribute columns the influence stage reads. Hidden points come from the store's derived
/// state, not from a column.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Cols {
    /// The mask, unless the brush writes the mask or hide itself.
    pub mask: Option<AttrId>,
    pub group: Option<AttrId>,
}

/// Everything about the stroke that kernels read.
pub(crate) struct Ctx<'a> {
    pub preset: &'a BrushPreset,
    pub tip: &'a Tip,
    pub alpha: Option<AlphaRead<'a>>,
    pub sym: Symmetry,
    pub lock: [bool; 3],
    /// An inverted stroke (Alt or Subtract).
    pub invert: bool,
    pub starts: &'a [PassStart],
    pub cols: Cols,
}

impl Ctx<'_> {
    /// Where a pass's weight reaches zero.
    pub fn reach(&self, f: &Frame) -> f32 {
        match &self.preset.kernel {
            KernelSpec::Move(m) if m.select == Select::Elastic => f.radius * m.support,
            _ => f.radius,
        }
    }

    /// Infinite Depth's free directions in pass `f`, when it is on (decision 0053, section 3):
    /// the model axes switched on, turned by the pass's direction map, else the pass's view.
    pub fn free(&self, f: &Frame) -> Option<Free> {
        let d = &self.preset.depth;
        if !d.infinite {
            return None;
        }
        if !d.axes.contains(&true) {
            return Some(Free::new([f.view.normalized()]));
        }
        let unit = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let on = unit.into_iter().zip(d.axes).filter(|&(_, on)| on);
        Some(Free::new(on.map(|(e, _)| f.xf.apply(e))))
    }

    /// What every point of `node` shares in passes `frames`, worked out once for the node.
    pub fn plan<'n>(&self, node: &NodeView<'n>, frames: &[Frame]) -> Plan<'n> {
        let fall = self.fall();
        let pass = |f: &Frame| {
            let reach = self.reach(f);
            let r = reach * 1.001;
            let far = if fall == Fade::Edges {
                f32::INFINITY
            } else {
                r * r
            };
            (reach, far)
        };
        let am = &self.preset.automask;
        let free: Vec<Free> = frames.iter().filter_map(|f| self.free(f)).collect();
        Plan {
            passes: frames.iter().map(pass).collect(),
            fall,
            mask: self.mask(node),
            // Automask settings are never negative, so `!= 0.0` means switched on.
            automask: am.topology != 0.0 || am.backface != 0.0 || am.face_group != 0.0,
            deep: self.preset.depth.mask || !free.is_empty(),
            free,
            clamps: self.lock.contains(&true) || self.sym.mirror.contains(&true),
            feather: self.feather(frames),
        }
    }

    /// The symmetry feather of the dab whose passes are `frames` (decision 0040), its center
    /// measured across Infinite Depth's free directions (decision 0053, section 5).
    pub fn feather(&self, frames: &[Frame]) -> f32 {
        frames.first().map_or(1.0, |f| {
            let c = self.free(f).map_or(f.center, |e| e.across(f.center));
            self.sym.feather_scale(c, self.reach(f))
        })
    }

    /// The mask values of `node` the common factor reads, if the brush reads the mask.
    pub fn mask<'n>(&self, node: &NodeView<'n>) -> Option<&'n [f32]> {
        match self.cols.mask.and_then(|a| node.attr(a)) {
            Some(AttrVec::F32(mask)) => Some(mask),
            _ => None,
        }
    }

    /// The weight of a point in a pass from distance, the Depth Mask and alpha alone.
    pub fn shape(&self, f: &Frame, p: Vec3) -> f32 {
        let reach = self.reach(f);
        let gap = self
            .free(f)
            .map_or(p - f.center, |e| e.across(p - f.center));
        let mut w = self.fade(gap.len2(), reach, self.fall(), f.pen);
        if self.preset.depth.mask && w > 0.0 {
            w *= self.depth_mask(f, p);
        }
        self.stamp(f, reach, p, w)
    }

    /// How this brush's weights fall off.
    fn fall(&self) -> Fade {
        match &self.preset.kernel {
            KernelSpec::Move(m) if m.select == Select::Elastic => Fade::Elastic,
            KernelSpec::Move(m) if m.select == Select::Connected => Fade::Edges,
            _ => Fade::Tip,
        }
    }

    /// The weight of a point in a pass with reach `reach` from its squared distance `d2` alone,
    /// the pass taking `pen` of the Edit Curve.
    fn fade(&self, d2: f32, reach: f32, fall: Fade, pen: f32) -> f32 {
        let d = d2.sqrt() / reach;
        if fall == Fade::Elastic {
            moves::cutoff(d)
        } else {
            self.tip.falloff_at(d, pen)
        }
    }

    /// The weight `w` of a point at `p` times the alpha's factor there, the picture's square
    /// spanning the pass's reach. Negative only below a signed alpha's mid value; the alpha is
    /// read only where `w` is not 0.
    fn stamp(&self, f: &Frame, reach: f32, p: Vec3, w: f32) -> f32 {
        match &self.alpha {
            Some(a) if w != 0.0 => w * alpha_at(a, f, reach, p),
            _ => w,
        }
    }

    /// The weight of vertex `v` at `p` in pass `f` (pass `i` of the plan, reach `reach`)
    /// without the alpha and the automasks.
    fn weight<const DEEP: bool>(
        &self,
        plan: &Plan<'_>,
        (i, f): (usize, &Frame),
        reach: f32,
        v: VertId,
        p: Vec3,
    ) -> f32 {
        if plan.fall == Fade::Edges {
            let start = &self.starts[f.pass];
            start
                .along_edges(v)
                .map_or(0.0, |g| self.tip.falloff_at(g / f.radius, f.pen))
        } else {
            self.fade(plan.gap2::<DEEP>(i, f.center, p), reach, plan.fall, f.pen)
        }
    }

    /// The Depth Mask's factor at `p` in pass `f` (decision 0053, section 1): the Depth Curve
    /// over the height along the area normal above the brush sphere's center, out to Outer
    /// Depth above it and Inner Depth below, and 0 past them.
    fn depth_mask(&self, f: &Frame, p: Vec3) -> f32 {
        let d = &self.preset.depth;
        let h = (p - (f.origin + f.normal * (f.imbed * f.radius))).dot(f.normal);
        let edge = f.radius * if h > 0.0 { d.outer } else { -d.inner };
        let a = h.abs();
        if a > edge {
            0.0
        } else if edge > 0.0 {
            d.curve.eval(a / edge)
        } else {
            d.curve.eval(0.0)
        }
    }

    /// Weight `w` (above 0) of vertex `v` with normal `n` in pass `f` after the automasks.
    fn automask(&self, s: SculptSurface<'_>, f: &Frame, v: VertId, n: Vec3, w: f32) -> f32 {
        let (start, am, mut w) = (&self.starts[f.pass], &self.preset.automask, w);
        if am.topology != 0.0
            && !start
                .along_edges(v)
                .is_some_and(|g| g <= am.topology * f.radius)
        {
            return 0.0;
        }
        if am.backface != 0.0 && n.dot(f.view) > 0.0 {
            w *= 1.0 - am.backface;
        }
        if am.face_group != 0.0
            && let (Some(g), Some(col)) = (start.group, self.cols.group)
            && !(s.topo().ring(v).iter()).any(|&f| s.attr_reader(col).label(f) == g)
        {
            w *= 1.0 - am.face_group;
        }
        w
    }

    /// The factor every pass shares: 0 for a hidden vertex (the store's derived bit), `1 -
    /// mask`, and the cavity automask. A mask value that is not a finite number masks fully:
    /// `clamp` would pass NaN on, and a NaN position marks a dead slot (review 0002, M5; import
    /// refuses such values too). `mask` is the node's [`Ctx::mask`], `n` the vertex's normal.
    pub fn common(
        &self,
        s: SculptSurface<'_>,
        node: &NodeView<'_>,
        mask: Option<&[f32]>,
        l: usize,
        n: Vec3,
    ) -> f32 {
        if node.hidden(l) {
            return 0.0;
        }
        let mut m = 1.0;
        if let Some(mask) = mask {
            let x = mask[l];
            m = if x.is_finite() {
                1.0 - x.clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
        let c = self.preset.automask.cavity;
        if c != 0.0 {
            m *= 1.0 - cavity_held(s, node.base + l as u32, n, c);
        }
        m
    }

    /// The nodes a brush with these columns can change: never all-hidden ones, and not
    /// all-masked ones when the mask holds it back.
    pub fn node_reach(&self) -> Reach {
        if self.cols.mask.is_some() {
            Reach::Unmasked
        } else {
            Reach::Visible
        }
    }

    /// Weights of vertex `l` of `node` in every pass of `plan` (made for `node` and `frames`),
    /// times the alpha and feathered, into `ws`, and the common factor; `None` when nothing
    /// touches the vertex. A brush with neither the Depth Mask nor Infinite Depth runs a loop
    /// without them, the one it ran before them (decision 0053).
    #[inline]
    pub fn influence(
        &self,
        s: SculptSurface<'_>,
        node: &NodeView<'_>,
        plan: &Plan<'_>,
        l: usize,
        frames: &[Frame],
        ws: &mut [f32],
    ) -> Option<f32> {
        if plan.deep {
            self.weigh::<true>(s, node, plan, l, frames, ws)
        } else {
            self.weigh::<false>(s, node, plan, l, frames, ws)
        }
    }

    /// [`Ctx::influence`], with the Depth Mask and Infinite Depth read only when `DEEP`.
    #[inline]
    fn weigh<const DEEP: bool>(
        &self,
        s: SculptSurface<'_>,
        node: &NodeView<'_>,
        plan: &Plan<'_>,
        l: usize,
        frames: &[Frame],
        ws: &mut [f32],
    ) -> Option<f32> {
        let (v, p, n) = (node.base + l as u32, node.pos[l], node.nrm[l]);
        // Outside every pass's sphere every weight is 0: the squared distance, with room for
        // the rounding of the exact test in `fade`, rejects the point before any weight.
        let passes = frames.iter().enumerate().zip(&plan.passes);
        if passes
            .clone()
            .all(|((i, f), &(_, far))| plan.gap2::<DEEP>(i, f.center, p) > far)
        {
            return None;
        }
        for (w, ((i, f), &(reach, _))) in ws.iter_mut().zip(passes.clone()) {
            let mut x = self.weight::<DEEP>(plan, (i, f), reach, v, p);
            // The Depth Mask, a factor of its own before the automasks (decision 0053).
            if DEEP && self.preset.depth.mask && x > 0.0 {
                x *= self.depth_mask(f, p);
            }
            *w = if x <= 0.0 {
                0.0
            } else if plan.automask {
                self.automask(s, f, v, n, x)
            } else {
                x
            };
        }
        if self.alpha.is_some() {
            for (w, ((_, f), &(reach, _))) in ws.iter_mut().zip(passes) {
                *w = self.stamp(f, reach, p, *w);
            }
        }
        // Negative weights (a signed alpha below its mid value) carve, so only 0 stops here.
        if ws.iter().all(|&w| w == 0.0) {
            return None;
        }
        let m = self.common(s, node, plan.mask, l, n);
        // The feather is never above 1: below it, the passes overlap.
        if plan.feather < 1.0 {
            for w in ws.iter_mut() {
                *w *= plan.feather;
            }
        }
        // The common factor is never negative: `!= 0.0` means the vertex is touched.
        (m != 0.0).then_some(m)
    }
}

/// How much of full strength the cavity automask holds vertex `v`, whose normal is `n`, at
/// intensity `c` (decision 0056): the one-ring cavity measure's depth at contrast `100 * c`,
/// measured the other way for a negative `c`, so it holds peaks and never flat ground. Kept out
/// of line, like `alpha_at`, so the per-vertex loops of strokes without it compile as before.
#[inline(never)]
fn cavity_held(s: SculptSurface<'_>, v: VertId, n: Vec3, c: f32) -> f32 {
    thread_local! {
        /// The walk's lists, kept per thread so that measuring a point allocates nothing.
        static WALK: RefCell<Walk> = const { RefCell::new(Walk::new()) };
    }
    let cfg = s.config();
    let nrm = |u: VertId| {
        if u == v {
            n
        } else {
            s.node(cfg.node_of(u)).nrm[cfg.local_of(u) as usize]
        }
    };
    let reads = (&s.pos(), &s.topo(), s.visible());
    let f = WALK.with_borrow_mut(|w| cavity::measure(reads, nrm, v, 1, w));
    let f = if c < 0.0 { -f } else { f };
    cavity::depth(f, 100.0 * c.abs())
}

/// The alpha's factor at `p` in pass `f` with reach `reach`. Kept out of line, and applied
/// after the other weights, so the per-vertex loops of strokes without an alpha compile as
/// they did before alphas (inlined into the weights, it cost those loops up to 8%).
#[inline(never)]
fn alpha_at(a: &AlphaRead<'_>, f: &Frame, reach: f32, p: Vec3) -> f32 {
    let rel = p - f.center;
    let across = |axis: Vec3| rel.dot(axis) / reach;
    a.factor(f.level, across(f.tangent), across(f.bitangent))
}

/// What the influence stage reads that is the same for every point of a node, worked out once
/// per node instead of once per point ([`Ctx::plan`]). Nothing in it changes a result.
pub(crate) struct Plan<'a> {
    /// Per pass: its reach, and the squared distance past which every weight is 0 (the
    /// falloff and the elastic cutoff end at the reach). Infinite for Move Connected, whose
    /// weights go by distance along edges from the stroke's start.
    passes: Vec<(f32, f32)>,
    fall: Fade,
    mask: Option<&'a [f32]>,
    /// Whether any automask other than the cavity is on.
    automask: bool,
    /// Whether the Depth Mask or Infinite Depth is on: the influence stage's other loop.
    deep: bool,
    /// Per pass: Infinite Depth's free directions, distances measured across them; empty
    /// when it is off.
    free: Vec<Free>,
    /// Whether an axis lock or a mirror plane's clamp can change a translation.
    pub(super) clamps: bool,
    /// The dab's symmetry feather, every pass's weight scaled by it (decision 0040).
    feather: f32,
}

impl Plan<'_> {
    /// The squared distance of `p` from pass `i`'s center `c`: across the free directions
    /// when `DEEP` and Infinite Depth is on, else straight.
    #[inline]
    fn gap2<const DEEP: bool>(&self, i: usize, c: Vec3, p: Vec3) -> f32 {
        if DEEP && let Some(e) = self.free.get(i) {
            e.across(p - c).len2()
        } else {
            (p - c).len2()
        }
    }
}

/// How a brush's weights fall off with a point's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fade {
    /// The tip's falloff over the straight distance.
    Tip,
    /// Move Elastic: the Kelvinlet's cutoff over the straight distance.
    Elastic,
    /// Move Connected: the tip's falloff over the distance along edges from the start.
    Edges,
}
