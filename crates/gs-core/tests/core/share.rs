//! Shared copies (decision 0010): a share holds the very columns of its store and no history,
//! strokes on it give the same bits as on the original, and its one step lands back on the
//! original as the step the same edit would have made there. Steps have ids, and the store
//! lists what its columns and steps hold, so shared allocations can be counted once.

use std::collections::BTreeSet;

use gs_core::{Allocation, MeshStore, StepId, Vec3};

use crate::brush_kit::{at, ico, params, preset, run, store};
use crate::common::{fingerprint, ok, snap};

/// The sphere after one Draw stroke, so the copy is taken from an edited store (a re-import
/// of this mesh cuts different nodes, review 0002 M6).
fn edited() -> MeshStore {
    let mut m = store(&ico(4));
    let g = Vec3::new(0.3, 0.2, 1.0).normalized();
    let draw = preset("draw").preset;
    run(
        &mut m,
        &draw,
        params(None),
        &[at(g, 0.5), at(g + Vec3::new(-0.3, 0.3, 0.0), 0.5)],
    );
    m
}

fn inflate(m: &mut MeshStore) {
    let h = Vec3::new(-0.2, 0.1, 1.0).normalized();
    let smp = [at(h, 0.4), at(h + Vec3::new(0.1, -0.1, 0.0), 0.4)];
    run(m, &preset("inflate").preset, params(None), &smp);
}

fn addresses(m: &MeshStore) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    m.column_allocations(|a| {
        seen.insert(a.at);
    });
    seen
}

#[test]
fn a_share_strokes_to_the_same_bits_as_its_original() {
    let mut original = edited();
    let mut copy = original.share();
    assert!(
        copy.same_columns(&original),
        "a share holds the same columns"
    );
    assert_eq!((copy.undo_len(), copy.redo_len()), (0, 0), "and no history");
    inflate(&mut original);
    inflate(&mut copy);
    assert_eq!(
        fingerprint(&snap(&copy)),
        fingerprint(&snap(&original)),
        "the same stroke, the same bits"
    );
    assert!(
        copy.same_content(&original),
        "the same content, stored alike"
    );
    assert!(!copy.same_columns(&original), "in columns of their own now");
}

#[test]
fn writing_a_share_copies_only_the_nodes_written() {
    let original = edited();
    let before = fingerprint(&snap(&original));
    let mut copy = original.share();
    inflate(&mut copy);
    assert_eq!(
        fingerprint(&snap(&original)),
        before,
        "the original is untouched"
    );
    let (mine, theirs) = (addresses(&original), addresses(&copy));
    let shared = mine.intersection(&theirs).count();
    assert!(
        shared > 0 && shared < mine.len(),
        "{shared} of {} allocations still shared",
        mine.len()
    );
}

#[test]
fn an_adopted_step_is_the_step_the_edit_makes_in_place() {
    let mut here = edited();
    let mut original = edited();
    let mut copy = original.share();
    inflate(&mut here);
    inflate(&mut copy);
    let undo_before = original.undo_len();
    let adopted = ok(original.adopt(copy));
    assert_eq!(original.undo_len(), undo_before + 1, "one more undo step");
    assert_eq!(fingerprint(&snap(&original)), fingerprint(&snap(&here)));
    assert_eq!(
        original.history_bytes(),
        here.history_bytes(),
        "the same bytes held"
    );
    assert_eq!(
        adopted.bytes,
        original.history_bytes() - edited().history_bytes()
    );
    original.undo();
    assert_eq!(
        fingerprint(&snap(&original)),
        fingerprint(&snap(&edited())),
        "undo goes back"
    );
    original.redo();
    assert_eq!(
        fingerprint(&snap(&original)),
        fingerprint(&snap(&here)),
        "redo comes again"
    );
}

#[test]
fn adopting_refuses_a_copy_of_a_store_that_moved_on() {
    let mut original = edited();
    let mut copy = original.share();
    inflate(&mut copy);
    inflate(&mut original);
    let moved = fingerprint(&snap(&original));
    assert!(
        original.adopt(copy).is_err(),
        "the original changed meanwhile"
    );
    assert_eq!(fingerprint(&snap(&original)), moved, "and stays as it was");
    let mut unedited = original.share();
    unedited.begin_step();
    unedited.end_step();
    assert!(original.adopt(unedited).is_err(), "a copy without a step");
}

#[test]
fn steps_are_named_by_id_and_dropped_oldest_first() {
    let mut m = edited();
    inflate(&mut m);
    let mut ids = Vec::new();
    m.step_allocations(|id, _| {
        if ids.last() != Some(&id) {
            ids.push(id);
        }
    });
    assert_eq!(ids.len(), 2, "two steps, each listed with its allocations");
    assert_ne!(ids[0], ids[1]);
    assert!(!m.drop_step(ids[1]), "only the oldest step can be dropped");
    assert!(
        !m.drop_step(StepId::default()),
        "nor a step it does not hold"
    );
    assert!(m.drop_step(ids[0]));
    assert_eq!(m.undo_len(), 1);
    m.undo();
    assert_eq!(m.redo_len(), 1);
    m.clear_redo();
    assert_eq!((m.undo_len(), m.redo_len(), m.history_bytes()), (0, 0, 0));
}

#[test]
fn the_allocations_listed_add_up_to_the_bytes_counted() {
    let mut m = edited();
    inflate(&mut m);
    let (mut columns, mut steps) = (0, 0);
    m.column_allocations(|a: Allocation| columns += a.bytes);
    m.step_allocations(|_, a| steps += a.bytes);
    assert_eq!(columns, m.mesh_bytes(), "the columns");
    assert_eq!(steps, m.history_bytes(), "the steps");
}
