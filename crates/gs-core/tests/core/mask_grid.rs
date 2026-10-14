//! Every mask value a part stores is on the 2^-24 grid, `settle(x)` to the bit (decision 0015,
//! amended 2026-10-13): after a subdivision step over a soft mask, a Mask Pen stroke on a lower
//! level carried to the top, an edge split and a collapse under Live Detail, a reconstruct (with
//! the coarse-corner search behind it, the unsubdivide), and Project Detail.

use gs_core::live::{LiveDetail, LiveReport, Target, refine};
use gs_core::mask::settle;
use gs_core::projection::{Projection, Surface, carry, project};
use gs_core::{AttrDef, AttrVec, ColumnSet, Levels, MeshStore, Settings, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::fixtures;

use crate::brush_kit::{arc, params, preset, run};
use crate::common::{dir, mix, ok, small_nodes, some, sphere, store, surface_point, unit};
use crate::levels::stack;

/// `m` with a soft mask: a settled value from 0 to 0.8 at random on every point.
fn soft(mut m: PlainMesh) -> PlainMesh {
    let mask = (0..m.positions.len() as u64)
        .map(|i| settle((unit(mix(i ^ 5)) + 1.0) * 0.4))
        .collect();
    let values = AttrValues::F32(mask);
    let domain = AttrDomain::Vertex;
    m.attributes
        .insert("mask".into(), Attribute { domain, values });
    m
}

/// Every mask value `s` holds, dead slots too.
fn mask_values(s: &MeshStore) -> Vec<f32> {
    let a = some(s.attr_id(&AttrDef::mask().name));
    (0..s.node_count() as u32)
        .filter_map(|n| match s.node(n).attr(a) {
            Some(AttrVec::F32(v)) => Some(v.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Fails naming how many of `s`'s mask values are off the grid, and the first, or a mask with
/// nothing soft in it, which would test nothing.
fn on_grid(s: &MeshStore, what: &str) {
    let values = mask_values(s);
    let soft = values.iter().filter(|&&x| x > 0.0 && x < 1.0).count();
    assert!(soft > 0, "{what}: the mask holds soft values");
    let off: Vec<f32> = (values.iter().copied())
        .filter(|x| settle(*x).to_bits() != x.to_bits())
        .collect();
    assert!(
        off.is_empty(),
        "{what}: {} of {} mask values off the grid, the first {:?}",
        off.len(),
        values.len(),
        off.first()
    );
}

#[test]
fn a_subdivision_step_over_a_soft_mask_keeps_every_mask_value_on_the_grid() {
    for smooth in [true, false] {
        let mut l = stack(&sphere(6, 0.3, 31, true), &Settings::default());
        for step in 1..=2 {
            ok(l.divide(smooth));
            on_grid(l.top(), &format!("smooth {smooth}, step {step}"));
        }
    }
}

/// Puts every mask value of `s` on the grid, whatever made it.
fn settle_all(s: &mut MeshStore) {
    let a = some(s.attr_id(&AttrDef::mask().name));
    let nodes: Vec<u32> = (0..s.node_count() as u32).collect();
    s.write(&nodes, ColumnSet::attr(a)).each(|node| {
        if let AttrVec::F32(m) = node.attr(a) {
            for x in m.iter_mut() {
                *x = settle(*x);
            }
        }
    });
}

#[test]
fn a_mask_stroke_on_a_lower_level_carried_to_the_top_stays_on_the_grid() {
    let mut l = stack(&sphere(6, 0.3, 31, true), &Settings::default());
    ok(l.divide(true));
    ok(l.divide(true));
    // A top on the grid however the steps made it, so only the carry is tested.
    settle_all(l.store_mut());
    ok(l.set(1));
    let a = Vec3::new(-0.3, 0.1, 1.0).normalized();
    let b = Vec3::new(0.3, -0.1, 1.0).normalized();
    run(
        l.store_mut(),
        &preset("mask-pen").preset,
        params(None),
        &arc(a, b, 3, 0.3),
    );
    let carried = ok(l.set(2));
    assert!(carried.values > 0, "the stroke was carried");
    on_grid(l.top(), "the top after the carry");
}

/// Live Detail with a fixed target length and no relaxing.
fn detail(len: f32, combined: bool) -> LiveDetail {
    LiveDetail {
        target: Target::Constant(len),
        combined,
        smooth_split: false,
        relax: 0.0,
        ..LiveDetail::default()
    }
}

/// Ten Live Detail passes over spots of `s`, radius 0.4.
fn passes(s: &mut MeshStore, ld: &LiveDetail) -> LiveReport {
    let mut out = LiveReport::default();
    for i in 0..10 {
        let at = (surface_point(s, dir(90 + i)), 0.4);
        refine(s, &[at], ld, [false; 3], &mut out);
    }
    out
}

#[test]
fn an_edge_split_under_live_detail_over_a_soft_mask_stays_on_the_grid() {
    let mut s = store(&soft(sphere(6, 1.0, 9, false)), small_nodes());
    let out = passes(&mut s, &detail(0.05, false));
    assert!(out.splits > 50, "{out:?}");
    on_grid(&s, "after the splits");
}

#[test]
fn a_collapse_under_live_detail_over_a_soft_mask_stays_on_the_grid() {
    let mut s = store(&soft(sphere(16, 1.0, 9, false)), small_nodes());
    let out = passes(&mut s, &detail(0.6, true));
    assert!(out.collapses > 50, "{out:?}");
    on_grid(&s, "after the collapses");
}

#[test]
fn a_reconstruct_and_its_coarse_corner_search_keep_every_level_s_mask_on_the_grid() {
    // Two divisions of a cube as topology, each side a 4 by 4 grid.
    let l = stack(&soft(fixtures::cube(4)), &Settings::default());
    let twice = ok(ok(l.reconstruct()).reconstruct());
    assert_eq!(twice.count(), 3);
    for k in 0..3 {
        let mut at = twice.share();
        ok(at.set(k));
        on_grid(at.store(), &format!("level {k}"));
    }
}

#[test]
fn project_detail_keeps_the_target_s_mask_and_the_one_it_carries_on_the_grid() {
    let mut source = stack(&sphere(6, 0.3, 7, true), &Settings::default());
    ok(source.divide(true));
    let surface = Surface::of(source.top(), false);
    let how = Projection {
        keep: true,
        ..Projection::plain(&surface, 0.5)
    };
    // A target divided onto the surface: its own mask comes up through the steps.
    let target = stack(&sphere(6, 0.3, 31, true), &Settings::default());
    let faces = source.top().face_count() * 4;
    let (divided, _) = ok(Levels::divided_onto(target, &how, faces, 3));
    assert!(divided.count() > 1, "the target was divided");
    on_grid(divided.top(), "the divided target");
    // A mask carried across where the points land.
    let mut plain = store(&sphere(6, 0.3, 31, false), small_nodes());
    let done = project(&mut plain, &how);
    ok(carry(
        &mut plain,
        &how,
        source.top(),
        &AttrDef::mask(),
        &done.landed,
    ));
    on_grid(&plain, "the carried mask");
}
