//! Step 4 of a pass (decision 0021, section 2): collapse the short edges, shortest first.

use std::cmp::Ordering;

use crate::config::{FaceId, VertId};
use crate::math::{Vec3, face_cross, face_quality, leans};
use crate::mirror;
use crate::relax::THIN;
use crate::topo::Edge;

use super::{Key, Pass};
use crate::live::LiveReport;

/// A collapse leaves its kept point at most this many neighbours.
const MOST_VALENCE: usize = 20;

/// A planned collapse: the end kept, how far toward the other it moves (0 to 1), and whether
/// the kept and the removed end are feature points (each found once per try).
#[derive(Clone, Copy)]
struct Plan {
    keep: VertId,
    t: f32,
    keep_feature: bool,
    gone_feature: bool,
}

impl Pass<'_, '_> {
    pub(super) fn collapse_all(&mut self, edges: &[(VertId, VertId)], out: &mut LiveReport) {
        let mut short: Vec<Key> = (edges.iter())
            .filter(|&&(a, b)| self.short_pair(a, b))
            .map(|&(a, b)| self.key(a, b))
            .collect();
        short.sort_unstable();
        short.dedup();
        for (_, a, b) in short {
            if self.hit {
                return;
            }
            if !(self.ed.is_live(a) && self.ed.is_live(b)) || self.touches_hidden(a, b) {
                continue;
            }
            // A long thin face touching a sphere can hold a short edge far from it: that one
            // stays, so no point outside the spheres grown by `max` moves or goes.
            if !(self.reach(a) && self.reach(b)) {
                continue;
            }
            let Some(e) = self.ed.edge(a, b) else {
                continue;
            };
            if self.short_pair(a, b) {
                self.try_collapse(e, out);
            }
        }
    }

    /// Whether edge `e` lies along a border: an open one, or between two face groups.
    fn along_border(&self, e: &Edge) -> bool {
        e.g.is_none_or(|g| self.group(g) != self.group(e.f))
    }

    /// Which end of `e` a collapse keeps and how far toward the other it moves: a feature point
    /// stays put; otherwise the more masked end, moving half way when unmasked.
    fn plan_collapse(&self, e: &Edge) -> Plan {
        let (a, b) = (e.a, e.b);
        let (fa, fb) = (self.feature(a), self.feature(b));
        let (ma, mb) = (self.mask(a), self.mask(b));
        let keep = match (fa, fb) {
            (true, false) => a,
            (false, true) => b,
            _ => match ma.total_cmp(&mb) {
                Ordering::Greater => a,
                Ordering::Less => b,
                Ordering::Equal => a.min(b),
            },
        };
        let (fk, fg, mk) = if keep == a {
            (fa, fb, ma)
        } else {
            (fb, fa, mb)
        };
        Plan {
            keep,
            t: if fk { 0.0 } else { 0.5 * (1.0 - mk) },
            keep_feature: fk,
            gone_feature: fg,
        }
    }

    /// The same collapse on `e2`, the image of the planned edge: it keeps `keep`, as far along.
    fn image_plan(&self, e2: &Edge, keep: VertId, t: f32) -> Plan {
        let gone = if keep == e2.a { e2.b } else { e2.a };
        Plan {
            keep,
            t,
            keep_feature: self.feature(keep),
            gone_feature: self.feature(gone),
        }
    }

    /// Where a collapse of `e` by `plan` puts the kept point.
    fn collapse_point(&self, e: &Edge, plan: Plan) -> Vec3 {
        let gone = if plan.keep == e.a { e.b } else { e.a };
        let kp = self.ed.pos(plan.keep);
        kp + (self.ed.pos(gone) - kp) * plan.t
    }

    /// Whether `e` may collapse by `plan` with the kept point at `p`: no feature point removed
    /// (but along a border) or moved, no hidden face changed, the link condition, the kept
    /// point's valence and edges, and no face leaning or thinned past [`THIN`].
    fn collapse_ok(&self, e: &Edge, plan: Plan, p: Vec3) -> bool {
        let keep = plan.keep;
        let gone = if keep == e.a { e.b } else { e.a };
        if plan.gone_feature && !(plan.keep_feature && self.along_border(e)) {
            return false;
        }
        if plan.keep_feature && (p - self.ed.pos(keep)).len2() > 0.0 {
            return false;
        }
        if self.ed.ring_ref(gone).iter().any(|&f| self.hidden(f)) || !self.ed.can_collapse(e) {
            return false;
        }
        let mut around: Vec<VertId> = [self.ed.neighbours_of(keep), self.ed.neighbours_of(gone)]
            .concat()
            .into_iter()
            .filter(|&u| u != keep && u != gone)
            .collect();
        around.sort_unstable();
        around.dedup();
        if around.len() > MOST_VALENCE {
            return false;
        }
        // Two ends at one point, kept there: no face changes shape and every new edge is as
        // long as one the removed end had, so nothing is checked further. This removes the
        // faces without area of degenerate input, which the face checks below would refuse,
        // having no normal to lean from (review 0006, S3).
        if p == self.ed.pos(keep) && p == self.ed.pos(gone) {
            return true;
        }
        let long = |u: VertId| {
            let w = self.free(keep, u);
            (p - self.ed.pos(u)).len2() * w * w > self.max2
        };
        if around.iter().any(|&u| long(u)) {
            return false;
        }
        let edge_faces = [Some(e.f), e.g];
        let mut faces: Vec<FaceId> = [self.ed.ring(keep), self.ed.ring(gone)].concat();
        faces.retain(|f| !edge_faces.contains(&Some(*f)));
        let at = |u: VertId| {
            if u == keep || u == gone {
                p
            } else {
                self.ed.pos(u)
            }
        };
        let now = |u: VertId| self.ed.pos(u);
        for f in faces {
            let fc = self.ed.face(f);
            let (old, new) = (face_cross(fc, now), face_cross(fc, at));
            if leans(old, new) {
                return false;
            }
            let q = face_quality(fc, at);
            if q < THIN && q < face_quality(fc, now) {
                return false;
            }
        }
        true
    }

    /// Collapse `e` into `keep` at `p`, which [`Pass::collapse_ok`] allowed and was paid for.
    fn collapse(&mut self, e: Edge, keep: VertId, p: Vec3, out: &mut LiveReport) {
        let far: Vec<VertId> = ([Some(e.f), e.g].iter().flatten())
            .map(|&f| self.ed.far_corner(f, e.a, e.b))
            .collect();
        self.ed.collapse(e, keep, p);
        out.collapses += 1;
        out.removed += 1;
        self.changed.push(keep);
        self.changed.extend(far);
    }

    /// Collapse `e` if every check allows it, with its mirror image when there is a map: the
    /// image keeps the kept point's partner, as far along. An edge whose ends are partners
    /// collapses onto the plane; one lying in the plane is a feature edge and never does; an
    /// edge too near its image (one point around both) waits, as collapsing one could change
    /// the checks on the other.
    fn try_collapse(&mut self, e: Edge, out: &mut LiveReport) {
        let plan = self.plan_collapse(&e);
        let (keep, p) = (plan.keep, self.collapse_point(&e, plan));
        let Some((axis, col)) = self.mirror else {
            if self.collapse_ok(&e, plan, p) && self.spend(1) {
                self.collapse(e, keep, p, out);
            }
            return;
        };
        let Some((c, d)) = self.mirrored(e.a, e.b) else {
            if self.in_plane(e.a, e.b) {
                return;
            }
            let p = mirror::onto(p, axis);
            if self.collapse_ok(&e, plan, p) && self.spend(1) {
                self.collapse(e, keep, p, out);
                self.ed.set_partner(col, keep, keep);
            }
            return;
        };
        if !self.apart(e.a, e.b, c, d) || self.touches_hidden(c, d) {
            return;
        }
        let Some(e2) = self.ed.edge(c, d) else {
            return;
        };
        let plan2 = self.image_plan(&e2, self.twin(keep), plan.t);
        let p2 = self.collapse_point(&e2, plan2);
        if self.collapse_ok(&e, plan, p) && self.collapse_ok(&e2, plan2, p2) && self.spend(2) {
            self.collapse(e, keep, p, out);
            self.collapse(e2, plan2.keep, p2, out);
        }
    }
}
