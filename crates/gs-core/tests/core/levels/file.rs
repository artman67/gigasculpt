//! Levels as a scene file stores them (decision 0016, section 15). Round trips: a stack rebuilt
//! from its stored form and a copy of its top store is the same stack (levels, numberings,
//! active level, pending edits), and the next switch, edit and divide give the same bits on
//! both. Refusals: a stored form a stack could not hold is an error, not a later panic.

use std::sync::Arc;

use gs_core::select::{FaceRule, Selector};
use gs_core::{
    AttrValue, Carried, ColumnSet, Levels, MeshStore, Placement, Settings, StoredLevels, Vec3,
    VisibilityEdit,
};

use super::carry::{edit, levels, set_attr};
use super::{bits, positions, stack, top_positions};
use crate::brush_kit::{arc, params, preset, run};
use crate::common::{fingerprint, ok, snap, some, sphere};
use crate::exact::rebuilt;
use crate::hidden::same_state;

/// `s` as a file gives it back: every pending column a fresh copy, so nothing is shared with
/// the stack it came from.
fn copied(mut s: StoredLevels) -> StoredLevels {
    if let Some(p) = &mut s.pending {
        for (_, col) in &mut p.positions {
            *col = Arc::new(col.to_vec());
        }
        for (_, col) in p.columns.iter_mut().flatten() {
            *col = Arc::new((**col).clone());
        }
    }
    s
}

/// The parts of a reopened stack: a copy of the top store, its numbering and the stored form.
fn parts(l: &Levels) -> (MeshStore, Placement, StoredLevels) {
    let placed = l.level_numbering(l.top_level()).placement();
    (rebuilt(l.top()), placed, copied(l.stored()))
}

/// `l` saved and opened again.
fn reopened(l: &Levels) -> Levels {
    let (top, placed, stored) = parts(l);
    ok(Levels::from_file(top, placed, stored))
}

/// The two stacks hold the same levels, numberings, active level, edits and stores.
fn same(a: &Levels, b: &Levels, what: &str) {
    assert_eq!(
        (a.count(), a.active(), a.pending()),
        (b.count(), b.active(), b.pending()),
        "{what}: levels, active level, pending"
    );
    for j in 0..a.count() {
        let same = a.level_numbering(j) == b.level_numbering(j);
        assert!(same, "{what}: the numbering of level {j}");
    }
    assert!(a.stored() == b.stored(), "{what}: the stored form");
    assert!(
        a.store().same_content(b.store()),
        "{what}: the active store"
    );
    assert_eq!(
        fingerprint(&snap(a.store())),
        fingerprint(&snap(b.store())),
        "{what}: the active store's bits, normals included"
    );
    assert_eq!(
        fingerprint(&snap(a.top())),
        fingerprint(&snap(b.top())),
        "{what}: the top"
    );
}

/// What a session does next with a stack: switch to the top and to every level, edit level
/// 1, carry it up, divide once more. What each switch carried, and the bits after each step.
fn next(l: &mut Levels) -> (Vec<Carried>, Vec<Vec<[u32; 3]>>, u64) {
    let (mut carried, mut out) = (Vec::new(), Vec::new());
    let top = l.top_level();
    carried.push(ok(l.set(top)));
    out.push(bits(&top_positions(l)));
    for j in 0..=top {
        carried.push(ok(l.set(j)));
        out.push(bits(&positions(l)));
    }
    ok(l.set(1));
    edit(l, |c, p| if c % 5 == 0 { p * 1.01 } else { p });
    carried.push(ok(l.set(top)));
    out.push(bits(&top_positions(l)));
    ok(l.divide(true));
    out.push(bits(&top_positions(l)));
    (carried, out, fingerprint(&snap(l.top())))
}

/// Saves and reopens `l`, checks the two stacks agree, then that the next steps agree.
fn round_trip(mut l: Levels, what: &str) {
    let mut r = reopened(&l);
    same(&l, &r, what);
    assert_eq!(next(&mut l), next(&mut r), "{what}: the next steps");
    same(&l, &r, &format!("{what}, after the next steps"));
}

#[test]
fn a_stack_at_its_top_reopens_as_the_same_stack() {
    let l = levels(2);
    assert!(l.stored().pending.is_none(), "nothing pending at the top");
    round_trip(l, "at the top");
}

#[test]
fn a_lower_active_level_reopens_with_its_pending_edits() {
    let mut l = levels(3);
    ok(l.set(1));
    edit(&mut l, |c, p| if c % 3 == 0 { p * 1.02 } else { p });
    set_attr(&mut l, "group", 4, AttrValue::U32(9));
    assert!(l.pending(), "the edits are not carried yet");
    let p = some(l.stored().pending);
    assert!(!p.positions.is_empty(), "edited positions are stored");
    assert!(
        p.columns.iter().any(|c| !c.is_empty()),
        "an edited column is stored"
    );
    round_trip(l, "level 1 with edits");
}

#[test]
fn a_lower_level_with_no_edits_reopens_with_nothing_pending() {
    let mut l = levels(2);
    ok(l.set(0));
    let p = some(l.stored().pending);
    assert!(p.positions.is_empty() && p.columns.iter().all(Vec::is_empty));
    round_trip(l, "the base, unedited");
}

#[test]
fn creases_and_pinned_corners_reopen() {
    let mesh = sphere(5, 0.3, 11, true);
    // Only creases on real edges: take them from the first faces.
    let creases = (mesh.faces[..3].iter().zip([1u8, 2, 255]))
        .map(|(f, s)| (f[0].min(f[1]), f[0].max(f[1]), s))
        .collect();
    let settings = Settings {
        creases,
        pin_corners: true,
        corners: Vec::new(),
    };
    let mut l = stack(&mesh, &settings);
    ok(l.divide(true));
    ok(l.divide(true));
    let stored = l.stored();
    assert_eq!(stored.creases.len(), 3, "the creases are stored");
    assert!(stored.pin_corners);
    ok(l.set(1));
    edit(&mut l, |c, p| if c % 4 == 1 { p * 0.98 } else { p });
    round_trip(l, "creased");
}

#[test]
fn a_mask_hidden_faces_and_a_dropped_column_made_low_reopen() {
    let mut l = stack(&sphere(6, 0.3, 31, false), &Settings::default());
    ok(l.divide(true));
    ok(l.divide(true));
    ok(l.set(1));
    let a = Vec3::new(-0.3, 0.1, 1.0).normalized();
    let b = Vec3::new(0.3, -0.1, 1.0).normalized();
    run(
        l.store_mut(),
        &preset("mask-pen").preset,
        params(None),
        &arc(a, b, 3, 0.3),
    );
    // Carry the mask up, come back, drop one node's mask and hide the unmasked faces: both
    // pending when the file is saved.
    let top = l.top_level();
    ok(l.set(top));
    ok(l.set(1));
    let mask = some(l.store().attr_id("mask"));
    let s = l.store_mut();
    let held = (0..s.node_count() as u32).find(|&n| s.node(n).attr(mask).is_some());
    s.write(&[some(held)], ColumnSet::attr(mask))
        .each(|n| n.clear_attr(mask));
    s.begin_step();
    let unmasked = s.select(&Selector::Unmasked {
        threshold: 0.5,
        rule: FaceRule::Mean,
    });
    s.edit_visibility(VisibilityEdit::Hide(&unmasked));
    s.end_step();
    assert!(l.store().any_hidden(), "some faces hidden");
    let p = some(l.stored().pending);
    assert!(
        p.dropped.iter().any(|d| !d.is_empty()),
        "a dropped column is stored"
    );
    let r = reopened(&l);
    same_state(r.store(), "the reopened level");
    round_trip(l, "mask, hidden faces, a dropped column");
}

#[test]
fn a_column_first_made_on_the_active_level_reopens() {
    let mut l = stack(&sphere(6, 0.3, 31, false), &Settings::default());
    ok(l.divide(true));
    ok(l.set(0));
    let s = l.store_mut();
    let a = Vec3::new(0.2, 0.1, 1.0).normalized();
    let b = Vec3::new(-0.2, 0.3, 1.0).normalized();
    run(
        s,
        &preset("mask-pen").preset,
        params(None),
        &arc(a, b, 3, 0.4),
    );
    let p = some(l.stored().pending);
    assert!(
        p.defs.len() > l.top().attrs().len(),
        "the column is new on this level"
    );
    round_trip(l, "a column new on the base");
}

#[test]
fn a_stored_form_a_stack_cannot_hold_is_refused() {
    let mut l = levels(2);
    ok(l.set(1));
    edit(&mut l, |c, p| if c == 3 { p * 1.1 } else { p });
    // Each change, and words of the error it must give.
    let refused = |why: &str, change: &dyn Fn(&mut Placement, &mut StoredLevels)| {
        let (top, mut placed, mut stored) = parts(&l);
        change(&mut placed, &mut stored);
        match Levels::from_file(top, placed, stored) {
            Ok(_) => panic!("not refused, expected: {why}"),
            Err(e) => assert!(e.to_string().contains(why), "{e}, expected: {why}"),
        }
    };
    let count = "disagree on their count";
    refused(count, &|_, s| {
        s.smooth.pop();
    });
    refused(count, &|_, s| s.active = 9);
    refused(count, &|_, s| s.active = 2);
    let crease = "a crease is not an edge";
    refused(crease, &|_, s| s.creases = vec![(0, 10_000, 1)]);
    refused(crease, &|_, s| {
        let f = s.base_faces[0];
        s.creases = [(f[0], f[1]), (f[1], f[2])]
            .map(|(a, b)| (a.min(b), a.max(b), 1))
            .to_vec();
        s.creases.sort_unstable_by(|a, b| b.cmp(a));
    });
    refused("a base face names", &|_, s| s.base_faces[0][0] = 1 << 30);
    refused("node table does not hold", &|_, s| {
        s.lower[0].nodes[0][0] += 1;
    });
    refused("is not a numbering", &|_, s| {
        s.lower[1].numbering.vertices[0] = s.lower[1].numbering.vertices[1];
    });
    refused("do not follow the subdivision rule", &|p, _| {
        p.faces.swap(0, 7);
    });
    let column = "edited position column does not fit";
    refused(column, &|_, s| {
        let p = some(s.pending.as_mut());
        p.positions[0].1 = Arc::new(vec![Vec3::ZERO]);
    });
    refused(column, &|_, s| {
        let p = some(s.pending.as_mut());
        let first = p.positions[0].clone();
        p.positions.push(first);
    });
}
