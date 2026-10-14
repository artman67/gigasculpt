//! What undo holds of a level stack, counted as gs-engine's `history/held.rs` counts it, each
//! allocation once by address (decision 0010): every level below the top reports its seam and
//! shared lists, and a level made again shares them with the level it replaced, so after a
//! smoothed UV write undo holds no copy of them, and after deleting the higher levels it holds
//! those of the level made the top, which keeps none.

use std::collections::BTreeMap;

use gs_core::{Allocation, Levels, NodeId};

use super::stack;
use super::uvs::{mesh, settings, uvs};
use crate::common::ok;

/// `mesh(11)` divided smooth twice, its UVs smoothed when `smooth_uvs`: levels 0 and 1 then
/// keep their own UVs.
fn divided(smooth_uvs: bool) -> Levels {
    let mut l = stack(&mesh(11), &settings());
    for _ in 0..2 {
        ok(l.divide_uvs(true, smooth_uvs));
    }
    l
}

/// Every allocation `l` reports, by address: its active store's and the rest.
fn allocations(l: &Levels) -> BTreeMap<usize, usize> {
    let mut out = BTreeMap::new();
    let mut add = |a: Allocation| {
        out.insert(a.at, a.bytes);
    };
    l.store().start_allocations(&mut add);
    l.other_allocations(&mut add);
    out
}

/// The bytes `old` holds that `new` does not: what an undo step keeping `old` holds.
fn held(old: &Levels, new: &Levels) -> usize {
    let new = allocations(new);
    (allocations(old).into_iter())
        .filter(|(at, _)| !new.contains_key(at))
        .map(|(_, bytes)| bytes)
        .sum()
}

/// The bytes of level `j`'s seam and shared lists, 4 per entry, read on its store; the level
/// that was active is made active again.
fn lists(l: &mut Levels, j: usize) -> usize {
    let back = l.active();
    ok(l.set(j));
    let s = l.store();
    let entries: usize = (0..s.node_count() as NodeId)
        .map(|n| s.node(n).seam.len() + s.node(n).shared.len())
        .sum();
    ok(l.set(back));
    4 * entries
}

/// A smoothed write makes levels 0 and 1 again with their new UVs. Their seam and shared lists
/// are the old levels' own, reported by both stacks at one address, so undo holds the old UVs
/// alone: the top's eight columns, 32 bytes a face, and the two kept sets, 32 bytes a face.
#[test]
fn after_a_smoothed_uv_write_undo_holds_the_old_uvs_and_no_copy_of_the_lists() {
    let mut l = divided(true);
    let sizes = [lists(&mut l, 0), lists(&mut l, 1)];
    ok(l.set(0));
    let moved: Vec<_> = (uvs(&l).iter())
        .map(|c| c.map(|uv| uv.map(|x| x * 0.5 + 0.25)))
        .collect();
    ok(l.set(2));
    let old = l.share();
    ok(l.write_uvs(moved));
    let (before, after) = (allocations(&old), allocations(&l));
    for (j, bytes) in sizes.into_iter().enumerate() {
        let shared = (before.iter()).any(|(at, &b)| b == bytes && after.get(at) == Some(&b));
        assert!(
            shared,
            "level {j}'s lists ({bytes} bytes) reported by both stacks"
        );
    }
    let faces: usize = l.info().iter().map(|i| i.faces).sum();
    assert_eq!(held(&old, &l), 32 * faces, "the old UVs alone");
}

/// Deleting the levels above level 1 makes it the top, which keeps no skeleton: undo holds the
/// old top's columns, its numbering, and level 1's seam and shared lists.
#[test]
fn after_deleting_the_higher_levels_undo_holds_the_new_top_s_old_lists() {
    let mut l = divided(false);
    let lists = lists(&mut l, 1);
    ok(l.set(1));
    let old = l.share();
    assert!(l.delete_higher());
    let mut top = 0;
    old.top().column_allocations(|a| top += a.bytes);
    let numbering = old.level_numbering(2).bytes();
    assert_eq!(held(&old, &l), top + numbering + lists);
}
