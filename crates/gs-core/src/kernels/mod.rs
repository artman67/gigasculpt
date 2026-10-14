//! The five kernels (ARCHITECTURE 4.1) and the stages around them that every brush shares: the
//! influence factors (`influence`, the one home of hide, mask, automask, alpha and the symmetry
//! feather), the kernel summed over symmetry passes, then the plane clamp and axis lock. What
//! each kernel computes is specified in `crate::brush`.

mod attribute;
mod influence;
mod moves;
mod offset;
mod paint;
mod plane;
mod rounds;
mod smooth;

pub(crate) use attribute::paint;
use influence::Plan;
pub(crate) use influence::{Cols, Ctx};
pub(crate) use moves::flood;
pub(crate) use paint::{Paint, PaintBuffer, blend};
use rounds::Rounds;

use std::ops::Range;

use crate::brush::{KernelSpec, OffsetTerm, SmoothMode};
use crate::config::{NodeId, VertId};
use crate::kernel::{Free, Kernel, Reach, Scratch, SculptSurface};
use crate::math::Vec3;
use crate::par::merge;
use crate::store::MeshStore;
use crate::symmetry::Xform;
use crate::view::{NodeView, PosRead};

/// One symmetry pass of a dab, in that pass's own orientation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub center: Vec3,
    pub radius: f32,
    /// Strength times the stroke length this dab stands for, in radii.
    pub amount: f32,
    /// Strength times the pressure curve: the kernels that are not per length (move, hook).
    pub strength: f32,
    /// Area normal and area center.
    pub normal: Vec3,
    pub origin: Vec3,
    pub view: Vec3,
    /// The alpha's axes across the dab.
    pub tangent: Vec3,
    pub bitangent: Vec3,
    /// Move: the pointer's offset from the grab. Hook: the step from the previous dab.
    pub delta: Vec3,
    /// Plane: the plane's height above the area center, in radii, after its pressure curve.
    pub imbed: f32,
    /// Offset: the term the Brush Modifier adds at this dab's pressure, if any.
    pub modifier: Option<OffsetTerm>,
    /// The alpha level this dab reads (every pass reads the same).
    pub level: usize,
    /// The direction the alpha's first axis lies along, which a Parallel copy turns: the step,
    /// or the stroke's aim with Constant Tilt or below Align To Path 100 (decision 0058).
    pub aim: Vec3,
    /// Tilt Brush's lean in radians, made as each pass is copied (decision 0058).
    pub tilt: f32,
    /// Curve By Pen's share of the Edit Curve at this dab's pressure, 1 with it off (decision
    /// 0059).
    pub pen: f32,
    pub xf: Xform,
    pub pass: usize,
}

impl Frame {
    /// This frame mapped by a symmetry pass.
    pub fn moved(&self, xf: Xform, pass: usize) -> Frame {
        let m = |v: Vec3| xf.apply(v);
        Frame {
            center: m(self.center),
            normal: m(self.normal),
            origin: m(self.origin),
            view: m(self.view),
            tangent: m(self.tangent),
            bitangent: m(self.bitangent),
            delta: m(self.delta),
            aim: m(self.aim),
            xf,
            pass,
            ..*self
        }
    }
}

/// What a stroke learned at its first dab, per symmetry pass.
#[derive(Clone, Debug, Default)]
pub(crate) struct PassStart {
    pub normal: Vec3,
    pub origin: Vec3,
    /// Face group under the start, for the face group automask.
    pub group: Option<u32>,
    /// Vertices joined to the start along edges, ascending, with their distance along edges.
    pub region: Vec<(VertId, f32)>,
}

impl PassStart {
    fn along_edges(&self, v: VertId) -> Option<f32> {
        let i = self.region.binary_search_by_key(&v, |e| e.0).ok()?;
        Some(self.region[i].1)
    }
}

/// One dab of a position kernel over all its passes.
pub(crate) struct DabKernel<'a> {
    pub ctx: &'a Ctx<'a>,
    pub frames: &'a [Frame],
    pub spec: &'a KernelSpec,
    /// Add the preset's gravity (the main pass, not the auto-smooth pass).
    pub gravity: bool,
}

impl Kernel for DabKernel<'_> {
    fn spheres(&self) -> Vec<(Vec3, f32)> {
        let reach = |f: &Frame| (f.center, self.ctx.reach(f));
        self.frames.iter().map(reach).collect()
    }

    fn free(&self) -> Vec<Free> {
        self.frames
            .iter()
            .filter_map(|f| self.ctx.free(f))
            .collect()
    }

    fn reach(&self) -> Reach {
        self.ctx.node_reach()
    }

    /// Smooth reads neighbours, and so does the cavity automask; the influence stage's other
    /// cross-node reads are topology and attributes.
    fn own_only(&self) -> bool {
        !matches!(self.spec, KernelSpec::Smooth(_)) && self.ctx.preset.automask.cavity == 0.0
    }

    fn deltas(&self, s: &SculptSurface<'_>, node: &NodeView<'_>, out: &mut [Vec3]) {
        self.part_deltas(s, node, 0..node.pos.len() as u32, out);
    }

    fn parts(&self) -> bool {
        true
    }

    /// The kernel is chosen once per part: each kind gets its own copy of the slot loop with
    /// its translation inlined.
    fn part_deltas(
        &self,
        s: &SculptSurface<'_>,
        node: &NodeView<'_>,
        slots: Range<u32>,
        out: &mut [Vec3],
    ) {
        let (s, invert) = (*s, self.ctx.invert);
        let at = (s, node, slots);
        match self.spec {
            KernelSpec::Offset(k) => self.each(at, out, |f, _, p, n, w, _| {
                offset::offset(k, f, p, n, w, invert)
            }),
            KernelSpec::Plane(k) => {
                self.each(at, out, |f, _, p, _, w, _| plane::plane(k, f, p, w, invert));
            }
            KernelSpec::Smooth(k) if k.plain() => {
                let rate = (k.mode, k.gain);
                self.each(at, out, |f, v, p, n, w, nb| {
                    let d = smooth::smooth(rate, s, f, v, p, n, w, nb);
                    if invert { -d } else { d }
                });
            }
            KernelSpec::Smooth(k) => {
                let group = self.ctx.cols.group;
                let fresh = k.projection && !s.start_topology_changed(node.id);
                let start = s.start_nrm();
                self.each(at, out, |f, v, p, n, w, nb| {
                    let n0 = fresh.then(|| start.get(v));
                    let d = smooth::modes::smooth(*k, s, f, v, p, n, w, nb, group, n0);
                    if invert { -d } else { d }
                });
            }
            KernelSpec::Move(k) => {
                self.each(at, out, |f, _, p, _, w, _| moves::drag(k, f, p, w));
            }
            KernelSpec::Hook(k) => {
                self.each(at, out, |f, _, p, _, w, _| moves::hook(*k, f, p, w));
            }
            KernelSpec::Attribute(_) => self.each(at, out, |_, _, _, _, _, _| Vec3::ZERO),
        }
    }
}

impl DabKernel<'_> {
    /// Runs the dab on `store` and returns the nodes it changed. The Smooth kernel runs as
    /// [`smooth::sub_steps`] sub-steps (every node, every symmetry pass), each with its share of
    /// the amount (decision 0036, section 2): as the rounds of one step ([`Rounds`]) when its
    /// deltas read no normals and its spec is plain (decision 0049, section 6), else as whole
    /// runs. Every other kernel runs once.
    pub(crate) fn run(&self, store: &mut MeshStore, scratch: &mut Scratch) -> Vec<NodeId> {
        let n = match self.spec {
            KernelSpec::Smooth(k) => (self.frames.iter())
                .map(|f| smooth::sub_steps(k.gain, f.amount))
                .max()
                .unwrap_or(1),
            _ => 1,
        };
        if n == 1 {
            return store.run_kernel(self, scratch).brushed;
        }
        let share = |f: &Frame| Frame {
            amount: f.amount / n as f32,
            ..*f
        };
        let frames: Vec<Frame> = self.frames.iter().map(share).collect();
        let step = DabKernel {
            frames: &frames,
            ..*self
        };
        let am = &self.ctx.preset.automask;
        if let KernelSpec::Smooth(k) = self.spec
            && k.plain()
            && matches!(k.mode, SmoothMode::Uniform | SmoothMode::Stronger)
            && am.backface == 0.0
            && am.cavity == 0.0
        {
            let rounds = Rounds::new(&step, *k);
            return store.run_steps(&rounds, n, scratch).brushed;
        }
        let mut nodes = Vec::new();
        for _ in 0..n {
            nodes = merge(&nodes, &store.run_kernel(&step, scratch).brushed);
        }
        nodes
    }

    /// The slot loop of one kernel over the slots `slots` of `node` (with `out[i]` for slot
    /// `slots.start + i`): influence, `kernel` summed over the passes with the preset's
    /// gravity, then the plane clamp and axis lock.
    fn each<K>(
        &self,
        (s, node, slots): (SculptSurface<'_>, &NodeView<'_>, Range<u32>),
        out: &mut [Vec3],
        kernel: K,
    ) where
        K: Fn(&Frame, VertId, Vec3, Vec3, f32, &mut smooth::Scratch) -> Vec3,
    {
        let mut nb = smooth::Scratch::new(node, slots.clone());
        let ls = (slots.start as usize..).zip(out);
        self.slots(s, node, kernel).run(ls, &mut nb, |o, t| {
            if let Some(t) = t {
                *o = t;
            }
        });
    }

    /// The slot loop's state over `node` with `kernel`, worked out once per node or part.
    fn slots<'s, 'n, K>(
        &'s self,
        s: SculptSurface<'s>,
        node: &'s NodeView<'n>,
        kernel: K,
    ) -> SlotLoop<'s, 'n, K> {
        let ctx = self.ctx;
        SlotLoop {
            dab: self,
            s,
            node,
            plan: ctx.plan(node, self.frames),
            gravity: self.gravity && ctx.preset.post.gravity != 0.0,
            ws: vec![0.0; self.frames.len()],
            kernel,
        }
    }
}

/// What the slot loop of one kernel reads at every slot of a node or part: the plan, the
/// passes' weights and the kernel's translation.
struct SlotLoop<'s, 'n, K> {
    dab: &'s DabKernel<'s>,
    s: SculptSurface<'s>,
    node: &'s NodeView<'n>,
    plan: Plan<'n>,
    gravity: bool,
    ws: Vec<f32>,
    kernel: K,
}

impl<K> SlotLoop<'_, '_, K>
where
    K: Fn(&Frame, VertId, Vec3, Vec3, f32, &mut smooth::Scratch) -> Vec3,
{
    /// `each(x, translation)` for every slot `l` of `ls` (with `x` beside it), the translation
    /// `None` where the dab does not touch it: the kernel summed over the passes with the
    /// preset's gravity, then the plane clamp and axis lock. `nb` is the node's or part's
    /// Smooth scratch. The state is read into locals first, so the loop keeps it in registers.
    #[inline]
    fn run<X>(
        &mut self,
        ls: impl Iterator<Item = (usize, X)>,
        nb: &mut smooth::Scratch,
        mut each: impl FnMut(X, Option<Vec3>),
    ) {
        let SlotLoop {
            dab,
            s,
            node,
            plan,
            gravity,
            ws,
            kernel,
        } = self;
        let (s, node, gravity, frames) = (*s, *node, *gravity, dab.frames);
        let (ctx, post) = (dab.ctx, &dab.ctx.preset.post);
        for (l, x) in ls {
            let Some(m) = ctx.influence(s, node, plan, l, frames, ws) else {
                each(x, None);
                continue;
            };
            let (v, p, n) = (node.base + l as u32, node.pos[l], node.nrm[l]);
            let mut d = Vec3::ZERO;
            for (f, &w) in frames.iter().zip(ws.iter()) {
                if w != 0.0 {
                    let w = w * m;
                    let mut t = kernel(f, v, p, n, w, nb);
                    if gravity {
                        let g = f.xf.apply(Vec3::from_array(post.gravity_dir).normalized());
                        t += g * (post.gravity * w.abs() * f.amount * f.radius);
                    }
                    d += t;
                }
            }
            each(
                x,
                Some(if plan.clamps {
                    ctx.sym.clamp(p, d, ctx.lock)
                } else {
                    d
                }),
            );
        }
    }
}
