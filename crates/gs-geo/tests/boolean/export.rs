//! The 32-bit export rule (decision 0057, section 1, amended in step LB2): a cut whose plain
//! 32-bit result crosses itself comes back crossing nowhere, and passes the library's own scan
//! after a fresh import.

use gs_core::Vec3;
use gs_core::transform::Similarity;
use gs_geo::operation::Operation::{Difference, Union};

use crate::crossing::crossings;
use crate::shapes::{pair_b, sphere};
use crate::{op, raw, run, tris};

#[test]
fn the_export_rule_leaves_no_self_crossing() {
    // Research 26's pair at 50,700 triangles a part, and at 10,092 moved 256 units away from the
    // origin: both subtracted export with crossing pairs when rounded plainly (the probe's
    // `small` mode found them).
    let shift = Vec3::new(256.0, 128.0, 64.0);
    let mut far_b = pair_b();
    far_b.translation += shift;
    let cases = [
        ("the pair", sphere(65), Similarity::IDENTITY, pair_b()),
        (
            "the pair moved away",
            sphere(29),
            Similarity::moving(shift),
            far_b,
        ),
    ];
    for (name, ball, at_a, at_b) in cases {
        let tree = raw::Node::op(
            raw::OpType::Subtract,
            raw::leaf(&ball, &at_a),
            raw::leaf(&ball, &at_b),
        );
        let (positions, plain_tris) = raw::plain(&tree);
        let before = crossings(&positions, &plain_tris).len();
        assert!(before > 0, "{name}: the plain 32-bit export crosses itself");
        let out = run(&[op(&ball, at_a, Union), op(&ball, at_b, Difference)]);
        let pairs = crossings(&out.mesh.positions, &tris(&out.mesh));
        assert!(
            pairs.is_empty(),
            "{name}: {before} crossing pairs became {pairs:?}"
        );
        assert_eq!(
            raw::scan(&out.mesh),
            Some(false),
            "{name}: the library's scan passes"
        );
        assert!(!out.crosses, "{name}: the fold's own scan passed");
    }
}
