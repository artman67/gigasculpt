//! gs-geo quad remesh tests (decision 0038): the pin is the lock's; the size formula lands the
//! count near the target; a fine open rim is resampled and the result stays manifold; the
//! sharp-edge file's indices put patch borders on the group borders. The command's own tests
//! are in gs-engine. These run the program, built once per pin (`third_party/quadwild/`);
//! `stand_in` runs a stand-in for it that dies or leaves nothing.

#[cfg(unix)]
mod stand_in;

use std::path::PathBuf;

use gs_geo::quad::{Job, PIN, Program, Quads, Settings, bwrap_in, places, remesh};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::fixtures;
use gs_testkit::validate::{Expect, validate};

/// A fresh job folder for the test `name`.
fn job(name: &str) -> Job {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("quad-{name}-{}", std::process::id()));
    Job::new(dir).unwrap_or_else(|e| panic!("a fresh job folder: {e}"))
}

/// The program, found as the engine finds it; missing, the test fails naming the fix.
fn program() -> Program {
    let env = std::env::var_os;
    let at = places(env("GS_QUADWILD"), None, env("XDG_CACHE_HOME"), env("HOME"));
    Program::find(&at, bwrap_in(env("PATH"))).unwrap_or_else(|why| {
        panic!(
            "the Quad Remesh program is missing ({why}); build it with \
             third_party/quadwild/fetch.sh and build.sh (decision 0038, section 1)"
        )
    })
}

/// `mesh` remeshed to about `faces` quads in a job folder of its own.
fn quads(name: &str, mesh: &PlainMesh, settings: &Settings) -> Quads {
    remesh(mesh, settings, &program(), &job(name), &|| false)
        .unwrap_or_else(|e| panic!("quads: {e}"))
}

fn to(faces: usize) -> Settings {
    Settings {
        faces,
        ..Settings::default()
    }
}

#[test]
fn the_pin_is_the_one_the_lock_names() {
    let lock = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/quadwild/sources.lock"),
    )
    .expect("the lock");
    let pin = lock.lines().find_map(|l| l.strip_prefix("pin "));
    assert_eq!(
        pin.map(str::trim),
        Some(PIN),
        "gs_geo::quad::PIN and the lock's pin"
    );
}

#[test]
fn a_sphere_becomes_closed_quads_near_the_count() {
    let out = quads("sphere", &fixtures::icosphere(4), &to(500));
    let n = out.mesh.faces.len();
    assert!((450..=550).contains(&n), "{n} quads for 500");
    assert!(
        out.mesh.faces.iter().all(|f| !f.contains(&u32::MAX)),
        "quads only"
    );
    let found = validate(&out.mesh, Expect::closed(2));
    assert!(found.is_ok(), "a closed sphere of quads: {found:?}");
}

#[test]
fn a_fine_rim_is_resampled_and_stays_one_open_loop() {
    let out = quads("rim", &fixtures::patch(40), &to(100));
    let rim = out.preparation.rim_points_removed;
    assert!(rim >= 60, "{rim} of the 160 rim points went");
    let found = validate(&out.mesh, Expect::open(1));
    assert!(found.is_ok(), "an open disc of quads: {found:?}");
}

#[test]
fn group_borders_become_patch_borders() {
    let mut cube = fixtures::cube(8);
    let side = |f: &[u32; 4]| {
        let c = f.map(|v| cube.positions[v as usize]);
        let axis = (0..3)
            .find(|&k| c.iter().all(|p| (p[k] - c[0][k]).abs() < 1e-6))
            .unwrap_or(0);
        (2 * axis) as u32 + u32::from(c[0][axis] > 0.0)
    };
    let groups = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(cube.faces.iter().map(side).collect()),
    };
    cube.attributes.insert("group".into(), groups);
    let settings = Settings {
        keep_groups: true,
        ..to(300)
    };
    let out = quads("groups", &cube, &settings);
    assert_eq!(
        out.preparation.sharp_edges,
        12 * 8,
        "every cube edge, once each"
    );
    // Each quad lies on one side of the cube: its four points share a coordinate of +-1.
    let on_one_side = |f: &[u32; 4]| {
        let c = f.map(|v| out.mesh.positions[v as usize]);
        (0..3)
            .any(|k| (c.iter().all(|p| (p[k].abs() - 1.0).abs() < 0.02)) && c[0][k] * c[2][k] > 0.0)
    };
    let across = out.mesh.faces.iter().filter(|f| !on_one_side(f)).count();
    assert_eq!(across, 0, "quads that cross a group border");
}
