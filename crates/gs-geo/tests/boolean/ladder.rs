//! The repair ladder (decision 0057, section 2), each step on an operand made for it: clean
//! operands used as they are (steps 1 and 2), inside out turned (3), self-crossing rebuilt (4),
//! and open or non-manifold ones handed back by name for the voxel path (5, LB3's).

use gs_core::Vec3;
use gs_core::transform::Similarity;
use gs_geo::boolean::{self, BooleanError, Repaired, Step, Unfit};
use gs_geo::operation::Operation::{Difference, Union};
use gs_protocol::{NO_VERTEX, PlainMesh};
use gs_testkit::measure::stats;

use crate::shapes::{cuboid, fin, fold, inverted, open, shells, small_b, sphere};
use crate::{off, op, run, tris, volume};

const I: Similarity = Similarity::IDENTITY;

/// `outer` with `inner` at half size inside it, turned inward: a hollow ball, whose inner shell
/// has a negative volume and is right as it is.
fn hollow(outer: &PlainMesh, inner: &PlainMesh) -> PlainMesh {
    let inward = inverted(inner);
    let mut out = outer.clone();
    let base = out.positions.len() as u32;
    let half = inward.positions.iter().map(|p| p.map(|c| c * 0.5));
    out.positions.extend(half);
    let moved = |v: u32| if v == NO_VERTEX { v } else { v + base };
    out.faces.extend(inward.faces.iter().map(|f| f.map(moved)));
    out
}

fn steps(mesh: &PlainMesh) -> Vec<Step> {
    let p = boolean::prepare(mesh).unwrap_or_else(|e| panic!("{e}"));
    p.steps().to_vec()
}

/// The result is a clean operand for the next cut: closed, and the ladder finds nothing to mend.
fn assert_clean(mesh: &PlainMesh, what: &str) {
    assert_eq!(
        stats(mesh).boundary_edges,
        0,
        "{what}: the result is closed"
    );
    assert!(steps(mesh).is_empty(), "{what}: the result needs no repair");
    let pairs = crate::crossing::crossings(&mesh.positions, &tris(mesh));
    assert!(pairs.is_empty(), "{what}: crossing pairs {pairs:?}");
}

#[test]
fn a_clean_operand_is_used_as_it_is() {
    let ball = sphere(10);
    let cases = [
        ("a ball", ball.clone()),
        ("a box", cuboid([0.0; 3], [1.0; 3])),
        ("a hollow ball", hollow(&ball, &sphere(6))),
    ];
    for (name, mesh) in cases {
        assert!(steps(&mesh).is_empty(), "{name} takes no repair");
        let out = run(&[op(&mesh, I, Union)]);
        assert!(
            out.repaired.is_empty(),
            "{name}: nothing listed as repaired"
        );
        assert!(
            off(volume(&out.mesh), volume(&mesh)) < 1e-9,
            "{name} comes through with its volume"
        );
    }
}

#[test]
fn an_inside_out_operand_is_turned_right_way_out() {
    let ball = sphere(10);
    let inside_out = inverted(&ball);
    assert!(volume(&inside_out) < 0.0, "the operand is inside out");
    assert_eq!(steps(&inside_out), [Step::TurnedOut], "step 3");
    let alone = run(&[op(&inside_out, I, Union)]);
    assert!(
        off(volume(&alone.mesh), volume(&ball)) < 1e-9,
        "turned, it holds the ball's volume"
    );
    assert_clean(&alone.mesh, "turned");
    let cut = run(&[op(&ball, I, Union), op(&inside_out, small_b(), Difference)]);
    let right = run(&[op(&ball, I, Union), op(&ball, small_b(), Difference)]);
    assert_eq!(
        cut.repaired,
        [Repaired {
            operand: 1,
            step: Step::TurnedOut
        }],
        "the result lists the repaired operand, its step and why"
    );
    assert!(
        off(volume(&cut.mesh), volume(&right.mesh)) < 1e-9,
        "the turned cutter cuts as the right way out one does"
    );
}

#[test]
fn a_self_crossing_operand_is_rebuilt() {
    let ball = sphere(10);
    let copy = Similarity {
        translation: Vec3::new(0.5, 0.0, 0.0),
        scale: 0.7,
        ..I
    };
    let union = volume(&run(&[op(&ball, I, Union), op(&ball, copy, Union)]).mesh);
    for (name, mesh) in [("two shells", shells(&ball)), ("folded", fold(&ball))] {
        assert_eq!(steps(&mesh), [Step::Rebuilt], "{name}: step 4");
        let out = run(&[op(&mesh, I, Union)]);
        assert_eq!(
            out.repaired,
            [Repaired {
                operand: 0,
                step: Step::Rebuilt
            }],
            "{name}: listed"
        );
        assert_clean(&out.mesh, name);
        let named = out.sources.faces.iter().all(Option::is_some);
        assert!(named, "{name}: every face names its source");
        assert!(volume(&out.mesh) > 0.5, "{name}: a solid is left");
    }
    let two = volume(&run(&[op(&shells(&ball), I, Union)]).mesh);
    assert!(
        off(two, union) < 1e-6,
        "two shells rebuilt hold their union: {two} against {union}"
    );
}

#[test]
fn an_operand_the_ladder_cannot_mend_is_named_for_the_voxel_path() {
    let ball = sphere(10);
    let cut_by =
        |bad: &PlainMesh| boolean::fold(&[op(&ball, I, Union), op(bad, small_b(), Difference)]);
    match cut_by(&open(&ball)) {
        Err(BooleanError::Operand {
            operand: 1,
            why: Unfit::Open { edges },
        }) => assert!(edges > 0, "open edges counted"),
        other => panic!("an open cutter: {:?}", other.map(|o| o.mesh.faces.len())),
    }
    match cut_by(&fin(&ball)) {
        Err(BooleanError::Operand {
            operand: 1,
            why: Unfit::NotManifold { edges },
        }) => assert!(edges > 0, "the fin's edge counted"),
        other => panic!(
            "a cutter with a fin: {:?}",
            other.map(|o| o.mesh.faces.len())
        ),
    }
    let mut far = ball.clone();
    far.positions[3][1] = f32::NAN;
    let e = cut_by(&far).err();
    assert!(
        matches!(
            e,
            Some(BooleanError::Operand {
                operand: 1,
                why: Unfit::NotFinite
            })
        ),
        "a cutter with a point at no place: {e:?}"
    );
    let message = cut_by(&open(&ball)).err().map(|e| e.to_string());
    assert!(
        message.as_deref().is_some_and(|m| m.contains("operand 1")),
        "the message names the operand: {message:?}"
    );
}
