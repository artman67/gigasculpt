//! The metrics (decision 0061 section 1, as UV2's amendment defines them), each on a hand-made
//! layout with a known answer.

use std::collections::BTreeMap;

use gs_geo::uv::Corners;
use gs_geo::uv::metrics::{Layout, cover, density, flips, stretch};
use gs_protocol::{NO_VERTEX, PlainMesh};

use crate::{grid, surface};

fn near(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-6, "{what} is {got}, not {want}");
}

/// Each corner's UV as `uv` of its point.
fn mapped(mesh: &PlainMesh, uv: impl Fn([f32; 3]) -> [f32; 2]) -> Vec<Corners> {
    (mesh.faces.iter())
        .map(|face| {
            let mut c = [[0.0; 2]; 4];
            for (k, &v) in face.iter().enumerate().filter(|&(_, &v)| v != NO_VERTEX) {
                c[k] = uv(mesh.positions[v as usize]);
            }
            c
        })
        .collect()
}

/// Squares lying flat in x and z, each its own face and chart, at `corners` (low x, low z, side).
fn squares(corners: &[[f32; 3]]) -> PlainMesh {
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    for &[x, z, s] in corners {
        let v = positions.len() as u32;
        positions.extend([
            [x, 0.0, z],
            [x, 0.0, z + s],
            [x + s, 0.0, z + s],
            [x + s, 0.0, z],
        ]);
        faces.push([v, v + 1, v + 2, v + 3]);
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

#[test]
fn an_isometric_layout_has_no_stretch() {
    let mesh = grid(4);
    let uvs = mapped(&mesh, |p| [p[0], p[2]]);
    let chart = vec![0; mesh.faces.len()];
    let s = stretch(&Layout {
        surface: surface(&mesh),
        uvs: &uvs,
        chart: &chart,
    });
    near(s.l2, 1.0, "L2");
    near(s.p99, 1.0, "p99");
    near(s.worst, 1.0, "worst");
    near(s.angle, 1.0, "angle");
    near(s.area_spread, 0.0, "area spread");
}

#[test]
fn a_layout_stretched_two_to_one_has_its_known_values() {
    let mesh = grid(4);
    // Twice as long in u: the map back to the surface halves u, so its singular values are 1
    // and 1/2 before normalizing by the square root of the area ratio, 2.
    let uvs = mapped(&mesh, |p| [2.0 * p[0], p[2]]);
    let chart = vec![0; mesh.faces.len()];
    let s = stretch(&Layout {
        surface: surface(&mesh),
        uvs: &uvs,
        chart: &chart,
    });
    near(s.l2, 1.25_f64.sqrt(), "L2");
    near(s.p99, 2.0_f64.sqrt(), "p99");
    near(s.worst, 2.0_f64.sqrt(), "worst");
    near(s.angle, 2.0, "angle");
    near(s.area_spread, 0.0, "area spread");
}

#[test]
fn area_spread_is_the_mean_log_of_each_area_ratio_over_the_whole() {
    // Two equal squares, one at its size and one at twice it: ratios 1 and 4 against 2.5 overall,
    // whose logs average ln 2.
    let mesh = squares(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0]]);
    let uvs = mapped(&mesh, |p| {
        if p[0] < 1.5 {
            [p[0], p[2]]
        } else {
            [2.0 * p[0], 2.0 * p[2]]
        }
    });
    let s = stretch(&Layout {
        surface: surface(&mesh),
        uvs: &uvs,
        chart: &[0, 1],
    });
    near(s.area_spread, libm::log(2.0), "area spread");
}

#[test]
fn a_triangle_against_its_charts_winding_is_flipped_and_a_chart_against_the_surface_mirrored() {
    let positions = vec![
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [5.0, 0.0, 0.0],
        [5.0, 0.0, 1.0],
        [6.0, 0.0, 1.0],
    ];
    let faces = vec![
        [0, 1, 2, NO_VERTEX],
        [0, 2, 3, NO_VERTEX],
        [4, 5, 6, NO_VERTEX],
    ];
    let mesh = PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    };
    let uvs: Vec<Corners> = vec![
        // Counterclockwise, area 1/2.
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0; 2]],
        // Clockwise, area 1/8: against its chart.
        [[0.0, 0.0], [0.5, 0.5], [0.5, 0.0], [0.0; 2]],
        // Clockwise alone: its chart is mirrored, the triangle agrees with it.
        [[2.0, 0.0], [2.0, 1.0], [3.0, 1.0], [0.0; 2]],
    ];
    let f = flips(&Layout {
        surface: surface(&mesh),
        uvs: &uvs,
        chart: &[0, 0, 1],
    });
    assert_eq!(
        (f.flipped, f.mirrored),
        (1, 1),
        "one flipped triangle, one mirrored chart"
    );
}

#[test]
fn cover_counts_used_pixels_overlaps_and_the_narrowest_gap() {
    // At 16 pixels a side a square of side 0.25 covers 4 by 4 pixels, its diagonal's pixel
    // centres counted once.
    let apart = squares(&[[0.0, 0.0, 0.25], [0.5, 0.0, 0.25]]);
    let uvs = mapped(&apart, |p| [p[0], p[2]]);
    let c = cover(
        &Layout {
            surface: surface(&apart),
            uvs: &uvs,
            chart: &[0, 1],
        },
        16,
    );
    near(c.used, 32.0 / 256.0, "use");
    assert_eq!(c.overlap, 0, "nothing overlaps");
    assert_eq!(c.gap, Some(4), "pixels 4 to 7 lie between the squares");

    let over = squares(&[[0.0, 0.0, 0.25], [0.5, 0.0, 0.25], [0.125, 0.0, 0.25]]);
    let uvs = mapped(&over, |p| [p[0], p[2]]);
    let c = cover(
        &Layout {
            surface: surface(&over),
            uvs: &uvs,
            chart: &[0, 1, 2],
        },
        16,
    );
    near(c.used, 40.0 / 256.0, "use");
    assert_eq!(
        c.overlap, 8,
        "the third square lies on 2 by 4 of the first's pixels"
    );
    assert_eq!(c.gap, Some(0), "two charts share pixels");
}

#[test]
fn density_is_each_charts_uv_length_per_surface_length() {
    let mesh = squares(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0]]);
    let uvs = mapped(&mesh, |p| {
        if p[0] < 1.5 {
            [0.25 * p[0], 0.25 * p[2]]
        } else {
            [0.5 * p[0], 0.5 * p[2]]
        }
    });
    let d = density(&Layout {
        surface: surface(&mesh),
        uvs: &uvs,
        chart: &[0, 1],
    });
    near(d.p5, 0.25, "p5");
    near(d.p95, 0.5, "p95");
}
