//! Switching levels with nothing to carry: a lower level is the top's first points on its own
//! node skeleton, a round trip changes no bit, and no switch sorts. The top holds normals only
//! while it is active (decision 0014, section 13).

use gs_core::{Levels, Settings, subdivide_faces};

use super::carry::edit;
use super::{bits, faces, positions, stack, top_positions};
use crate::common::{check, fingerprint, ok, snap, sphere};

/// A sphere with three levels above its base.
fn three_up() -> Levels {
    let mut l = stack(&sphere(6, 0.3, 21, true), &Settings::default());
    for _ in 0..3 {
        ok(l.divide(true));
    }
    l
}

#[test]
fn a_lower_level_is_the_tops_first_points() {
    let mut l = three_up();
    let top = top_positions(&l);
    let base = faces(&{
        let mut b = l.share();
        ok(b.set(0));
        b
    });
    let mut want_faces = base.clone();
    let mut count = l.level_numbering(0).vertex_count() as u32;
    for j in 0..l.count() {
        ok(l.set(j));
        assert_eq!(l.active(), j);
        let p = positions(&l);
        assert_eq!(
            bits(&p),
            bits(&top[..p.len()]),
            "level {j} is the top's prefix"
        );
        assert_eq!(
            faces(&l),
            want_faces,
            "level {j} faces follow decision 0006's rule"
        );
        assert_eq!(
            check(&snap(l.store())),
            Ok(()),
            "level {j} store invariants"
        );
        (want_faces, count) = subdivide_faces(&want_faces, count).unwrap_or_default();
    }
}

#[test]
fn switching_down_and_up_with_no_edit_changes_no_bit() {
    let mut l = three_up();
    let before = l.share();
    let print = fingerprint(&snap(l.top()));
    for j in [0, 2, 1, 3, 0, 3] {
        ok(l.set(j));
    }
    let top = (l.top(), before.top());
    assert!(
        (0..top.0.node_count() as u32).all(|n| std::ptr::eq(top.0.node(n).pos, top.1.node(n).pos)),
        "the top keeps its very positions"
    );
    assert_eq!(
        fingerprint(&snap(l.top())),
        print,
        "and its normals computed again are the same bits"
    );
    assert!(!l.pending());
}

#[test]
fn a_switch_never_sorts_or_rebuilds_rings() {
    let mut l = three_up();
    let top_sorts = l.top().work().sorts;
    ok(l.set(1));
    assert_eq!(
        l.store().work().sorts,
        0,
        "gathered onto the cached skeleton"
    );
    ok(l.set(3));
    assert_eq!(l.store().work().sorts, top_sorts, "the top is the top");
}

#[test]
fn the_top_holds_normals_only_while_it_is_active() {
    let mut l = three_up();
    let top = |l: &Levels| -> Vec<usize> {
        (0..l.top().node_count() as u32)
            .map(|n| l.top().node(n).nrm.len())
            .collect()
    };
    ok(l.set(2));
    assert!(
        top(&l).iter().all(|&n| n == 0),
        "a lower level's switch drops them"
    );
    // A carry from level 2 down to level 1 writes the top without its normals.
    edit(&mut l, |c, p| if c % 7 == 0 { p * 1.05 } else { p });
    let carried = ok(l.set(1));
    assert!(carried.points > 0, "the edit reached the top");
    assert!(top(&l).iter().all(|&n| n == 0), "still none");
    ok(l.set(3));
    assert_eq!(
        check(&snap(l.top())),
        Ok(()),
        "the top's switch computes every normal again, bit for bit"
    );
}
