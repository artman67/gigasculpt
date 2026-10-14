//! The measures give the known answers on shapes whose measurements are known exactly, and a
//! broken shape cannot hide behind them (NaN is infinitely far, an empty mesh is not close).

use gs_protocol::PlainMesh;
use gs_testkit::fixtures;
use gs_testkit::measure::{Stats, bounds, hausdorff, hausdorff_from, stats};

use crate::data::{joined, moved, tetra, triangles};

fn close(got: f64, want: f64, what: &str) {
    assert!(
        (got - want).abs() <= 1e-12 * want.abs().max(1.0),
        "{what}: got {got}, want {want}"
    );
}

#[test]
fn counts_area_and_volume_of_known_shapes() {
    let s = stats(&tetra());
    let counts = (
        s.vertices,
        s.faces,
        s.triangles,
        s.quads,
        s.edges,
        s.boundary_edges,
    );
    assert_eq!(counts, (4, 4, 4, 0, 6, 0), "tetrahedron counts");
    close(s.area, 9.0, "tetrahedron area");
    close(s.volume, 1.0, "tetrahedron volume");
    let mut inside_out = tetra();
    inside_out.faces.iter_mut().for_each(|f| f[..3].reverse());
    close(stats(&inside_out).volume, -1.0, "turned inside out");
    let cube = stats(&fixtures::cube(2));
    let counts = (
        cube.vertices,
        cube.quads,
        cube.triangles,
        cube.edges,
        cube.boundary_edges,
    );
    assert_eq!(counts, (26, 24, 0, 48, 0), "cube counts");
    close(cube.area, 24.0, "cube area");
    close(cube.volume, 8.0, "cube volume");
    let open = stats(&fixtures::patch(3));
    assert_eq!(
        (open.quads, open.edges, open.boundary_edges),
        (9, 24, 12),
        "patch counts"
    );
    let cylinder = stats(&fixtures::cylinder());
    assert_eq!(
        (cylinder.triangles, cylinder.quads),
        (24, 12),
        "mixed faces"
    );
    let prism = (cylinder.volume - 6.0).abs();
    assert!(
        prism < 1e-6,
        "a twelve-sided prism, radius 1, height 2: off by {prism}"
    );
}

#[test]
fn measures_follow_size_and_position() {
    let base = stats(&tetra());
    let mut doubled = tetra();
    doubled
        .positions
        .iter_mut()
        .flatten()
        .for_each(|c| *c *= 2.0);
    let big = stats(&doubled);
    close(big.area, 4.0 * base.area, "area scales with the square");
    close(big.volume, 8.0 * base.volume, "volume scales with the cube");
    let shifted = stats(&moved(&tetra(), [3.0, -2.0, 0.5]));
    close(
        shifted.volume,
        base.volume,
        "a closed volume does not depend on position",
    );
    let empty = stats(&PlainMesh::default());
    assert_eq!(
        empty,
        Stats {
            vertices: 0,
            faces: 0,
            triangles: 0,
            quads: 0,
            edges: 0,
            boundary_edges: 0,
            area: 0.0,
            volume: 0.0
        },
        "nothing"
    );
}

#[test]
fn bounds_hold_the_extremes() {
    let points = [[1.0, -2.0, 3.0], [-4.0, 5.0, 0.5], [2.0, 0.0, -6.0]];
    assert_eq!(
        bounds(&points),
        Some([[-4.0, -2.0, -6.0], [2.0, 5.0, 3.0]]),
        "per axis"
    );
    assert_eq!(
        bounds(&points[..1]),
        Some([points[0], points[0]]),
        "one point"
    );
    assert_eq!(bounds(&[]), None, "no points");
}

fn square(y: f32, x0: f32) -> PlainMesh {
    let x1 = x0 + 2.0;
    let corners = [[x0, y, -1.0], [x1, y, -1.0], [x1, y, 1.0], [x0, y, 1.0]];
    triangles(&corners, &[[0, 2, 1], [0, 3, 2]])
}

fn tiny_triangle_at(c: [f32; 3]) -> PlainMesh {
    let d = 0.001;
    let points = [c, [c[0] + d, c[1], c[2]], [c[0], c[1], c[2] + d]];
    triangles(&points, &[[0, 2, 1]])
}

#[test]
fn hausdorff_distances_of_known_arrangements() {
    let cube = fixtures::cube(1);
    assert_eq!(hausdorff(&cube, &cube.clone()), 0.0, "the same mesh");
    let shifted = moved(&cube, [0.25, 0.0, 0.0]);
    close(
        hausdorff(&cube, &shifted),
        0.25,
        "a cube moved a quarter along x",
    );
    close(hausdorff(&shifted, &cube), 0.25, "either way round");
    close(
        hausdorff(&square(0.0, -1.0), &square(1.0, -1.0)),
        1.0,
        "two squares stacked 1 apart",
    );
    close(
        hausdorff(&square(0.0, -1.0), &square(0.0, 4.0)),
        5.0,
        "two squares side by side",
    );
    let big = triangles(
        &[[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, 4.0]],
        &[[0, 2, 1]],
    );
    let from = |c: [f32; 3]| hausdorff_from(&tiny_triangle_at(c), &big);
    assert!(
        (from([1.0, 2.0, 1.0]) - 2.0).abs() < 0.01,
        "over the face: {}",
        from([1.0, 2.0, 1.0])
    );
    assert!(
        (from([1.0, 0.0, -3.0]) - 3.0).abs() < 0.01,
        "beside an edge: {}",
        from([1.0, 0.0, -3.0])
    );
    let corner = from([-3.0, 0.0, -3.0]);
    assert!(
        (corner - 18f64.sqrt()).abs() < 0.01,
        "beyond a corner: {corner}"
    );
}

/// One-sided distances can be zero while the shapes differ; the two-sided one cannot.
#[test]
fn hausdorff_is_two_sided() {
    let cube = fixtures::cube(1);
    let more = joined(&cube, &moved(&cube, [10.0, 0.0, 0.0]));
    assert_eq!(
        hausdorff_from(&cube, &more),
        0.0,
        "the cube lies on the pair"
    );
    assert!(
        hausdorff_from(&more, &cube) >= 9.0,
        "the far cube does not lie on the first"
    );
    assert!(
        hausdorff(&cube, &more) >= 9.0,
        "so the two-sided distance is large"
    );
    assert!(hausdorff(&more, &cube) >= 9.0, "in either order");
}

#[test]
fn hausdorff_never_calls_broken_or_missing_geometry_close() {
    let cube = fixtures::cube(1);
    for broken in [f32::NAN, f32::INFINITY] {
        let mut bad = cube.clone();
        bad.positions[3][1] = broken;
        assert_eq!(
            hausdorff(&cube, &bad),
            f64::INFINITY,
            "{broken} in the second"
        );
        assert_eq!(
            hausdorff(&bad, &cube),
            f64::INFINITY,
            "{broken} in the first"
        );
        assert_eq!(
            hausdorff_from(&cube, &bad),
            f64::INFINITY,
            "{broken}, one-sided"
        );
    }
    let empty = PlainMesh::default();
    assert_eq!(hausdorff(&cube, &empty), f64::INFINITY, "against nothing");
    assert_eq!(hausdorff(&empty, &empty), 0.0, "nothing against nothing");
    let points = PlainMesh {
        faces: Vec::new(),
        ..cube.clone()
    };
    assert_eq!(hausdorff_from(&points, &cube), 0.0, "vertices on the cube");
    assert_eq!(
        hausdorff_from(&cube, &points),
        f64::INFINITY,
        "no faces to measure against"
    );
}
