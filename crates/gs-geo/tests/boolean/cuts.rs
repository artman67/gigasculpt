//! What the cuts make: the volume identity, A minus A, boxes that touch or lie flush, and the
//! left fold as one expression.

use gs_core::transform::Similarity;
use gs_geo::boolean::{self, Leaf};
use gs_geo::operation::Operation::{Difference, Intersect, Union};
use gs_testkit::measure::stats;

use crate::shapes::{cuboid, small_b, sphere, turned};
use crate::{off, op, raw, run, tris, volume};

const I: Similarity = Similarity::IDENTITY;

#[test]
fn union_and_intersection_add_up_to_the_operands() {
    let a = sphere(12);
    let lone = |at| volume(&run(&[op(&a, at, Union)]).mesh);
    let (va, vb) = (lone(I), lone(small_b()));
    let with = |operation| volume(&run(&[op(&a, I, Union), op(&a, small_b(), operation)]).mesh);
    let (union, common, difference) = (with(Union), with(Intersect), with(Difference));
    assert!(
        common > 0.05 && union < va + vb - 0.05,
        "the pair overlaps: union {union}, intersection {common}"
    );
    assert!(
        off(union + common, va + vb) < 1e-6,
        "A and B {union} + {common} against A + B {va} + {vb}"
    );
    assert!(
        off(difference + common, va) < 1e-6,
        "A minus B {difference} + A and B {common} against A {va}"
    );
}

#[test]
fn a_minus_a_is_empty_and_not_an_error() {
    let a = sphere(12);
    let out = run(&[op(&a, I, Union), op(&a, I, Difference)]);
    assert!(
        out.mesh.faces.is_empty() && out.mesh.positions.is_empty(),
        "A minus A left {} faces",
        out.mesh.faces.len()
    );
    assert!(
        out.sources.points.is_empty() && out.sources.faces.is_empty(),
        "an empty result has no sources"
    );
    // One prepared operand used twice, as a cache would hand it in.
    let p = boolean::prepare(&a).unwrap_or_else(|e| panic!("{e}"));
    let leaf = |operation| Leaf {
        operand: &p,
        at: I,
        operation,
    };
    let twice = |operation| {
        boolean::fold_prepared(&[leaf(Union), leaf(operation)])
            .unwrap_or_else(|e| panic!("{e}"))
            .mesh
    };
    assert!(
        twice(Difference).faces.is_empty(),
        "A minus A, one operand used twice"
    );
    let va = volume(&run(&[op(&a, I, Union)]).mesh);
    for operation in [Union, Intersect] {
        let v = volume(&twice(operation));
        assert!(
            off(v, va) < 1e-9,
            "A {operation:?} A is A: {v} against {va}"
        );
    }
}

#[test]
fn boxes_that_touch_or_lie_flush_cut_cleanly() {
    let a = cuboid([0.0; 3], [1.0; 3]);
    // The other box, and the volumes of its union, intersection and difference with `a`.
    let cases = [
        (
            "touching on a face",
            cuboid([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]),
            [2.0, 0.0, 1.0],
        ),
        (
            "touching on an edge",
            cuboid([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]),
            [2.0, 0.0, 1.0],
        ),
        (
            "flush",
            cuboid([0.5, 0.0, 0.0], [1.5, 1.0, 1.0]),
            [1.5, 0.5, 0.5],
        ),
        (
            "flush inside",
            cuboid([0.0, 0.0, 0.0], [0.5, 1.0, 1.0]),
            [1.0, 0.5, 0.5],
        ),
    ];
    for (name, b, wants) in cases {
        for (operation, want) in [Union, Intersect, Difference].into_iter().zip(wants) {
            let out = run(&[op(&a, I, Union), op(&b, I, operation)]);
            let s = stats(&out.mesh);
            assert!(
                (s.volume - want).abs() < 1e-9,
                "{name}, {operation:?}: volume {} against {want}",
                s.volume
            );
            assert_eq!(
                s.boundary_edges, 0,
                "{name}, {operation:?}: the result is closed"
            );
            if want < 1e-9 {
                assert!(
                    out.mesh.faces.is_empty(),
                    "{name}, {operation:?}: nothing is left"
                );
            }
            let pairs = crate::crossing::crossings(&out.mesh.positions, &tris(&out.mesh));
            assert!(
                pairs.is_empty(),
                "{name}, {operation:?}: crossing pairs {pairs:?}"
            );
        }
    }
}

#[test]
fn a_left_fold_of_differences_is_one_cut() {
    let (a, cutter) = (sphere(12), sphere(8));
    let (b_at, c_at) = (
        turned(0.4, 20.0, [0.8, 0.2, 0.0]),
        turned(0.4, 50.0, [-0.2, 0.8, 0.1]),
    );
    let out = run(&[
        op(&a, I, Union),
        op(&cutter, b_at, Difference),
        op(&cutter, c_at, Difference),
    ]);
    // The library's own A minus (B union C), imported and ruled as the fold imports and rules.
    let leaves = [
        raw::leaf(&a, &I),
        raw::leaf(&cutter, &b_at),
        raw::leaf(&cutter, &c_at),
    ];
    let [la, lb, lc] = leaves;
    let own = raw::cut(&raw::Node::op(
        raw::OpType::Subtract,
        la,
        raw::Node::op(raw::OpType::Add, lb, lc),
    ));
    let mine = raw::keys(&out.mesh.positions, &tris(&out.mesh));
    assert!(
        own.len() > 1000,
        "the cut made something: {} triangles",
        own.len()
    );
    assert!(
        mine == own,
        "(A - B) - C gave {} triangles, A - (B + C) {}, not the same bits",
        mine.len(),
        own.len()
    );
}
