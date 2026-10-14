//! Step 6 of a pass (decision 0021, section 2): turn edges that even out the valences.

use crate::collapse::Around;
use crate::config::VertId;
use crate::math::{NO, face_cross, face_quality, leans};
use crate::relax::THIN;
use crate::topo::{Edge, next, prev};

use super::{Key, Pass};
use crate::live::LiveReport;

impl Pass<'_, '_> {
    pub(super) fn flip_all(&mut self, out: &mut LiveReport) {
        self.changed.sort_unstable();
        self.changed.dedup();
        let live: Vec<VertId> = (self.changed.iter().copied())
            .filter(|&v| self.ed.is_live(v) && self.reach(v))
            .collect();
        let mut cands: Vec<Key> = Vec::new();
        for &v in &live {
            for u in self.ed.neighbours_of(v) {
                if self.reach(u) {
                    cands.push(self.key(v, u));
                }
            }
        }
        cands.sort_unstable();
        cands.dedup();
        for (_, a, b) in cands {
            if self.hit {
                return;
            }
            let Some(e) = self.ed.edge(a, b) else {
                continue;
            };
            // With a map, an edge turns with its image, both decided before either turns; one
            // crossing the plane is its own image and turns alone, one in the plane never.
            let e2 = match self.mirrored(a, b).map(|(c, d)| self.ed.edge(c, d)) {
                Some(Some(e2)) if self.flips_apart(&e, &e2) => Some(e2),
                Some(_) => continue,
                None if self.in_plane(a, b) => continue,
                None => None,
            };
            if !(self.should_flip(&e) && e2.as_ref().is_none_or(|e2| self.should_flip(e2))) {
                continue;
            }
            if !self.spend(1 + u32::from(e2.is_some())) {
                return;
            }
            for e in [Some(e), e2].into_iter().flatten() {
                let (c, d) = self.ed.wings(&e).unwrap_or((NO, NO));
                self.ed.flip(e);
                out.flips += 1;
                self.changed.extend([e.a, e.b, c, d]);
            }
        }
    }

    /// Whether turning edge `e` evens out the valences around it without harm: both faces
    /// visible and in one group, no corner fully masked, no existing edge made twice, neither new
    /// face leaning from the pair's old normal (see [`leans`]) and none thinner than the old.
    fn should_flip(&self, e: &Edge) -> bool {
        let Some(g) = e.g else { return false };
        let f = e.f;
        if self.hidden(f) || self.hidden(g) || self.group(f) != self.group(g) {
            return false;
        }
        let Some((c, d)) = self.ed.wings(e) else {
            return false;
        };
        let (a, b) = (e.a, e.b);
        if c == d || self.joined(c, d) {
            return false;
        }
        if [a, b, c, d].iter().any(|&v| self.mask(v) >= 1.0) {
            return false;
        }
        let mut before = 0;
        let mut after = 0;
        for (v, gains) in [(a, false), (b, false), (c, true), (d, true)] {
            let fan = self.ed.fan(v);
            debug_assert_eq!(fan.1, self.ed.neighbours_of(v).len(), "point {v}'s valence");
            let (val, target) = match fan {
                (Around::Inside, n) => (n, 6),
                (Around::Border, n) => (n, 4),
                (Around::Tangled, _) => return false,
            };
            let then = if gains {
                val + 1
            } else {
                val.saturating_sub(1)
            };
            before += val.abs_diff(target).pow(2);
            after += then.abs_diff(target).pow(2);
        }
        if after >= before {
            return false;
        }
        let pos = |u: VertId| self.ed.pos(u);
        let (fo, go) = (self.ed.face(f), self.ed.face(g));
        let old = face_cross(fo, pos) + face_cross(go, pos);
        let new1 = [a, d, c, NO];
        let new2 = [b, c, d, NO];
        let (n1, n2) = (face_cross(new1, pos), face_cross(new2, pos));
        if leans(old, n1) || leans(old, n2) {
            return false;
        }
        let worst_old = face_quality(fo, pos).min(face_quality(go, pos));
        let worst_new = face_quality(new1, pos).min(face_quality(new2, pos));
        worst_new >= THIN.min(worst_old)
    }

    /// Whether `c` and `d` share an edge: `neighbours_of(c)` holding `d`, without the list.
    fn joined(&self, c: VertId, d: VertId) -> bool {
        (self.ed.ring_ref(c).iter()).any(|&f| {
            let fc = self.ed.face(f);
            next(fc, c) == d || prev(fc, c) == d
        })
    }
}
