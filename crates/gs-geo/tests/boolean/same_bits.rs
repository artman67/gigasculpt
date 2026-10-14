//! The same bits at any thread count (decision 0057, section 4): each operation on research 26's
//! pair at 100k triangles, and each repair step, at 1 and 24 threads.

use gs_core::transform::Similarity;
use gs_geo::boolean::{Booleaned, Repaired, Step};
use gs_geo::operation::Operation::{self, Difference, Intersect, Union};
use gs_protocol::PlainMesh;

use crate::shapes::{fold, inverted, pair_b, shells, sphere};
use crate::{op, run};

/// Points, faces, point sources, face sources and repairs.
type Bits = (
    Vec<[u32; 3]>,
    Vec<[u32; 4]>,
    Vec<Option<([u32; 3], [u32; 3])>>,
    Vec<Option<u32>>,
    Vec<Repaired>,
);

/// Everything a cut hands back, as bits.
fn bits(out: &Booleaned) -> Bits {
    let positions = out
        .mesh
        .positions
        .iter()
        .map(|p| p.map(f32::to_bits))
        .collect();
    let points = (out.sources.points.iter())
        .map(|s| s.map(|(tri, w)| (tri, w.map(f32::to_bits))))
        .collect();
    let faces = out.sources.faces.clone();
    (
        positions,
        out.mesh.faces.clone(),
        points,
        faces,
        out.repaired.clone(),
    )
}

/// The group cut at 1 and at 24 threads gives the same bits.
fn same(name: &str, group: &[(&PlainMesh, Similarity, Operation)]) {
    let cut = || {
        let ops: Vec<_> = group.iter().map(|&(m, at, o)| op(m, at, o)).collect();
        bits(&run(&ops))
    };
    let one = gs_par::with_threads(1, cut);
    let many = gs_par::with_threads(24, cut);
    assert!(one.1.len() > 1000, "{name}: the cut made something");
    assert!(one == many, "{name}: not the same bits at 1 and 24 threads");
}

#[test]
fn every_operation_gives_the_same_bits_at_any_thread_count() {
    let ball = sphere(91);
    for operation in [Union, Difference, Intersect] {
        let group = [
            (&ball, Similarity::IDENTITY, Union),
            (&ball, pair_b(), operation),
        ];
        same(&format!("{operation:?}"), &group);
    }
}

#[test]
fn every_repair_gives_the_same_bits_at_any_thread_count() {
    let ball = sphere(41);
    let cases = [
        ("turned", inverted(&ball), Step::TurnedOut),
        ("two shells", shells(&ball), Step::Rebuilt),
        ("folded", fold(&ball), Step::Rebuilt),
    ];
    for (name, mesh, step) in cases {
        let alone = run(&[op(&mesh, Similarity::IDENTITY, Union)]);
        assert_eq!(
            alone.repaired,
            [Repaired { operand: 0, step }],
            "{name}: the step taken"
        );
        same(name, &[(&mesh, Similarity::IDENTITY, Union)]);
    }
}
