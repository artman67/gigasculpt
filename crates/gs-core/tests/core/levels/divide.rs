//! Adding a level: decision 0006's numbering, Catmull-Clark positions against the slow
//! reference, creases, flat steps, and no global sort.

use std::collections::BTreeMap;

use gs_core::{NO, Settings, subdivide_faces};
use gs_protocol::PlainMesh;
use gs_testkit::{SplitMix64, fixtures};

use super::reference::{self, Mesh};
use super::{faces, positions, stack};
use crate::common::{check, ok, snap, sphere};

/// The testkit's unit cube grid with `n` quads per side edge.
fn cube(n: u32) -> PlainMesh {
    fixtures::cube(n)
}

/// A `w` by `h` grid of quads in the plane z = 0, some split into triangles: an open mesh with
/// a boundary and corners.
fn grid(w: u32, h: u32, seed: u64) -> PlainMesh {
    let mut rng = SplitMix64::new(seed);
    let mut positions = Vec::new();
    for y in 0..=h {
        for x in 0..=w {
            let z = (rng.below(1000) as f32) / 4000.0;
            positions.push([x as f32, y as f32, z]);
        }
    }
    let mut faces = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let a = y * (w + 1) + x;
            let (b, c, d) = (a + 1, a + w + 2, a + w + 1);
            if rng.below(3) == 0 {
                faces.extend([[a, b, c, NO], [a, c, d, NO]]);
            } else {
                faces.push([a, b, c, d]);
            }
        }
    }
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

/// Compares the active level with the reference, within `tol` of the mesh size.
fn matches(got: &[gs_core::Vec3], want: &[[f64; 3]], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: point counts");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let g = g.to_array();
        let d = (0..3)
            .map(|k| (f64::from(g[k]) - w[k]).abs())
            .fold(0.0, f64::max);
        assert!(d <= tol, "{what}: point {i} is {g:?}, the reference {w:?}");
    }
}

#[test]
fn a_cube_divides_to_26_points_and_24_faces_keeping_its_corner_numbers() {
    let mesh = cube(1);
    let mut l = stack(&mesh, &Settings::default());
    let before = positions(&l);
    ok(l.divide(false));
    assert_eq!((l.count(), l.active()), (2, 1));
    assert_eq!(l.store().vertex_count(), 26);
    assert_eq!(l.store().face_count(), 24);
    let want = subdivide_faces(&mesh.faces, 8).map(|x| x.0);
    assert_eq!(Some(faces(&l)), want, "faces follow decision 0006's rule");
    let flat = positions(&l);
    assert_eq!(
        super::bits(&flat[..8]),
        super::bits(&before),
        "a flat step keeps points 0 to 7"
    );
    let mut smooth = stack(&mesh, &Settings::default());
    ok(smooth.divide(true));
    let moved = positions(&smooth);
    assert!(
        moved[..8]
            .iter()
            .zip(&before)
            .all(|(a, b)| a.bits() != b.bits()),
        "smoothing moves the corners in"
    );
    assert_eq!(
        faces(&smooth),
        faces(&l),
        "smooth and flat steps number alike"
    );
}

/// A test mesh: its name, the mesh, its creases and whether boundary corners are pinned.
type Case = (String, PlainMesh, Vec<(u32, u32, u8)>, bool);

#[test]
fn catmull_clark_matches_the_slow_reference() {
    let cases: Vec<Case> = vec![
        (
            "quad sphere".into(),
            sphere(6, 0.0, 3, false),
            vec![],
            false,
        ),
        (
            "mixed sphere".into(),
            sphere(5, 0.5, 4, false),
            vec![],
            false,
        ),
        (
            "triangle sphere".into(),
            sphere(4, 1.0, 5, false),
            vec![],
            false,
        ),
        ("open grid".into(), grid(7, 5, 6), vec![], false),
        (
            "open grid, pinned corners".into(),
            grid(6, 6, 7),
            vec![],
            true,
        ),
        (
            "creased cube".into(),
            cube(2),
            vec![(0, 1, 1), (1, 2, 2), (5, 6, 255)],
            false,
        ),
    ];
    for (name, mesh, creases, pin) in cases {
        // Only creases on real edges: take them from the first faces.
        let creases: Vec<(u32, u32, u8)> = if creases.is_empty() {
            creases
        } else {
            mesh.faces[..3]
                .iter()
                .zip(&creases)
                .map(|(f, c)| (f[0], f[1], c.2))
                .collect()
        };
        let settings = Settings {
            creases: creases.clone(),
            pin_corners: pin,
            corners: Vec::new(),
        };
        let mut l = stack(&mesh, &settings);
        let mut m = Mesh {
            faces: mesh.faces.clone(),
            pos: (mesh.positions.iter()).map(|p| p.map(f64::from)).collect(),
            sharp: creases
                .iter()
                .map(|&(a, b, s)| ((a.min(b), a.max(b)), s))
                .collect::<BTreeMap<_, _>>(),
        };
        for depth in 1..=3 {
            ok(l.divide(true));
            m = reference::step(&m, true, pin);
            assert_eq!(faces(&l), m.faces, "{name}, level {depth}: faces");
            matches(
                &positions(&l),
                &m.pos,
                2e-5,
                &format!("{name}, level {depth}"),
            );
            assert_eq!(
                check(&snap(l.store())),
                Ok(()),
                "{name}, level {depth}: store invariants"
            );
        }
    }
}

#[test]
fn a_flat_step_puts_new_points_on_the_old_faces() {
    let mesh = sphere(4, 0.4, 9, false);
    let mut l = stack(&mesh, &Settings::default());
    let mut m = Mesh {
        faces: mesh.faces.clone(),
        pos: (mesh.positions.iter()).map(|p| p.map(f64::from)).collect(),
        sharp: BTreeMap::new(),
    };
    for depth in 1..=2 {
        ok(l.divide(false));
        m = reference::step(&m, false, false);
        matches(&positions(&l), &m.pos, 1e-6, &format!("flat level {depth}"));
    }
}

#[test]
fn sharp_creases_hold_the_cube_for_their_level_count() {
    let mesh = cube(1);
    let edges: Vec<(u32, u32, u8)> = (mesh.faces.iter())
        .flat_map(|f| (0..4).map(move |i| (f[i], f[(i + 1) % 4], 2)))
        .collect();
    let settings = Settings {
        creases: edges,
        ..Settings::default()
    };
    let mut l = stack(&mesh, &settings);
    let corners = super::bits(&positions(&l));
    for depth in 1..=3 {
        ok(l.divide(true));
        let now = super::bits(&positions(&l)[..8]);
        if depth <= 2 {
            assert_eq!(
                now, corners,
                "level {depth}: every corner has three sharp edges and stays"
            );
        } else {
            assert!(
                now.iter().zip(&corners).all(|(a, b)| a != b),
                "level 3: the creases ran out"
            );
        }
    }
}

#[test]
fn neither_divide_nor_its_store_sorts() {
    let mut l = stack(&sphere(6, 0.3, 11, true), &Settings::default());
    let sorted = l.store().work().sorts;
    assert_eq!(sorted, 1, "the import sorts once");
    ok(l.divide(true));
    ok(l.divide(true));
    assert_eq!(
        l.store().work().sorts,
        0,
        "levels are cut from the level below"
    );
    assert_eq!(check(&snap(l.store())), Ok(()));
    let nodes = l.store().node_count();
    assert!(
        nodes >= 16,
        "the cut made {nodes} nodes from a small-node base"
    );
}
