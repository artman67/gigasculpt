//! The spatial index: a binary tree over node boxes and nothing else (ARCHITECTURE 3.1). Each
//! leaf is one mesh node. Built by ordering the nodes along a Morton curve and halving; refit
//! after edits; rebuilt when the node count changes.

use crate::math::{Aabb, Vec3, morton};

#[derive(Clone, Copy, Debug)]
struct Branch {
    b: Aabb,
    /// A leaf's mesh node, or an inner branch's first child (the second is `a + 1`).
    a: u32,
    leaf: bool,
}

const UNSET: Branch = Branch {
    b: Aabb::EMPTY,
    a: 0,
    leaf: true,
};

/// Bounding boxes and child links only. Children always sit after their parent.
#[derive(Clone, Debug, Default)]
pub struct NodeTree {
    t: Vec<Branch>,
}

impl NodeTree {
    /// A tree over `boxes`, one leaf per box.
    pub fn build(boxes: &[Aabb]) -> NodeTree {
        let all = boxes.iter().fold(Aabb::EMPTY, |a, b| a.union(*b));
        let mut order: Vec<(u64, u32)> = boxes
            .iter()
            .enumerate()
            .map(|(k, b)| (morton(b.lo, all), k as u32))
            .collect();
        order.sort_unstable();
        let items: Vec<u32> = order.into_iter().map(|x| x.1).collect();
        let mut t = NodeTree { t: Vec::new() };
        if !items.is_empty() {
            t.t.push(UNSET);
            t.split(0, &items);
            t.refit(boxes);
        }
        t
    }

    fn split(&mut self, at: usize, items: &[u32]) {
        if let [one] = items {
            self.t[at].a = *one;
            return;
        }
        let c = self.t.len();
        self.t.extend([UNSET, UNSET]);
        self.t[at] = Branch {
            b: Aabb::EMPTY,
            a: c as u32,
            leaf: false,
        };
        let (l, r) = items.split_at(items.len() / 2);
        self.split(c, l);
        self.split(c + 1, r);
    }

    /// Recompute every branch box from the leaf boxes, bottom up.
    pub fn refit(&mut self, boxes: &[Aabb]) {
        for i in (0..self.t.len()).rev() {
            let x = self.t[i];
            let a = x.a as usize;
            self.t[i].b = if x.leaf {
                boxes[a]
            } else {
                self.t[a].b.union(self.t[a + 1].b)
            };
        }
    }

    /// Leaves whose box touches the sphere, ascending, and how many boxes the search tested
    /// (the work counter that tests hold to a limit).
    pub fn query(&self, c: Vec3, r: f32) -> (Vec<u32>, usize) {
        let (found, tested) = self.walk(&|b| b.hits_sphere(c, r).then_some(0.0));
        let mut out: Vec<u32> = found.into_iter().map(|f| f.0).collect();
        out.sort_unstable();
        (out, tested)
    }

    /// Leaves whose box a ray from `o` along `d` enters, each with the distance (in lengths of
    /// `d`) at which it enters, in no particular order; and how many boxes the search tested.
    pub fn ray(&self, o: Vec3, d: Vec3) -> (Vec<(u32, f32)>, usize) {
        self.walk(&|b| b.ray_entry(o, d))
    }

    /// Leaves whose boxes, and whose ancestors' boxes, `keep` measures, with the measure.
    fn walk(&self, keep: &dyn Fn(Aabb) -> Option<f32>) -> (Vec<(u32, f32)>, usize) {
        let mut out = Vec::new();
        let mut tested = 0;
        if !self.t.is_empty() {
            self.visit(0, keep, &mut out, &mut tested);
        }
        (out, tested)
    }

    fn visit(
        &self,
        i: usize,
        keep: &dyn Fn(Aabb) -> Option<f32>,
        out: &mut Vec<(u32, f32)>,
        tested: &mut usize,
    ) {
        let x = self.t[i];
        *tested += 1;
        let Some(m) = keep(x.b) else {
            return;
        };
        if x.leaf {
            out.push((x.a, m));
        } else {
            self.visit(x.a as usize, keep, out, tested);
            self.visit(x.a as usize + 1, keep, out, tested);
        }
    }
}
