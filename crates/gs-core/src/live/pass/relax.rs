//! Step 7 of a pass (decision 0021, section 2): relax the points the pass made or changed.

use crate::config::VertId;

use super::Pass;

impl Pass<'_, '_> {
    pub(super) fn relax_all(&mut self) {
        self.changed.sort_unstable();
        self.changed.dedup();
        let w = self.ld.relax;
        // With a map, a point relaxes when it or its partner is in reach, so both halves
        // move alike, and a point that is its own partner keeps its distance from the plane.
        let held = |v: VertId| self.mirror.map(|m| m.0).filter(|_| self.twin(v) == v);
        let points: Vec<(VertId, f32, Option<usize>)> = (self.changed.iter().copied())
            .filter(|&v| self.ed.is_live(v) && (self.reach(v) || self.reach(self.twin(v))))
            .filter(|&v| !self.feature(v))
            .map(|v| (v, w * (1.0 - self.mask(v)), held(v)))
            .collect();
        self.ed.relax(&points);
    }
}
