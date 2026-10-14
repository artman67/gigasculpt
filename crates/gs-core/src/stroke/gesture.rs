//! Gesture strokes (ARCHITECTURE 4.2.1): a curve or an outline drawn on the screen, run once
//! when the stroke ends, through the whole depth of the model, ignoring the dab frame.
//!
//! The points lie on a plane facing the camera. A vertex or face center is carried along its
//! view ray onto that plane (straight along the view direction, or from the eye in
//! perspective) to see where it falls.
//!
//! - Clip (a curve over the Plane kernel, Clip Curve): along the path through the press, the
//!   points Alt taps in and the pen ([`curve_path`]), every vertex on the shadow side of the
//!   nearest segment is pushed onto the surface that segment sweeps along the view rays, times
//!   `1 - mask`. The shadow side is the left of the curve's direction as the camera sees it;
//!   an inverted stroke (Alt) flips it. Nothing is added or removed.
//! - Select (a closed outline over the hide column): the faces under the outline are shown and
//!   every other face hidden; inverted (Alt), the faces under it are hidden. The outline is the
//!   polygon through every point, or (Select Rect) the rectangle from the first point to the
//!   last, its sides square to the screen ([`rectangle`]). An outline with no area does nothing.
//!
//! Which faces count as under an outline is the "smart" rule (decision 0015):
//! if at least one face lies wholly inside, only the faces wholly inside count; otherwise
//! every face with a corner inside. The rule is applied once per symmetry pass, to the outline
//! mirrored or turned by that pass, and the passes are united.

use super::frame::axes;
use super::{Stroke, View, curve_path};
use crate::bits::Bits;
use crate::brush::{Column, KernelSpec, Shape, StrokeSpec};
use crate::config::NodeId;
use crate::kernel::{Kernel, Reach, Scratch, SculptSurface};
use crate::kernels::Ctx;
use crate::math::{NO, Vec3, arity};
use crate::select::FaceSet;
use crate::store::MeshStore;
use crate::symmetry::Xform;
use crate::view::{NodeView, PosRead};
use crate::visibility::VisibilityEdit;

/// The rectangle a drag from `a` to `b` draws, as its four corners from `a`'s on: corner to
/// corner; with `square`, both sides as long as the drag's longer part, the far corner on
/// `b`'s side; with `center`, `a` its middle and the far corner a corner. The engine's select
/// and the window's outline both draw it, so the outline shown is the one selected (decision
/// 0051, sections 3 and 4).
pub fn rectangle(a: [f32; 2], b: [f32; 2], square: bool, center: bool) -> [[f32; 2]; 4] {
    let mut b = b;
    if square {
        let d = [b[0] - a[0], b[1] - a[1]];
        let side = d[0].abs().max(d[1].abs());
        b = [a[0] + side.copysign(d[0]), a[1] + side.copysign(d[1])];
    }
    let a = if center {
        [a[0] - (b[0] - a[0]), a[1] - (b[1] - a[1])]
    } else {
        a
    };
    [a, [b[0], a[1]], b, [a[0], b[1]]]
}

/// The gesture's points and the camera that drew them.
struct Gesture {
    pts: Vec<Vec3>,
    view: View,
    /// Axes of the gesture plane.
    u: Vec3,
    w: Vec3,
    /// A select's closed outline, on the plane; empty for a clip.
    outline: Vec<[f32; 2]>,
}

impl Gesture {
    /// A curve through `pts`, on the plane's own axes.
    pub fn new(pts: Vec<Vec3>, view: View) -> Gesture {
        let (u, w) = axes(view.dir, Vec3::ZERO);
        Gesture {
            pts,
            view,
            u,
            w,
            outline: Vec::new(),
        }
    }

    /// A select's outline through `pts` as `stroke` shapes it: the polygon through them, or the
    /// rectangle from the first to the last, its sides along the screen whose up is `up` (the
    /// plane's own axes without one). `None` for an outline with no area: fewer than three
    /// points, or a rectangle with a side of zero.
    fn outline(pts: Vec<Vec3>, view: View, stroke: StrokeSpec, up: Option<Vec3>) -> Option<Self> {
        let StrokeSpec::Gesture {
            shape: Shape::Rect,
            square,
            center,
            ..
        } = stroke
        else {
            let mut g = Gesture::new(pts, view);
            g.outline = g.pts.iter().map(|&p| g.flat(p)).collect();
            return (g.outline.len() >= 3).then_some(g);
        };
        let (u, w) = axes(view.dir, up.unwrap_or(Vec3::ZERO));
        let mut g = Gesture {
            u,
            w,
            ..Gesture::new(pts, view)
        };
        let ends = (g.pts.first()).zip(g.pts.last());
        let (a, b) = ends.map(|(&a, &b)| (g.flat(a), g.flat(b)))?;
        let corners = rectangle(a, b, square, center);
        let [lo, hi] = [corners[0], corners[2]];
        if hi[0] - lo[0] == 0.0 || hi[1] - lo[1] == 0.0 {
            return None;
        }
        g.outline = corners.to_vec();
        Some(g)
    }

    /// Where `p` falls on the gesture plane, carried along its view ray, in plane
    /// coordinates; `None` behind the eye. Straight along the view, the plane's own axes
    /// already ignore depth.
    fn place(&self, p: Vec3) -> Option<[f32; 2]> {
        let d = self.view.dir;
        match self.view.eye {
            None => Some(self.flat(p)),
            Some(e) => {
                let along = (p - e).dot(d);
                let scale = (self.pts[0] - e).dot(d) / along;
                along
                    .is_sign_positive()
                    .then(|| self.flat(e + (p - e) * scale))
            }
        }
    }

    /// Coordinates on the gesture plane.
    fn flat(&self, q: Vec3) -> [f32; 2] {
        [q.dot(self.u), q.dot(self.w)]
    }

    /// The direction of the view ray through `a`.
    fn ray(&self, a: Vec3) -> Vec3 {
        self.view
            .eye
            .map_or(self.view.dir, |e| (a - e).normalized())
    }
}

/// Squared distance from `p` to the segment `a`-`b`, on the plane.
fn segment_dist2(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / len2.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    let (x, y) = (ap[0] - ab[0] * t, ap[1] - ab[1] * t);
    x * x + y * y
}

/// Whether `p` is inside the closed outline `poly`, by counting the edges that cross the ray
/// from `p` toward +x. A point level with a corner counts as below it, so each crossing is
/// counted once.
fn inside(poly: &[[f32; 2]], p: [f32; 2]) -> bool {
    let above = |q: [f32; 2]| (q[1] - p[1]).is_sign_positive();
    let mut odd = false;
    for (i, a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        if above(*a) != above(b) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            odd ^= (x - p[0]).is_sign_positive();
        }
    }
    odd
}

/// The clip as a kernel over every node.
struct Clip<'a> {
    g: &'a Gesture,
    ctx: &'a Ctx<'a>,
    /// Per segment: its start, its start on the plane, its end on the plane, and the normal of
    /// its swept surface pointing to the shadow side.
    cuts: Vec<(Vec3, [f32; 2], [f32; 2], Vec3)>,
}

impl Kernel for Clip<'_> {
    fn spheres(&self) -> Vec<(Vec3, f32)> {
        vec![(Vec3::ZERO, f32::INFINITY)]
    }

    fn reach(&self) -> Reach {
        self.ctx.node_reach()
    }

    fn deltas(&self, s: &SculptSurface<'_>, node: &NodeView<'_>, out: &mut [Vec3]) {
        let mask = self.ctx.mask(node);
        for (l, o) in out.iter_mut().enumerate() {
            let p = node.pos[l];
            let Some(q) = self.g.place(p) else {
                continue;
            };
            let dist = |c: &(Vec3, [f32; 2], [f32; 2], Vec3)| segment_dist2(q, c.1, c.2);
            let Some(&(a, _, _, n)) = self.cuts.iter().min_by(|x, y| dist(x).total_cmp(&dist(y)))
            else {
                continue;
            };
            // Only the shadow side (positive distance) moves.
            let h = (p - a).dot(n).max(0.0);
            *o = n * (-h * self.ctx.common(*s, node, mask, l, node.nrm[l]));
        }
    }
}

/// Clip Curve. Returns the nodes written.
fn clip(store: &mut MeshStore, g: &Gesture, ctx: &Ctx<'_>) -> Vec<NodeId> {
    let cuts = (g.pts.windows(2))
        .map(|s| {
            let n = (s[1] - s[0]).cross(g.ray(s[0])).normalized();
            let n = if ctx.invert { -n } else { n };
            let flat = |p: Vec3| g.place(p).unwrap_or_default();
            (s[0], flat(s[0]), flat(s[1]), n)
        })
        .collect();
    let k = Clip { g, ctx, cuts };
    store.run_kernel(&k, &mut Scratch::default()).brushed
}

impl Gesture {
    /// The faces under the outline by the smart rule, one pass per symmetry map in `passes`,
    /// united. Only visible faces choose the rule: a hidden face wholly inside does not make
    /// the visible faces around it need to be wholly inside too (review 0003 s2).
    fn faces_under(&self, store: &MeshStore, passes: &[Xform]) -> FaceSet {
        let poly = &self.outline;
        let (pos, visible) = (store.pos(), store.visible());
        let undo: Vec<Xform> = passes.iter().map(|xf| xf.inverse()).collect();
        // Per node and pass: the faces wholly inside, those with a corner inside, and whether
        // a visible face is wholly inside.
        let per = gs_par::map(store.node_count(), |k| {
            let node = store.node(k as NodeId);
            let len = node.faces.len();
            (undo.iter())
                .map(|back| {
                    let (mut whole, mut any) = (Bits::zeros(len), Bits::zeros(len));
                    let mut shown_whole = false;
                    let under = |u: u32| {
                        let q = self.place(back.apply(pos.get(u)));
                        q.is_some_and(|q| inside(poly, q))
                    };
                    for (j, fc) in node.faces.iter().enumerate().filter(|f| f.1[0] != NO) {
                        let corners = &fc[..arity(*fc)];
                        let n = corners.iter().filter(|&&u| under(u)).count();
                        whole.set(j, n == corners.len());
                        any.set(j, n > 0);
                        shown_whole |=
                            n == corners.len() && visible.face(node.first_face + j as u32);
                    }
                    (whole, any, shown_whole)
                })
                .collect::<Vec<_>>()
        });
        let holds_whole: Vec<bool> = (0..passes.len())
            .map(|p| per.iter().any(|node| node[p].2))
            .collect();
        let nodes = (per.into_iter())
            .map(|node| {
                let mut out = Bits::zeros(node.first().map_or(0, |n| n.0.len()));
                for (p, (whole, any, _)) in node.iter().enumerate() {
                    out.or(if holds_whole[p] { whole } else { any });
                }
                out
            })
            .collect();
        FaceSet::from_nodes(nodes)
    }
}

/// Select Rect (and any closed outline): show the faces under it and hide every other face, or,
/// inverted, hide the faces under it. Returns the nodes written.
fn select(store: &mut MeshStore, g: &Gesture, passes: &[Xform], invert: bool) -> Vec<NodeId> {
    let set = g.faces_under(store, passes);
    store.edit_visibility(if invert {
        VisibilityEdit::Hide(&set)
    } else {
        VisibilityEdit::Only(&set)
    })
}

impl Stroke {
    /// Run a gesture stroke's one action.
    pub(super) fn gesture(&mut self, store: &mut MeshStore) {
        let pts: Vec<Vec3> = if self.preset.stroke.takes_points() {
            let mut pts: Vec<Vec3> = curve_path(&self.path()).iter().map(|s| s.pos).collect();
            // Ends that meet would make a segment with no direction.
            pts.dedup();
            pts
        } else {
            self.held.iter().map(|s| s.pos).collect()
        };
        let (view, up) = (self.params.view, self.params.up);
        let nodes = match &self.preset.kernel {
            KernelSpec::Attribute(a) if a.column == Column::Hide => {
                let Some(g) = Gesture::outline(pts, view, self.preset.stroke, up) else {
                    return;
                };
                select(store, &g, &self.passes, self.params.invert)
            }
            KernelSpec::Plane(_) if pts.len() >= 2 => {
                clip(store, &Gesture::new(pts, view), &self.ctx())
            }
            _ => return,
        };
        self.note(&nodes);
    }
}
