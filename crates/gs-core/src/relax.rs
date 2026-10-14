//! Tangential relaxation, Live Detail's last step (decision 0021, section 2, step 7): points a
//! pass changed slide along the surface toward the middle of their neighbours, so triangles stay
//! well shaped without the surface moving.

use crate::config::VertId;
use crate::math::{Vec3, arity, face_cross, face_quality, leans};
use crate::normals::vertex_normal;
use crate::topo::TopoEditor;
use crate::view::{PosReader, TopoReader};

/// Relaxation leaves no face thinner than this (see [`face_quality`]) unless it already was.
pub(crate) const THIN: f32 = 0.05;

impl TopoEditor<'_> {
    /// The normal of `v` from the faces and positions as they are now.
    pub(crate) fn normal_now(&self, v: VertId) -> Vec3 {
        let cfg = self.s.cfg;
        let t = TopoReader {
            cfg,
            topo: &self.s.cols.topo,
        };
        let p = PosReader {
            cfg,
            cols: &self.s.cols.pos,
        };
        vertex_normal(&t, &p, v)
    }

    /// Move each point `v` toward the mean of its neighbours by its weight `w`, less the move's
    /// part along its normal, and with its coordinate along `held` kept when given. Every move
    /// is worked out from the positions before any is written, so the order of the points does
    /// not matter. Then, until none is left, the moves of every corner of a face the moves
    /// together would lean over (see [`leans`]), or make thinner than both [`THIN`] and itself
    /// before, are dropped.
    pub(crate) fn relax(&mut self, points: &[(VertId, f32, Option<usize>)]) {
        let mut moves: Vec<(VertId, Vec3)> = (points.iter())
            .filter(|&&(v, w, _)| w > 0.0 && self.is_live(v))
            .filter_map(|&(v, w, held)| {
                let ns = self.neighbours_of(v);
                if ns.is_empty() {
                    return None;
                }
                let sum = ns.iter().fold(Vec3::ZERO, |s, &u| s + self.pos(u));
                let p = self.pos(v);
                let d = (sum * (1.0 / ns.len() as f32) - p) * w;
                let n = self.normal_now(v);
                let mut q = (p + (d - n * n.dot(d))).to_array();
                if let Some(a) = held {
                    q[a] = p.to_array()[a];
                }
                Some((v, Vec3::from_array(q)))
            })
            .collect();
        moves.sort_unstable_by_key(|m| m.0);
        moves.dedup_by_key(|m| m.0);
        loop {
            let bad = self.harmed(&moves);
            if bad.is_empty() {
                break;
            }
            moves.retain(|m| bad.binary_search(&m.0).is_err());
        }
        for (v, p) in moves {
            self.set_pos(v, p);
        }
    }

    /// Corners (ascending) of the faces that `moves` (ascending by point) together lean over or
    /// make too thin.
    fn harmed(&self, moves: &[(VertId, Vec3)]) -> Vec<VertId> {
        let now = |u: VertId| self.pos(u);
        let after = |u: VertId| match moves.binary_search_by_key(&u, |m| m.0) {
            Ok(i) => moves[i].1,
            Err(_) => self.pos(u),
        };
        let mut bad = Vec::new();
        for &(v, _) in moves {
            for &f in self.ring_ref(v) {
                let fc = self.face(f);
                let turned = leans(face_cross(fc, now), face_cross(fc, after));
                let q = face_quality(fc, after);
                if turned || (q < THIN && q < face_quality(fc, now)) {
                    bad.extend_from_slice(&fc[..arity(fc)]);
                }
            }
        }
        bad.sort_unstable();
        bad.dedup();
        bad
    }
}
