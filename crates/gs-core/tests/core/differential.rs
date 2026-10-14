//! Differential tests against brute-force references, and work-counter limits that keep the
//! optimizations switched on (ARCHITECTURE 10.1, kind 3).

use gs_core::{ColumnSet, MeshStore, NodeConfig, Scratch, Vec3};

use crate::common::{
    Relax, brute_query, check, dir, refine, snap, sphere, store, stroke, surface_point,
};

/// Spheres all over, plus a point at both corners of every node's halo box, which a tree with
/// stale boxes would miss.
fn probe(s: &MeshStore, tag: &str) {
    let sn = snap(s);
    let mut spheres: Vec<(Vec3, f32)> = (0..60u64)
        .map(|i| {
            (
                dir(i) * [0.5, 1.0, 1.4][(i % 3) as usize],
                [0.02, 0.1, 0.3, 0.8, 2.5][(i % 5) as usize],
            )
        })
        .collect();
    spheres.extend(
        sn.nodes
            .iter()
            .flat_map(|n| [(n.halo.lo, 0.0), (n.halo.hi, 0.0)]),
    );
    for (i, &(c, r)) in spheres.iter().enumerate() {
        let q = s.query(c, r);
        let (brushed, shell) = brute_query(&sn, c, r);
        assert_eq!((q.brushed, q.shell), (brushed, shell), "{tag}, probe {i}");
    }
}

#[test]
fn tree_queries_match_brute_force_through_edits() {
    let mut s = store(&sphere(12, 1.0, 41, false), NodeConfig::new(6, 40).unwrap());
    probe(&s, "fresh");
    stroke(&mut s, 2, 10, 0.4);
    probe(&s, "after a stroke");
    // Smoothing shrinks boxes; push a few nodes outward so their boxes grow, which a tree
    // that missed the refit would not see.
    s.write(&[3, 9, 17], ColumnSet::POSITION).each(|n| {
        for p in n.positions() {
            *p = *p * 1.02;
        }
    });
    probe(&s, "after growing nodes");
    let c = surface_point(&s, Vec3::new(1.0, -0.2, 0.3));
    s.begin_step();
    refine(&mut s, c, 0.4, 0.02, 3);
    s.end_step();
    probe(&s, "after refinement");
    s.undo();
    probe(&s, "after undo");
    s.resort().unwrap();
    probe(&s, "after re-sort");
}

#[test]
fn normals_after_every_dab_equal_a_full_recompute() {
    let mut s = store(&sphere(12, 0.5, 43, false), NodeConfig::new(6, 40).unwrap());
    let mut sc = Scratch::default();
    for i in 0..12u64 {
        let c = surface_point(&s, dir(i % 4) + dir(9) * (i as f32 * 0.03));
        let r = [0.05, 0.2, 0.45][(i % 3) as usize];
        s.run_kernel(&Relax { c, r, s: 0.9 }, &mut sc);
        check(&snap(&s)).unwrap_or_else(|e| panic!("dab {i}: {e}"));
    }
}

#[test]
fn a_small_query_tests_few_boxes_and_returns_few_nodes() {
    // The input order is shuffled, so only the Morton sort can make nodes compact.
    let s = store(&sphere(48, 0.5, 42, false), NodeConfig::new(6, 40).unwrap());
    let (nodes, boxes) = (s.node_count(), 2 * s.node_count() - 1);
    assert!(nodes > 300, "{nodes} nodes");
    for i in 0..20 {
        let c = surface_point(&s, dir(i));
        let q = s.query(c, 0.03);
        let touched = q.brushed.len() + q.shell.len();
        assert!(
            touched * 25 <= nodes,
            "probe {i} touched {touched} of {nodes} nodes"
        );
        assert!(
            q.tested * 6 <= boxes,
            "probe {i} tested {} of {boxes} boxes",
            q.tested
        );
    }
    let q = s.query(Vec3::ZERO, 10.0);
    assert_eq!(
        (q.brushed.len(), q.tested),
        (nodes, boxes),
        "a query of everything tests every box"
    );
}

#[test]
fn a_kernel_step_recomputes_only_the_normals_it_can_change() {
    let mut s = store(&sphere(16, 0.5, 44, false), NodeConfig::new(6, 40).unwrap());
    let c = surface_point(&s, Vec3::new(0.0, -1.0, 0.0));
    let w0 = s.work();
    let q = s.query(c, 0.3);
    let rep = s.run_kernel(&Relax { c, r: 0.3, s: 0.5 }, &mut Scratch::default());
    let w1 = s.work();
    let slots: usize = rep.brushed.iter().map(|&n| s.node(n).pos.len()).sum();
    let seams: usize = rep.shell.iter().map(|&n| s.node(n).seam.len()).sum();
    assert_eq!(
        w1.normals - w0.normals,
        (slots + seams) as u64,
        "normals recomputed"
    );
    assert_eq!(
        w1.boxes_tested - w0.boxes_tested,
        q.tested as u64,
        "boxes tested"
    );
    assert_eq!(
        w1.tree_builds, w0.tree_builds,
        "the tree is refit, not rebuilt"
    );
    assert!(slots * 4 < s.vertex_count(), "a small part of the mesh");
}

#[test]
fn a_position_write_recomputes_its_nodes_and_the_seams_around_them() {
    let mut s = store(&sphere(16, 0.5, 45, false), NodeConfig::new(6, 40).unwrap());
    let w0 = s.work();
    s.take_dirty();
    s.write(&[7, 8], ColumnSet::POSITION).each(|n| {
        for p in n.positions() {
            p.x -= 0.002;
        }
    });
    let dirty = s.take_dirty();
    let slots: usize = [7, 8].iter().map(|&n| s.node(n).pos.len()).sum();
    let seams: usize = (dirty.iter().filter(|d| d.0 != 7 && d.0 != 8))
        .map(|d| s.node(d.0).seam.len())
        .sum();
    assert_eq!(
        s.work().normals - w0.normals,
        (slots + seams) as u64,
        "normals recomputed"
    );
    assert_eq!(s.work().tree_builds, w0.tree_builds, "refit only");
}

/// The tree is rebuilt only when nodes are added or removed (decision 0021, section 6): a
/// topology edit inside the existing nodes, and its undo, refit it.
#[test]
fn the_tree_is_rebuilt_only_when_the_node_count_changes() {
    let mut s = store(&sphere(10, 1.0, 46, false), NodeConfig::new(6, 40).unwrap());
    let builds = |s: &MeshStore| s.work().tree_builds;
    let b0 = builds(&s);
    stroke(&mut s, 1, 3, 0.3);
    s.undo();
    s.redo();
    assert_eq!(builds(&s), b0, "strokes and their undo refit");
    let c = surface_point(&s, Vec3::new(0.0, 0.0, -1.0));
    s.begin_step();
    let cuts = refine(&mut s, c, 0.1, 0.05, 1);
    s.end_step();
    assert_eq!(cuts, 0, "a small refinement fits its nodes");
    assert_eq!(builds(&s), b0, "a topology edit within the nodes refits");
    s.undo();
    assert_eq!(builds(&s), b0, "and so does its undo");
    s.begin_step();
    let cuts = refine(&mut s, c, 0.6, 0.01, 1);
    s.end_step();
    assert!(cuts > 0, "a large refinement cuts nodes");
    assert_eq!(builds(&s), b0 + 1, "one rebuild when nodes are added");
    s.undo();
    assert_eq!(builds(&s), b0 + 2, "and one when its undo removes them");
    s.resort().unwrap();
    assert_eq!(builds(&s), b0 + 3, "and per whole-mesh change");
    check(&snap(&s)).unwrap();
}
