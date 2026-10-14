//! Undo history: a model test against recorded snapshots (ARCHITECTURE 10.1), cancel, redo
//! clearing, byte accounting and trimming, and the stroke-start snapshot.

use gs_core::{ColumnSet, MeshStore, NodeConfig, PosRead, Scratch, Vec3};

use crate::common::{
    Relax, check, check_closed, fingerprint, ok, refine, snap, some, sphere, store, stroke,
    surface_point,
};

type Op = fn(&mut MeshStore);

fn op_stroke(s: &mut MeshStore) {
    stroke(s, 3, 6, 0.4);
}

fn op_paint(s: &mut MeshStore) {
    let (mask, color) = (some(s.attr_id("mask")), some(s.attr_id("color")));
    let group = some(s.attr_id("group"));
    let nodes: Vec<u32> = (0..s.node_count() as u32).step_by(3).collect();
    s.begin_step();
    let cols = ColumnSet::attr(mask) | ColumnSet::attr(color) | ColumnSet::attr(group);
    s.write(&nodes, cols).each(|n| {
        n.clear_attr(mask);
        if let gs_core::AttrVec::U32(v) = n.attr(color) {
            for x in v {
                *x = x.rotate_left(8);
            }
        }
        if let gs_core::AttrVec::U32(v) = n.attr(group) {
            for x in v {
                *x += 1;
            }
        }
    });
    s.end_step();
}

fn op_move(s: &mut MeshStore) {
    s.begin_step();
    s.write(&[1, 4, 5], ColumnSet::POSITION).each(|n| {
        for p in n.positions() {
            *p = *p * 1.01;
        }
    });
    s.end_step();
}

fn op_refine(s: &mut MeshStore) {
    let c = surface_point(s, Vec3::new(0.2, 1.0, -0.4));
    s.begin_step();
    assert!(
        refine(s, c, 0.35, 0.02, 3) > 0,
        "the refinement splits nodes"
    );
    s.end_step();
}

fn op_resort(s: &mut MeshStore) {
    s.begin_step();
    ok(s.resort());
    s.end_step();
}

const OPS: [Op; 7] = [
    op_stroke, op_paint, op_move, op_refine, op_stroke, op_resort, op_stroke,
];

#[test]
fn undo_and_redo_restore_every_step_bit_for_bit() {
    let mut s = store(&sphere(10, 1.0, 5, true), NodeConfig::new(6, 40).unwrap());
    let mut prints = vec![fingerprint(&snap(&s))];
    for op in OPS {
        op(&mut s);
        let sn = snap(&s);
        check(&sn).unwrap();
        check_closed(&s).unwrap();
        prints.push(fingerprint(&sn));
    }
    assert_eq!(s.undo_len(), OPS.len(), "one step per operation");
    for i in (0..OPS.len()).rev() {
        assert!(s.undo().is_some(), "undo {i}");
        assert_eq!(fingerprint(&snap(&s)), prints[i], "after undoing step {i}");
    }
    assert!(s.undo().is_none(), "nothing left to undo");
    for (i, want) in prints.iter().enumerate().skip(1) {
        assert!(s.redo().is_some(), "redo {i}");
        assert_eq!(fingerprint(&snap(&s)), *want, "after redoing step {i}");
    }
    assert!(s.redo().is_none(), "nothing left to redo");
}

#[test]
fn cancel_restores_the_mesh_exactly() {
    let mut s = store(&sphere(10, 1.0, 6, true), NodeConfig::new(6, 40).unwrap());
    let before = fingerprint(&snap(&s));
    let nodes = s.node_count();
    s.begin_step();
    s.write(
        &[0, 2],
        ColumnSet::POSITION | ColumnSet::attr(s.attr_id("mask").unwrap()),
    )
    .each(|n| n.positions()[0].x += 0.1);
    s.cancel_step();
    assert_eq!(fingerprint(&snap(&s)), before, "a cancelled write");
    s.begin_step();
    let c = surface_point(&s, Vec3::new(0.0, 0.0, 1.0));
    refine(&mut s, c, 0.4, 0.02, 3);
    assert!(s.node_count() > nodes, "the step added nodes");
    s.cancel_step();
    assert_eq!(fingerprint(&snap(&s)), before, "cancelled");
    assert_eq!((s.undo_len(), s.redo_len()), (0, 0), "nothing kept");
}

#[test]
fn a_new_step_drops_redo_and_an_empty_step_keeps_it() {
    let mut s = store(&sphere(8, 0.5, 7, false), NodeConfig::new(6, 40).unwrap());
    op_stroke(&mut s);
    op_stroke(&mut s);
    s.undo();
    assert_eq!(s.redo_len(), 1, "one step to redo");
    s.begin_step();
    let summary = s.end_step();
    assert_eq!((summary.nodes, summary.bytes), (0, 0), "an empty step");
    assert_eq!(
        (s.undo_len(), s.redo_len()),
        (1, 1),
        "an empty step is not kept"
    );
    op_move(&mut s);
    assert_eq!(
        (s.undo_len(), s.redo_len()),
        (2, 0),
        "a real step drops redo"
    );
}

#[test]
fn history_holds_exactly_the_columns_a_step_wrote() {
    let mut s = store(&sphere(10, 0.5, 8, true), NodeConfig::new(6, 40).unwrap());
    assert_eq!(s.history_bytes(), 0, "no history yet");
    let c = surface_point(&s, Vec3::new(1.0, 0.1, 0.1));
    let before = snap(&s);
    s.begin_step();
    let rep = s.run_kernel(&Relax { c, r: 0.3, s: 0.5 }, &mut Scratch::default());
    let summary = s.end_step();
    let slots = |ns: &[u32]| ns.iter().map(|&n| s.node(n).pos.len()).sum::<usize>();
    // A shell node whose seam normals all came out the same is not held.
    let bits = |v: &[Vec3]| v.iter().map(|x| x.bits()).collect::<Vec<_>>();
    let turned: Vec<u32> = (rep.shell.iter().copied())
        .filter(|&n| bits(&before.nodes[n as usize].nrm) != bits(s.node(n).nrm))
        .collect();
    assert!(!turned.is_empty(), "the shell's normals turned");
    let want = 24 * slots(&rep.brushed) + 12 * slots(&turned);
    assert_eq!(
        summary.bytes, want,
        "positions and normals of brushed nodes, normals of the shell nodes whose normals turned"
    );
    assert_eq!(
        summary.nodes,
        rep.brushed.len() + turned.len(),
        "nodes held"
    );
    assert_eq!(s.history_bytes(), want, "history total");
    s.undo();
    assert_eq!(s.history_bytes(), want, "the same bytes wait for redo");
    let mask = s.attr_id("mask").unwrap();
    s.begin_step();
    s.write(&[1, 2], ColumnSet::attr(mask))
        .each(|n| n.clear_attr(mask));
    let summary = s.end_step();
    let slots = s.node(1).pos.len() + s.node(2).pos.len();
    assert_eq!(
        (summary.nodes, summary.bytes),
        (2, 4 * slots),
        "an attribute step holds the old values"
    );
}

#[test]
fn trimming_drops_the_oldest_steps_first() {
    let mut s = store(&sphere(8, 0.5, 9, false), NodeConfig::new(6, 40).unwrap());
    op_stroke(&mut s);
    let first = s.history_bytes();
    op_move(&mut s);
    op_stroke(&mut s);
    s.undo();
    let total = s.history_bytes();
    s.trim_history(total);
    assert_eq!(
        (s.undo_len(), s.redo_len()),
        (2, 1),
        "within budget, nothing dropped"
    );
    s.trim_history(total - first);
    assert_eq!(
        (s.undo_len(), s.redo_len()),
        (1, 1),
        "the oldest undo step went"
    );
    s.trim_history(1);
    assert_eq!((s.undo_len(), s.redo_len()), (0, 0), "then undo, then redo");
    assert_eq!(s.history_bytes(), 0, "nothing held");
}

#[test]
fn the_stroke_start_snapshot_is_the_mesh_when_the_step_opened() {
    let mut s = store(&sphere(8, 0.5, 10, false), NodeConfig::new(6, 40).unwrap());
    let before = snap(&s);
    let c = surface_point(&s, Vec3::new(0.0, 1.0, 0.0));
    s.begin_step();
    assert!(s.step_open(), "open");
    let mut sc = Scratch::default();
    for _ in 0..3 {
        s.run_kernel(&Relax { c, r: 0.5, s: 0.9 }, &mut sc);
    }
    let (start, now) = (s.start_pos(), s.pos());
    let mut moved = 0;
    for n in &before.nodes {
        for (l, p) in n.pos.iter().enumerate() {
            let v = n.base + l as u32;
            assert_eq!(start.get(v).bits(), p.bits(), "start of {v}");
            moved += usize::from(now.get(v).bits() != p.bits());
        }
    }
    assert!(moved > 10, "the dabs moved {moved} vertices");
    s.end_step();
    assert!(!s.step_open(), "closed");
    let v = before.nodes[0].base;
    assert_eq!(
        s.start_pos().get(v).bits(),
        s.pos().get(v).bits(),
        "no step: current"
    );
}

#[test]
fn undo_marks_what_it_changed_dirty() {
    let mut s = store(&sphere(8, 0.5, 11, false), NodeConfig::new(6, 40).unwrap());
    op_move(&mut s);
    s.take_dirty();
    let changed = s.undo().unwrap();
    let dirty = s.take_dirty();
    let nodes: Vec<u32> = dirty.iter().map(|d| d.0).collect();
    assert_eq!(nodes, changed, "dirty nodes");
    for (n, cols) in dirty {
        assert!(cols.contains(ColumnSet::NORMAL), "node {n} normals");
        let moved = [1, 4, 5].contains(&n);
        assert_eq!(
            cols.contains(ColumnSet::POSITION),
            moved,
            "node {n} positions"
        );
    }
}
