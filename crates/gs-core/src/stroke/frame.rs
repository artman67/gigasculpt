//! The dab frame (area normal and area center over the sampling radius) and what each symmetry
//! pass learns at the first dab of a stroke: what every stroke class reads before its kernel.

use super::Stroke;
use crate::brush::{BrushPreset, KernelSpec, Sampling, Select};
use crate::config::VertId;
use crate::kernel::Reach;
use crate::kernels::{Cols, Frame, PassStart, flood};
use crate::math::Vec3;
use crate::store::MeshStore;
use crate::symmetry::Xform;
use crate::view::{PosRead, TopoRead};

/// The surface a dab frame samples.
#[derive(Clone, Copy)]
pub(super) enum Surface<'a> {
    /// The store: as it is now when the frame accumulates, else as the stroke started (the
    /// open step's snapshot, by the same ids).
    Store(&'a MeshStore),
    /// A share of the store taken as a refining stroke started, read as it is (decision 0021,
    /// section 5).
    Start(&'a MeshStore),
}

/// Area normal and area center around `c` for a brush of radius `r` (specified in
/// `crate::brush`), from visible vertices only, and, when `count` is set (a stroke with an
/// alpha), how many visible vertices lie within the larger sampling radius, facing or not (for
/// the alpha's level; 0 otherwise). Per-node sums are combined in node order, so the result
/// does not depend on the thread count.
pub(super) fn sample(
    surface: Surface<'_>,
    c: Vec3,
    r: f32,
    view: Vec3,
    sm: &Sampling,
    count: bool,
) -> (Vec3, Vec3, usize) {
    let (rn, rp) = (sm.normal_radius * r, sm.position_radius * r);
    let reach = rn.max(rp);
    let (store, (pos, nrm)) = match surface {
        Surface::Start(s) => (s, (s.pos(), s.nrm())),
        Surface::Store(s) if sm.accumulate => (s, (s.pos(), s.nrm())),
        Surface::Store(s) => (s, (s.start_pos(), s.start_nrm())),
    };
    let (nodes, _) = store.reachable(&store.query(c, reach).brushed, Reach::Visible);
    let parts = gs_par::map(nodes.len(), |i| {
        let node = store.node(nodes[i]);
        let mut acc = (Vec3::ZERO, Vec3::ZERO, 0.0f32, 0usize);
        for l in 0..node.pos.len() {
            let v = node.base + l as u32;
            let (p, n) = (pos.get(v), nrm.get(v));
            if p.is_dead() || node.hidden(l) {
                continue;
            }
            if count {
                acc.3 += usize::from((p - c).len2() <= reach * reach);
            }
            if sm.front_only && n.dot(view) >= 0.0 {
                continue;
            }
            // Weights fall to zero at each radius and stay there beyond it.
            let d = (p - c).len2().sqrt();
            let (wn, wp) = ((1.0 - d / rn).max(0.0), (1.0 - d / rp).max(0.0));
            acc.0 += n * wn;
            acc.1 += p * wp;
            acc.2 += wp;
        }
        acc
    });
    let (sn, sp, sw, near) = (parts.into_iter()).fold((Vec3::ZERO, Vec3::ZERO, 0.0, 0), |a, b| {
        (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3)
    });
    let normal = if sn.len2() > 0.0 {
        sn.normalized()
    } else {
        Vec3::ZERO - view
    };
    let origin = if sw > 0.0 { sp * (1.0 / sw) } else { c };
    (normal, origin, near)
}

/// Two unit axes across `normal`: the first along `dir` within the surface, or, when `dir`
/// has (almost) no such part, along the coordinate axis most across the normal (the first
/// such axis on a tie). "Almost" is a part under a hundred-thousandth of `dir`'s length, so
/// rounding never turns a direction along the normal into an arbitrary axis.
pub(super) fn axes(normal: Vec3, dir: Vec3) -> (Vec3, Vec3) {
    let across = |e: Vec3| normal.cross(e).cross(normal);
    let mut t = across(dir);
    if t.len2() <= 1e-10 * dir.len2() {
        let a = normal.to_array().map(f32::abs);
        let k = (0..3).min_by(|&i, &j| a[i].total_cmp(&a[j])).unwrap_or(0);
        let mut e = [0.0; 3];
        e[k] = 1.0;
        t = across(Vec3::from_array(e));
    }
    let t = t.normalized();
    (t, normal.cross(t))
}

/// Pass `pass`'s copy of the dab `f`, placed by `at` with its directions turned by `dir`
/// (decision 0044), then leaned by Tilt Brush. Where the two maps are one (Mirror and Radial),
/// it is `f` mapped by it. A Parallel copy turns its step or grab (`delta`) and its aim by
/// `dir` and builds its alpha's axes the way the dab built its own, at the copy's place: along
/// the aim within the copy's surface, or the coordinate-axis fallback with none, its bitangent
/// turned over only where `dir` mirrors.
pub(super) fn copy(f: &Frame, at: Xform, dir: Xform, pass: usize) -> Frame {
    let mut c = f.moved(at, pass);
    if at != dir {
        (c.delta, c.aim) = (dir.apply(f.delta), dir.apply(f.aim));
        let (t, b) = axes(c.normal, c.aim);
        (c.tangent, c.bitangent, c.xf) = (t, b * dir.det(), dir);
    }
    lean(c)
}

/// `c` leaned by its tilt about its second axis (decision 0058, section 1): the normal turns
/// toward the first axis, and the first axis with it. The first axis is first made square to
/// the normal, which a held stroke plane's normal need not be.
fn lean(mut c: Frame) -> Frame {
    if c.tilt == 0.0 {
        return c;
    }
    let t = c.tangent - c.normal * c.normal.dot(c.tangent);
    if t.len2() > 1e-10 {
        let (t, s, k) = (t.normalized(), libm::sinf(c.tilt), libm::cosf(c.tilt));
        (c.normal, c.tangent) = (c.normal * k + t * s, t * k - c.normal * s);
    }
    c
}

/// The live visible vertex nearest `c` among the nodes a sphere of radius `r` there touches,
/// the lowest id on a tie.
fn nearest(store: &MeshStore, c: Vec3, r: f32) -> Option<VertId> {
    let (nodes, _) = store.reachable(&store.query(c, r).brushed, Reach::Visible);
    let all = nodes.iter().flat_map(|&n| {
        let node = store.node(n);
        (node.pos.iter().enumerate())
            .filter(move |&(l, _)| !node.hidden(l))
            .map(move |(l, &p)| ((p - c).len2(), node.base + l as u32))
    });
    all.filter(|e| !e.0.is_nan())
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|e| e.1)
}

/// What each pass needs from the stroke start: the first frame, the face group under it, and
/// the vertices joined to it along edges (for the connected move and the topology automask).
pub(super) fn starts(
    store: &MeshStore,
    first: &Frame,
    passes: &[Xform],
    preset: &BrushPreset,
    cols: Cols,
) -> Vec<PassStart> {
    let am = &preset.automask;
    let connected = matches!(&preset.kernel, KernelSpec::Move(m) if m.select == Select::Connected);
    let reach = if connected { 1.0f32 } else { 0.0 }.max(am.topology) * first.radius;
    let s = store.surface();
    let group = cols.group.filter(|_| am.face_group != 0.0);
    passes
        .iter()
        .enumerate()
        .map(|(i, &xf)| {
            let f = first.moved(xf, i);
            let near = nearest(store, f.center, first.radius);
            let ring = |v: VertId| {
                let visible = s.visible();
                s.topo().ring(v).iter().copied().find(|&f| visible.face(f))
            };
            PassStart {
                normal: f.normal,
                origin: f.origin,
                group: group
                    .zip(near.and_then(ring))
                    .map(|(g, fc)| s.attr_reader(g).label(fc)),
                region: near.map_or_else(Vec::new, |v| flood(store, v, reach)),
            }
        })
        .collect()
}

impl Stroke {
    /// The frame around `center` with only the sampled parts filled in, and the alpha level
    /// for the vertex spacing there (specified in `crate::brush`).
    pub(super) fn frame(&mut self, store: &MeshStore, center: Vec3, r: f32, dir: Vec3) -> Frame {
        let view = self.params.view.dir;
        let sm = self.preset.sampling;
        let count = self.params.alpha.is_some();
        let surface = match &self.start_view {
            Some(start) if !sm.accumulate => Surface::Start(start),
            _ => Surface::Store(store),
        };
        // Sampled at the pass image with the greatest coordinates and mirrored back, so a
        // stroke and its mirror image read the same points in the same order and get the
        // same frames bit for bit (decision 0021, L4). Radial images are not exact; they stay.
        let to = (self.passes.iter().copied())
            .filter(|_| self.params.symmetry.radial.is_none())
            .reduce(|best, xf| {
                let (a, b) = (xf.apply(center).to_array(), best.apply(center).to_array());
                let order = (0..3).map(|k| a[k].total_cmp(&b[k])).find(|o| o.is_ne());
                if order == Some(std::cmp::Ordering::Greater) {
                    xf
                } else {
                    best
                }
            })
            .unwrap_or(Xform::IDENTITY);
        let (normal, origin, near) =
            sample(surface, to.apply(center), r, to.apply(view), &sm, count);
        let (normal, origin) = (to.inverse().apply(normal), to.inverse().apply(origin));
        if let (Some(pyramid), Some(spec)) = (&self.params.alpha, &self.preset.tip.alpha) {
            let reach = sm.normal_radius.max(sm.position_radius) * r;
            let spacing = reach * (std::f32::consts::PI / near.max(1) as f32).sqrt();
            self.level = pyramid.level_for(r, spacing * spec.adaptive, spec.tiles);
        }
        let dir = self.follow(normal, dir, r);
        self.bare(center, r, normal, origin, dir)
    }

    /// The direction a dab of radius `r` lays its first axis along, given its `step` and its
    /// surface's `normal` (decision 0058, sections 2 and 3): the step at Align To Path 100,
    /// else the stroke's aim, the direction the previous dab followed. The first step within
    /// the surface sets the aim; Constant Tilt holds it, and below 100 each later step turns
    /// it toward itself, about the normal, by `1 - exp(-s / L)` of the angle between them,
    /// `L = 2r (1 - align / 100)`. A dab with no step within the surface keeps the aim.
    fn follow(&self, normal: Vec3, step: Vec3, r: f32) -> Vec3 {
        let tip = &self.preset.tip;
        if tip.align >= 100.0 && !tip.constant_tilt {
            return step;
        }
        let across = |v: Vec3| normal.cross(v).cross(normal);
        let none = |v: Vec3| across(v).len2() <= 1e-10 * v.len2();
        let held = self.prev.map(|(_, aim)| aim).filter(|&aim| !none(aim));
        let Some(held) = held else {
            return if none(step) {
                step
            } else {
                across(step).normalized()
            };
        };
        if tip.constant_tilt || none(step) {
            return held;
        }
        let (h, s) = (across(held).normalized(), across(step).normalized());
        let k = 1.0 - libm::expf(-step.len2().sqrt() / (2.0 * r * (1.0 - tip.align / 100.0)));
        let turn = k * libm::atan2f(normal.dot(h.cross(s)), h.dot(s));
        h * libm::cosf(turn) + normal.cross(h) * libm::sinf(turn)
    }

    /// A frame from a known normal and center.
    pub(super) fn bare(
        &self,
        center: Vec3,
        r: f32,
        normal: Vec3,
        origin: Vec3,
        dir: Vec3,
    ) -> Frame {
        let (tangent, bitangent) = axes(normal, dir);
        Frame {
            center,
            radius: r,
            amount: 0.0,
            strength: 0.0,
            normal,
            origin,
            view: self.params.view.dir,
            tangent,
            bitangent,
            delta: Vec3::ZERO,
            imbed: 0.0,
            modifier: None,
            level: self.level,
            aim: dir,
            tilt: self.preset.tip.tilt.to_radians(),
            pen: 1.0,
            xf: Xform::IDENTITY,
            pass: 0,
        }
    }

    /// Learn what every pass needs from the stroke start.
    pub(super) fn start(&mut self, store: &MeshStore, first: &Frame) {
        self.starts = starts(store, first, &self.passes, &self.preset, self.cols);
        self.report.walked = self.starts.iter().map(|s| s.region.len()).sum();
    }
}
