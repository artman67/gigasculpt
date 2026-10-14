//! Node upkeep at the end of a refining stroke (decision 0021, section 7, L2): after 2,000 dabs
//! circling one spot the nodes stay about as local as a fresh sort's; a node a coarsening stroke
//! leaves small merges into a neighbour, the next node cut reuses it, and undo is exact.

use gs_core::live::{LiveDetail, Target};
use gs_core::{MeshStore, NO, NodeConfig, StrokeParams, StrokeSample, Vec3};

use crate::brush_kit::{at, params, run_preset};
use crate::common::{
    check, check_closed, fingerprint, ok, snap, some, sphere, store, surface_point,
};

fn constant(len: f32) -> StrokeParams {
    let mut p = params(None);
    p.live_detail = Some(LiveDetail {
        target: Target::Constant(len),
        ..LiveDetail::default()
    });
    p
}

/// Samples once round a circle of angular radius `rho` about the front (+z) of the sphere,
/// starting at angle `turn`.
fn circle(s: &MeshStore, rho: f32, turn: f32, r: f32) -> Vec<StrokeSample> {
    (0..=48)
        .map(|k| {
            let t = turn + std::f32::consts::TAU * k as f32 / 48.0;
            let d = Vec3::new(rho * libm::cosf(t), rho * libm::sinf(t), 1.0);
            at(surface_point(s, d), r)
        })
        .collect()
}

/// Edges between two nodes, and the mean index gap along edges inside one.
fn locality(s: &MeshStore) -> (usize, f64) {
    let cfg = s.config();
    let mut edges = Vec::new();
    for n in 0..s.node_count() as u32 {
        for f in s.node(n).faces.iter().filter(|f| f[0] != NO) {
            let k = if f[3] == NO { 3 } else { 4 };
            for i in 0..k {
                let (a, b) = (f[i], f[(i + 1) % k]);
                edges.push((a.min(b), a.max(b)));
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let (mut cross, mut inside, mut gaps) = (0, 0, 0u64);
    for (a, b) in edges {
        if cfg.node_of(a) == cfg.node_of(b) {
            inside += 1;
            gaps += u64::from(cfg.local_of(a).abs_diff(cfg.local_of(b)));
        } else {
            cross += 1;
        }
    }
    (cross, gaps as f64 / f64::from(inside.max(1)))
}

/// Nodes holding points, and the fewest points any of them holds.
fn sizes(s: &MeshStore) -> (usize, usize) {
    let lens: Vec<usize> = (0..s.node_count() as u32)
        .map(|n| s.node(n).pos.len())
        .filter(|&l| l > 0)
        .collect();
    (lens.len(), lens.iter().copied().min().unwrap_or(0))
}

#[test]
fn two_thousand_dabs_circling_one_spot_keep_nodes_as_local_as_a_fresh_sort() {
    let mut s = store(&sphere(16, 1.0, 31, true), ok(NodeConfig::new(8, 128)));
    let (mut dabs, mut strokes) = (0, 0);
    while dabs < 2000 {
        // Fine and coarse in turn, so the strokes both add and remove points.
        let len = [0.03, 0.06, 0.04][strokes % 3];
        let samples = circle(&s, 0.35, strokes as f32 * 0.7, 0.15);
        let out = run_preset(&mut s, "clay", constant(len), &samples);
        assert!(some(out.live).ops() > 0, "stroke {strokes} refined");
        dabs += out.dabs;
        strokes += 1;
    }
    if let Err(e) = check(&snap(&s)).and(check_closed(&s)) {
        panic!("after {strokes} strokes: {e}");
    }
    let mut fresh = s.share();
    ok(fresh.resort());
    let ((cross, gap), (cross0, gap0)) = (locality(&s), locality(&fresh));
    let report = format!("{strokes} strokes: cross {cross} vs {cross0}, gap {gap:.1} vs {gap0:.1}");
    assert!(cross as f64 <= 1.2 * cross0 as f64, "{report}");
    assert!(gap <= 2.0 * gap0, "{report}");
}

#[test]
fn a_node_left_small_merges_and_the_next_cut_reuses_it() {
    let cfg = ok(NodeConfig::new(8, 128));
    let mut s = store(&sphere(16, 1.0, 32, true), cfg);
    let fine = circle(&s, 0.2, 0.0, 0.2);
    run_preset(&mut s, "clay", constant(0.03), &fine);
    let before = (fingerprint(&snap(&s)), s.node_count(), sizes(&s).0);
    // Coarse enough to collapse most of what the fine stroke added.
    let coarse = circle(&s, 0.2, 0.3, 0.25);
    let out = run_preset(&mut s, "smooth", constant(0.2), &coarse);
    assert!(some(out.live).collapses > 0, "{:?}", out.live);
    if let Err(e) = check(&snap(&s)).and(check_closed(&s)) {
        panic!("after the coarse stroke: {e}");
    }
    let (held, least) = sizes(&s);
    assert_eq!(s.node_count(), before.1, "merging keeps the node count");
    assert!(
        held < before.2,
        "a node merged away: {held} of {}",
        before.2
    );
    assert!(least > 0, "{least}");
    // Undo brings the merged node back exactly.
    let after = fingerprint(&snap(&s));
    some(s.undo());
    assert_eq!(fingerprint(&snap(&s)), before.0, "undo");
    some(s.redo());
    assert_eq!(fingerprint(&snap(&s)), after, "redo");
    // Refining again cuts nodes into the empty ones before adding any.
    let empty = s.node_count() - held;
    run_preset(&mut s, "clay", constant(0.02), &fine);
    let (now, _) = sizes(&s);
    assert!(now > held, "the fine stroke cut nodes");
    assert!(
        s.node_count() == before.1 || now - held > empty,
        "a new node only once the empty ones are used: {now} held, {} nodes",
        s.node_count()
    );
    ok(check(&snap(&s)));
}
