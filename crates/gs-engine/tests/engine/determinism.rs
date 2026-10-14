//! Determinism (ARCHITECTURE 8): the same seed and calls give the same scene, random numbers
//! follow the documented stream, and the thread count never changes a result.

use std::path::PathBuf;

use gs_engine::EngineConfig;
use gs_protocol::Role;
use serde_json::json;

use gs_testkit::rng::signed_unit;
use gs_testkit::{Session, SplitMix64, success};

/// A script with every kind of logged call; returns the scene hash after each.
pub fn determinism_script(s: &mut Session) -> Vec<String> {
    let calls = [
        ("primitive.sphere", json!({ "subdivisions": 4 })),
        ("mesh.jitter", json!({ "amount": "2%" })),
        ("primitive.cube", json!({ "center": [3, 0, 0] })),
        (
            "mesh.jitter",
            json!({ "part": "cube", "amount": 0.05, "seed": 11 }),
        ),
        ("history.undo", json!({})),
        ("history.redo", json!({})),
        (
            "mesh.translate",
            json!({ "part": "sphere", "direction": [1, 2, 3], "distance": "10%" }),
        ),
        ("history.checkpoint", json!({ "name": "x" })),
        ("mesh.jitter", json!({ "part": "sphere", "amount": 0.01 })),
        ("history.restore", json!({ "name": "x" })),
    ];
    calls
        .into_iter()
        .map(|(command, args)| {
            s.ok(command, args);
            s.hash()
        })
        .collect()
}

/// An agent's session with this seed, logging to `log`.
pub fn seeded(seed: u64, log: Option<PathBuf>) -> Session {
    Session::with(
        EngineConfig {
            seed,
            log_path: log,
            ..EngineConfig::default()
        },
        Role::Agent,
    )
}

#[test]
fn the_same_script_gives_the_same_hashes() {
    let first = determinism_script(&mut seeded(7, None));
    let second = determinism_script(&mut seeded(7, None));
    assert_eq!(first, second, "same seed, same results at every step");
    let other = determinism_script(&mut seeded(8, None));
    assert_ne!(
        first.last(),
        other.last(),
        "the session seed drives unseeded randomness"
    );
    let (mut a, mut b) = (seeded(7, None), seeded(8, None));
    for s in [&mut a, &mut b] {
        s.ok("primitive.cube", json!({}));
        s.ok("mesh.jitter", json!({ "amount": 1, "seed": 5 }));
    }
    assert_eq!(
        a.hash(),
        b.hash(),
        "an explicit seed does not depend on the session seed"
    );
}

/// Vertex `i` of a part moves by value `3i + axis` of the seeded stream, `i` counting in the
/// order the part was made with (here not the mesh store's spatial order).
#[test]
fn random_values_follow_the_documented_stream() {
    let mut s = seeded(42, None);
    let base = [
        [0.0, 0.0, 4.0],
        [4.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.0, 4.0, 0.0],
    ];
    let tetra = json!({ "positions": base,
                        "faces": [[2, 3, 1, u32::MAX], [2, 1, 0, u32::MAX], [2, 0, 3, u32::MAX], [1, 3, 0, u32::MAX]] });
    s.ok("part.create", json!({ "name": "t", "mesh": tetra }));
    s.ok("mesh.jitter", json!({ "amount": 1, "seed": 1234 }));
    let expected: Vec<u64> = SplitMix64::new(1234).take(12).collect();
    let got: Vec<f32> = s.mesh("t").positions.into_iter().flatten().collect();
    let want: Vec<f32> = (base.iter().flatten().zip(&expected))
        .map(|(&c, &v): (&f32, &u64)| c + 1.0 * signed_unit(v))
        .collect();
    assert_eq!(
        got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        want.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        "offsets"
    );
    let response = s.call("mesh.jitter", json!({ "amount": 1 }));
    let seq = response.seq.unwrap_or_default();
    let done = success(response, "unseeded jitter");
    let session_stream: Vec<u64> = SplitMix64::new(42).take(seq as usize + 1).collect();
    assert_eq!(
        done.result["seed"],
        json!(session_stream[seq as usize]),
        "seed = value seq of the session stream"
    );
}

/// The same script on a private pool of one thread and of 24 gives the same scene after every
/// call. The parts span several mesh-store nodes, so the stores' parallel loops really split.
#[test]
fn one_thread_and_twenty_four_give_the_same_scene() {
    let run = |threads: usize| {
        gs_par::with_threads(threads, || {
            assert_eq!(gs_par::threads(), threads, "the private pool is in use");
            let mut s = seeded(5, None);
            let mut hashes = Vec::new();
            let mut call = |s: &mut Session, command: &str, args| {
                s.ok(command, args);
                hashes.push(s.hash());
            };
            call(&mut s, "primitive.sphere", json!({ "subdivisions": 96 }));
            call(&mut s, "mesh.jitter", json!({ "amount": "0.5%" }));
            let copy = s.mesh("sphere");
            call(
                &mut s,
                "part.create",
                json!({ "name": "copy", "mesh": copy }),
            );
            let moved = json!({ "part": "copy", "direction": [1, 1, 0], "distance": 3 });
            call(&mut s, "mesh.translate", moved);
            call(&mut s, "history.checkpoint", json!({ "name": "x" }));
            call(&mut s, "history.undo", json!({}));
            call(&mut s, "history.redo", json!({}));
            let fine = json!({ "part": "sphere", "amount": 0.001, "seed": 9 });
            call(&mut s, "mesh.jitter", fine);
            // The heaviest parallel code in the engine (review 0002 T4).
            let remesh = json!({ "part": "copy", "resolution": 32, "project": true, "blur": 30 });
            call(&mut s, "mesh.voxel_remesh", remesh);
            call(&mut s, "history.restore", json!({ "name": "x" }));
            hashes
        })
    };
    let one = run(1);
    assert_eq!(one.len(), 10, "every call ran");
    assert_eq!(one, run(24), "the same scene after every call");
}

/// On a part of two mesh-store nodes, every vertex moves by its own values of the stream,
/// counted in the part's numbering: for a primitive (the store's order) and for a part made
/// from plain data (the order given).
#[test]
fn every_vertex_of_a_bigger_part_follows_the_stream() {
    let mut s = seeded(1, None);
    s.ok(
        "primitive.sphere",
        json!({ "name": "ball", "subdivisions": 40 }),
    );
    let base = s.mesh("ball");
    s.ok("part.create", json!({ "name": "copy", "mesh": base }));
    let stream: Vec<u64> = SplitMix64::new(77).take(base.positions.len() * 3).collect();
    let want: Vec<u32> = (base.positions.iter().flatten().zip(&stream))
        .map(|(&c, &v): (&f32, &u64)| (c + 0.001 * signed_unit(v)).to_bits())
        .collect();
    for part in ["ball", "copy"] {
        s.ok(
            "mesh.jitter",
            json!({ "part": part, "amount": 0.001, "seed": 77 }),
        );
        let got: Vec<u32> = (s.mesh(part).positions.iter().flatten())
            .map(|c| c.to_bits())
            .collect();
        assert_eq!(got.len(), 9602 * 3, "{part}: two nodes' worth");
        assert!(got == want, "{part}: every vertex follows the stream");
    }
}
