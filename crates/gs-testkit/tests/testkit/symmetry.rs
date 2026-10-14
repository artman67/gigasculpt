//! The metamorphic helpers transform exactly what they say, undo exactly, and expose a command
//! that does not commute with them.

use std::collections::BTreeSet;

use gs_protocol::PlainMesh;
use gs_testkit::compare::{identical, vertex_diff};
use gs_testkit::fixtures::{cube, tetra, torus};
use gs_testkit::measure::{hausdorff, stats};
use gs_testkit::symmetry::{CubeSymmetry, Renumbering, select};
use gs_testkit::{Expect, Session, validate};
use serde_json::{Value, json};

#[test]
fn there_are_48_distinct_symmetries_half_of_them_mirrors() {
    let all = CubeSymmetry::all();
    let images: BTreeSet<[i32; 3]> = all.iter().map(|g| g.apply([1, 2, 3])).collect();
    assert_eq!(images.len(), 48, "48 different images of (1, 2, 3)");
    assert_eq!(
        all[0].apply([1, 2, 3]),
        [1, 2, 3],
        "the identity comes first"
    );
    assert!(!all[0].is_reflection(), "the identity does not mirror");
    assert_eq!(
        all.iter().filter(|g| g.is_reflection()).count(),
        24,
        "24 mirror"
    );
    let g = CubeSymmetry {
        axes: [1, 2, 0],
        flips: [true, false, false],
    };
    assert_eq!(
        g.apply([1, 2, 3]),
        [-2, 3, 1],
        "axis 0 takes -y, axis 1 z, axis 2 x"
    );
    assert!(
        g.is_reflection(),
        "a rotation of the axes with one flip mirrors"
    );
    for g in &all {
        assert_eq!(
            g.inverse().apply(g.apply([1, 2, 3])),
            [1, 2, 3],
            "{g:?} undone"
        );
        assert_eq!(g.inverse().inverse(), *g, "{g:?}: inverse twice");
    }
}

/// Every symmetry keeps a closed mesh closed and outward-facing, with the same volume.
#[test]
fn symmetric_meshes_keep_their_shape_and_their_outside() {
    let base = torus();
    let volume = stats(&base).volume;
    for g in CubeSymmetry::all() {
        let moved = g.mesh(&base);
        assert_eq!(
            validate(&moved, Expect::closed(0)),
            Ok(()),
            "{g:?} keeps a torus"
        );
        assert!(
            (stats(&moved).volume - volume).abs() < 1e-6,
            "{g:?} keeps the volume outward"
        );
        let back = g.inverse().mesh(&moved);
        assert!(identical(&back, &base), "{g:?} undone exactly");
    }
}

#[test]
fn arguments_transform_at_their_pointers() {
    let g = CubeSymmetry {
        axes: [2, 0, 1],
        flips: [false, true, false],
    };
    let args = json!({
        "direction": [1, 2, 3],
        "center": [1, 2, 3],
        "distance": 2,
        "samples": [{ "position": [1, 0, 0], "pressure": [1, 2, 3] }, { "position": [0, 0, 1] }],
        "odd": [1, 2],
    });
    let pointers = [
        "/direction",
        "/samples/*/position",
        "/odd",
        "/missing",
        "/samples/*/x",
    ];
    let moved = g.args(&args, &pointers.map(String::from));
    let want = json!({
        "direction": [3.0, -1.0, 2.0],
        "center": [1, 2, 3],
        "distance": 2,
        "samples": [{ "position": [0.0, -1.0, 0.0], "pressure": [1, 2, 3] },
                    { "position": [1.0, -0.0, 0.0] }],
        "odd": [1, 2],
    });
    assert_eq!(moved, want, "only the named vectors move");
    let mut value = json!({ "a": [{ "b": 1 }, { "b": 2 }, { "c": 3 }] });
    let found: Vec<Value> = select(&mut value, "/a/*/b")
        .into_iter()
        .map(|v| v.clone())
        .collect();
    assert_eq!(
        found,
        [json!(1), json!(2)],
        "a wildcard step reaches every element"
    );
    assert_eq!(
        select(&mut value, "").len(),
        1,
        "the empty pointer is the whole value"
    );
}

#[test]
fn renumbering_changes_the_numbers_but_not_the_shape() {
    let base = tetra();
    for seed in 0..20 {
        let order = Renumbering::random(&base, seed);
        let shuffled = order.apply(&base);
        assert_eq!(
            validate(&shuffled, Expect::closed(2)),
            Ok(()),
            "seed {seed}: still valid"
        );
        assert!(
            (stats(&shuffled).volume - 1.0).abs() < 1e-12,
            "seed {seed}: same volume"
        );
        assert_eq!(
            hausdorff(&shuffled, &base),
            0.0,
            "seed {seed}: same surface"
        );
        assert!(
            identical(&order.undo(&shuffled), &base),
            "seed {seed}: undone exactly"
        );
    }
    let changed = (0..20).filter(|&seed| {
        let shuffled = Renumbering::random(&base, seed).apply(&base);
        shuffled.positions != base.positions && shuffled.faces != base.faces
    });
    assert!(
        changed.count() >= 15,
        "most orders move both vertices and faces"
    );
    let big = cube(3);
    let order = Renumbering::random(&big, 7);
    assert!(
        identical(&order.undo(&order.apply(&big)), &big),
        "a bigger mesh comes back too"
    );
}

/// Applies `command` to `mesh`, through a session, and returns the result.
fn after(command: &str, args: &Value, mesh: &PlainMesh) -> PlainMesh {
    let mut s = Session::new();
    s.add_part("p", mesh);
    s.ok(command, args.clone());
    s.mesh("p")
}

/// The relations hold for a command that should satisfy them, and fail for one that does not,
/// so they can tell the difference.
#[test]
fn relations_separate_a_command_that_commutes_from_one_that_does_not() {
    let base = torus();
    let order = Renumbering::random(&base, 3);
    let translate = json!({ "direction": [1, 2, 3], "distance": 0.5 });
    let jitter = json!({ "amount": 0.01, "seed": 5 });
    let renumbered = |args: &Value, command: &str| {
        let direct = after(command, args, &base);
        let via = order.undo(&after(command, args, &order.apply(&base)));
        vertex_diff(&direct, &via).max
    };
    assert_eq!(
        renumbered(&translate, "mesh.translate"),
        0.0,
        "translate ignores numbering"
    );
    assert!(
        renumbered(&jitter, "mesh.jitter") > 1e-3,
        "jitter follows the numbering"
    );
    let mirrored = |args: &Value, command: &str, g: CubeSymmetry| {
        let direct = g.mesh(&after(command, args, &base));
        let moved_args = g.args(args, &["/direction".to_string()]);
        let via = after(command, &moved_args, &g.mesh(&base));
        vertex_diff(&direct, &via).max
    };
    for g in CubeSymmetry::all() {
        assert!(
            mirrored(&translate, "mesh.translate", g) < 1e-6,
            "translate commutes with {g:?}"
        );
    }
    let flip_x = CubeSymmetry {
        axes: [0, 1, 2],
        flips: [true, false, false],
    };
    assert!(
        mirrored(&jitter, "mesh.jitter", flip_x) > 1e-3,
        "jitter does not mirror"
    );
}
