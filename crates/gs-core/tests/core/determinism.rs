//! Determinism (ARCHITECTURE 8): the same script gives bit-identical results on 1, 8 and 24
//! threads, step by step.

use gs_core::mask::MaskFilter;
use gs_core::{ColumnSet, MeshStore, NodeConfig, Vec3};

use crate::brush_kit::{arc, at, mirror_x, params, preset, run};

use crate::common::{
    check, fingerprint, ok, refine, snap, some, sphere, store, stroke, surface_point,
};

/// Import, strokes, attribute and position writes, local refinement with node splits and
/// re-sort. The fingerprint of every node's data after each stage. (Detail levels have their
/// own thread-count test in `levels`.)
fn script() -> Vec<u64> {
    let m = sphere(12, 0.6, 9, true);
    let mut s = store(&m, ok(NodeConfig::new(6, 40)));
    let mut prints = vec![fingerprint(&snap(&s))];
    let mut note = |s: &MeshStore| prints.push(fingerprint(&snap(s)));
    stroke(&mut s, 1, 8, 0.4);
    note(&s);
    paint(&mut s);
    note(&s);
    hide_and_filter(&mut s);
    note(&s);
    let c = surface_point(&s, Vec3::new(0.3, -1.0, 0.2));
    refine(&mut s, c, 0.4, 0.03, 3);
    note(&s);
    stroke(&mut s, 5, 4, 0.3);
    ok(s.resort());
    note(&s);
    ok(check(&snap(&s)));
    prints
}

/// Write the mask and nudge positions on the nodes around one point.
fn paint(s: &mut MeshStore) {
    let mask = some(s.attr_id("mask"));
    let c = surface_point(s, Vec3::new(-1.0, 0.2, 0.5));
    let nodes = s.query(c, 0.5).brushed;
    let mut w = s.write(&nodes, ColumnSet::POSITION | ColumnSet::attr(mask));
    w.each(|n| {
        let shift = n.id() as f32 * 1e-3;
        for p in n.positions().iter_mut() {
            p.y += shift;
        }
        if let gs_core::AttrVec::F32(v) = n.attr(mask) {
            for x in v {
                *x = 1.0 - *x;
            }
        }
    });
}

/// Hide with a mirrored Select Rect, run the mask filters, and smooth across the hidden border.
fn hide_and_filter(s: &mut MeshStore) {
    let corners = [(-0.1, -0.5), (0.6, 0.4)];
    let rect: Vec<_> = (corners.iter())
        .map(|&(x, y)| at(Vec3::new(x, y, 3.0), 0.1))
        .collect();
    run(
        s,
        &preset("select-rect").preset,
        params(Some(mirror_x())),
        &rect,
    );
    for filter in [MaskFilter::Sharpen, MaskFilter::Grow, MaskFilter::Blur] {
        s.mask_filter(filter, 1.0);
    }
    let (a, b) = (Vec3::new(0.5, 0.0, 0.9), Vec3::new(0.7, 0.3, 0.6));
    let path = arc(a.normalized(), b.normalized(), 5, 0.3);
    run(s, &preset("smooth").preset, params(None), &path);
}

#[test]
fn every_stage_is_bit_identical_on_1_8_and_24_threads() {
    let runs: Vec<Vec<u64>> = [1, 8, 24].map(|t| gs_par::with_threads(t, script)).to_vec();
    assert_eq!(runs[0], runs[1], "1 and 8 threads");
    assert_eq!(runs[0], runs[2], "1 and 24 threads");
    assert_eq!(runs[0], script(), "the global pool");
}
