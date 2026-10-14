//! Wrap Mode (decision 0054): every frame a stroke lays is copied by whole tiles of its part's
//! plane, so the dab pattern repeats tile for tile and a stroke leaving one border re-enters at
//! the other.

use crate::config::NodeId;
use crate::kernels::Frame;
use crate::math::{Aabb, Vec3};
use crate::store::MeshStore;

/// The tiles a wrapping stroke lays its copies over: the part's box when the stroke began, its
/// two longer sides each cut into Wrap Mode equal tiles.
#[derive(Clone, Copy, Debug)]
pub(super) struct Wrap {
    /// The two axes the tiles run along; the thinnest side runs across the plane.
    axes: [usize; 2],
    lo: [f32; 2],
    hi: [f32; 2],
    tile: [f32; 2],
}

impl Wrap {
    /// The tiles of `store`'s part at Wrap Mode `n`: none below 2, nor for a part with no
    /// width along either of its plane's axes.
    pub(super) fn of(store: &MeshStore, n: u32) -> Option<Wrap> {
        if n < 2 {
            return None;
        }
        let nodes = 0..store.node_count() as NodeId;
        let b = nodes.fold(Aabb::EMPTY, |b, k| b.union(store.bounds(k)));
        let (lo, hi) = (b.lo.to_array(), b.hi.to_array());
        let size: [f32; 3] = std::array::from_fn(|k| hi[k] - lo[k]);
        // The first of equal sides on a tie: x before y before z.
        let thin = (0..3).min_by(|&a, &b| size[a].total_cmp(&size[b]))?;
        let axes = match thin {
            0 => [1, 2],
            1 => [0, 2],
            _ => [0, 1],
        };
        let tile = axes.map(|k| size[k] / n as f32);
        (tile.iter().all(|t| t.is_finite() && *t > 0.0)).then_some(Wrap {
            axes,
            lo: axes.map(|k| lo[k]),
            hi: axes.map(|k| hi[k]),
            tile,
        })
    }

    /// `frames` with their copies (decision 0054, section 3): for each frame, its copies moved
    /// by whole tiles whose sphere of radius `reach` meets the box, and the frame itself, in the
    /// order of their tiles along the first axis, then the second.
    pub(super) fn copies(&self, frames: &[Frame], reach: impl Fn(&Frame) -> f32) -> Vec<Frame> {
        let mut out = Vec::new();
        for f in frames {
            let (c, r) = (f.center.to_array(), reach(f));
            // The tiles along axis `k` whose copy reaches the box, first and last.
            let span = |k: usize| {
                let (t, x) = (self.tile[k], c[self.axes[k]]);
                let first = ((self.lo[k] - r - x) / t).ceil() as i32;
                let last = ((self.hi[k] + r - x) / t).floor() as i32;
                (first, last)
            };
            let (a, b) = (span(0), span(1));
            let mut tiles: Vec<(i32, i32)> = (a.0..=a.1)
                .flat_map(|i| (b.0..=b.1).map(move |j| (i, j)))
                .collect();
            if !(a.0 <= 0 && 0 <= a.1 && b.0 <= 0 && 0 <= b.1) {
                tiles.push((0, 0));
                tiles.sort_unstable();
            }
            out.extend(tiles.into_iter().map(|(i, j)| self.moved(f, i, j)));
        }
        out
    }

    /// `f` moved `i` tiles along the first axis and `j` along the second: its center and area
    /// center, since a move along the plane turns no direction.
    fn moved(&self, f: &Frame, i: i32, j: i32) -> Frame {
        if (i, j) == (0, 0) {
            return *f;
        }
        let mut d = [0.0; 3];
        d[self.axes[0]] = i as f32 * self.tile[0];
        d[self.axes[1]] = j as f32 * self.tile[1];
        let d = Vec3::from_array(d);
        Frame {
            center: f.center + d,
            origin: f.origin + d,
            ..*f
        }
    }
}
