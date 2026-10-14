//! Non-triviality: each fixture really is the shape it claims to be, so no property can pass
//! because a fixture collapsed (research 06, section 2.4).

use std::f64::consts::PI;

use gs_protocol::PlainMesh;
use gs_testkit::fixtures::{self, cube, cylinder, icosphere, patch, torus};
use gs_testkit::measure::{bounds, stats};
use gs_testkit::{Expect, validate};

fn radius(p: [f32; 3]) -> f64 {
    p.iter()
        .map(|&c| f64::from(c) * f64::from(c))
        .sum::<f64>()
        .sqrt()
}

fn edge_lengths(mesh: &PlainMesh) -> Vec<f64> {
    let mut out = Vec::new();
    for face in &mesh.faces {
        for k in 0..3 {
            let (a, b) = (
                mesh.positions[face[k] as usize],
                mesh.positions[face[(k + 1) % 3] as usize],
            );
            out.push(radius(std::array::from_fn(|i| a[i] - b[i])));
        }
    }
    out
}

#[test]
fn the_icosphere_is_a_regular_unit_sphere() {
    let base = icosphere(0);
    let (vertices, faces) = (base.positions.len(), base.faces.len());
    assert_eq!((vertices, faces), (12, 20), "an icosahedron");
    let lengths = edge_lengths(&base);
    let spread = lengths
        .iter()
        .fold(0.0f64, |m, l| m.max((l - lengths[0]).abs()));
    assert!(spread < 1e-6, "every edge the same length: spread {spread}");
    for n in 0..=3 {
        let mesh = icosphere(n);
        assert_eq!(
            validate(&mesh, Expect::closed(2)),
            Ok(()),
            "level {n} is a closed sphere"
        );
        let worst = mesh
            .positions
            .iter()
            .map(|&p| (radius(p) - 1.0).abs())
            .fold(0.0, f64::max);
        assert!(
            worst < 1e-6,
            "level {n}: every vertex on the unit sphere ({worst})"
        );
        assert_eq!(mesh.faces.len(), 20 * 4usize.pow(n), "level {n} faces");
    }
    let ratio = stats(&icosphere(3)).volume / (4.0 / 3.0 * PI);
    assert!(
        (0.98..1.0).contains(&ratio),
        "level 3 holds {ratio} of the sphere's volume"
    );
}

#[test]
fn the_cube_is_a_cube() {
    for n in 1..=4 {
        let mesh = cube(n);
        let s = stats(&mesh);
        let n = u64::from(n);
        assert_eq!(
            (s.vertices, s.quads),
            (6 * n * n + 2, 6 * n * n),
            "cube {n} counts"
        );
        assert!(
            (s.volume - 8.0).abs() < 1e-9 && (s.area - 24.0).abs() < 1e-9,
            "cube {n}: {s:?}"
        );
        assert!(
            mesh.positions.iter().flatten().all(|c| c.abs() <= 1.0),
            "inside [-1, 1]"
        );
    }
}

#[test]
fn the_torus_has_a_hole() {
    let mesh = torus();
    assert_eq!(
        validate(&mesh, Expect::closed(0)),
        Ok(()),
        "closed with Euler characteristic 0"
    );
    for &[x, y, z] in &mesh.positions {
        let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
        let out = (x * x + z * z).sqrt() - 1.0;
        let ring = (out * out + y * y).sqrt();
        assert!(
            (ring - 0.375).abs() < 1e-6,
            "({x}, {y}, {z}) is {ring} from the core circle"
        );
    }
    // Twelve-sided rings both ways: the section is 3r^2 instead of pi r^2, swept around a
    // twelve-gon whose centroid path is a little shorter than the circle.
    let ratio = stats(&mesh).volume / (2.0 * PI * PI * 0.375 * 0.375);
    assert!((0.85..1.0).contains(&ratio), "volume ratio {ratio}");
}

#[test]
fn the_patch_is_open_and_domed() {
    let mesh = patch(4);
    let s = stats(&mesh);
    assert_eq!(
        (s.vertices, s.quads, s.boundary_edges),
        (25, 16, 16),
        "a 4x4 grid"
    );
    assert_eq!(validate(&mesh, Expect::open(1)), Ok(()), "a disc");
    let top = mesh.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
    assert!(
        (top - 0.25).abs() < 1e-6,
        "the dome rises to 0.25 in the middle: {top}"
    );
    assert!(s.area > 4.0, "more than the flat square: {}", s.area);
    assert_eq!(
        bounds(&mesh.positions),
        Some([[-1.0, 0.0, -1.0], [1.0, 0.25, 1.0]]),
        "spans -1 to 1, rising from 0 to 0.25"
    );
    for p in &mesh.positions {
        let rim = p[0].abs().to_bits() == 1f32.to_bits() || p[2].abs().to_bits() == 1f32.to_bits();
        assert!(!rim || p[1].to_bits() == 0, "the rim is flat: {p:?}");
    }
}

#[test]
fn the_cylinder_mixes_quads_and_triangles() {
    let mesh = cylinder();
    let s = stats(&mesh);
    assert_eq!((s.vertices, s.triangles, s.quads), (26, 24, 12), "counts");
    assert!(
        (s.volume - 6.0).abs() < 1e-6,
        "a twelve-sided prism: {}",
        s.volume
    );
    assert!(
        (s.area - (6.0 + 12.0 * 2.0 * (2.0 - 3f64.sqrt()).sqrt())).abs() < 1e-6,
        "area {}",
        s.area
    );
}

#[test]
fn the_standard_set_is_named_valid_and_wound_outward() {
    let all = fixtures::all();
    let names: Vec<&str> = all.iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        ["icosphere", "cube", "torus", "patch", "cylinder"],
        "names"
    );
    for f in &all {
        assert_eq!(validate(&f.mesh, f.expect), Ok(()), "{}", f.name);
        let volume = stats(&f.mesh).volume;
        if f.expect.closed {
            assert!(
                volume > 1.0,
                "{} is wound outward with volume {volume}",
                f.name
            );
        }
        assert!(
            f.mesh.positions.len() <= 200,
            "{} is small enough for property tests",
            f.name
        );
    }
}
