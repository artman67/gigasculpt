//! The flattener (decision 0061 sections 1, 2 and 7, step UV3): LSCM per chart lays shapes that
//! lie flat out with no stretch and at their own size, flips no triangle on the cutter's charts
//! of the bench shapes, lays each chart along u, mirrors twins exactly and a chart across the
//! plane onto itself, and gives the same bits at 1 and 24 threads.

use std::collections::BTreeMap;

use gs_geo::uv::metrics::{Layout, flips, stretch};
use gs_geo::uv::{Charts, Corners, Flattener, Steer, Symmetry, cut_with, seams};
use gs_protocol::{NO_VERTEX, PlainMesh, corners};
use gs_testkit::bench_shapes::head;

use crate::symmetry::{in_x, partners};
use crate::{charts, grid, shapes, surface};

/// Every chart of `charts` flattened under `symmetry`, written into its faces' corners.
fn laid_out(mesh: &PlainMesh, charts: &Charts, symmetry: Option<Symmetry>) -> Vec<Corners> {
    let surface = surface(mesh);
    let flattener = Flattener::new(surface, symmetry).unwrap_or_else(|r| panic!("refused: {r}"));
    let meshes = charts.meshes(surface);
    let mut out = vec![[[0.0; 2]; 4]; mesh.faces.len()];
    for (chart, flat) in meshes.iter().zip(flattener.charts(&meshes)) {
        chart.write(&flat.uvs, &mut out);
    }
    out
}

/// A surface of revolution about y, `m` points around: a ring per `[radius, height]`, quads
/// between rings and, with `apex` (a height above the first ring), triangles up to a point
/// there. With it a layout whose only seams are the edges where the surface closes around.
fn turned(m: u32, rings: &[[f64; 2]], apex: Option<f64>) -> (PlainMesh, Vec<Corners>) {
    let mut positions = Vec::new();
    for &[r, y] in rings {
        for i in 0..m {
            let t = 2.0 * std::f64::consts::PI * f64::from(i) / f64::from(m);
            positions.push([
                (r * libm::cos(t)) as f32,
                y as f32,
                (r * libm::sin(t)) as f32,
            ]);
        }
    }
    let at = |i: u32, j: usize| j as u32 * m + i % m;
    let uv = |i: u32, j: usize| [i as f32, j as f32];
    let (mut faces, mut layout) = (Vec::new(), Vec::new());
    if let Some(y) = apex {
        let a = positions.len() as u32;
        positions.push([0.0, y as f32, 0.0]);
        for i in 0..m {
            faces.push([a, at(i + 1, 0), at(i, 0), NO_VERTEX]);
            layout.push([[0.0, -1.0], uv(i + 1, 0), uv(i, 0), [0.0; 2]]);
        }
    }
    for j in 0..rings.len() - 1 {
        for i in 0..m {
            faces.push([at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
            layout.push([uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1)]);
        }
    }
    let mesh = PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    };
    (mesh, layout)
}

/// The mesh as one chart cut along `layout`'s seams, flattened: angle and L2 stretch 1 within
/// 1e-3, every edge its surface length within 1e-3 of it, nothing flipped.
fn assert_lies_flat(mesh: &PlainMesh, layout: &[Corners], what: &str) {
    let one = Charts {
        chart: vec![0; mesh.faces.len()],
        count: 1,
        seams: seams(&mesh.faces, layout),
        added_seams: 0,
    };
    let uvs = laid_out(mesh, &one, None);
    let all = Layout {
        surface: surface(mesh),
        uvs: &uvs,
        chart: &one.chart,
    };
    let s = stretch(&all);
    assert!(
        (s.l2 - 1.0).abs() <= 1e-3 && (s.angle - 1.0).abs() <= 1e-3,
        "{what}: L2 {} and angle {}",
        s.l2,
        s.angle
    );
    let f = flips(&all);
    assert_eq!((f.flipped, f.mirrored), (0, 0), "{what}: nothing flipped");
    for (face, uv) in mesh.faces.iter().zip(&uvs) {
        let n = corners(face).len();
        for k in 0..n {
            let (p, q) = (
                mesh.positions[face[k] as usize],
                mesh.positions[face[(k + 1) % n] as usize],
            );
            let d3 = (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum::<f32>();
            let (a, b) = (uv[k], uv[(k + 1) % n]);
            let d2 = (0..2).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>();
            let (d3, d2) = (d3.sqrt(), d2.sqrt());
            assert!(
                (d2 - d3).abs() <= 1e-3 * d3,
                "{what}: an edge {d3} long lies {d2} long"
            );
        }
    }
}

#[test]
fn a_flat_chart_comes_back_as_it_lies() {
    let mut mesh = grid(8);
    for (v, p) in mesh.positions.iter_mut().enumerate() {
        if p[0] > 0.0 && p[0] < 1.0 && p[2] > 0.0 && p[2] < 1.0 {
            p[0] += 0.015 * ((v * 7 % 5) as f32 - 2.0);
            p[2] += 0.015 * ((v * 3 % 5) as f32 - 2.0);
        }
    }
    let whole = vec![[[0.0; 2]; 4]; mesh.faces.len()];
    assert_lies_flat(&mesh, &whole, "a flat grid");
}

#[test]
fn a_cylinder_band_cut_open_lies_flat() {
    let rings: Vec<[f64; 2]> = (0..=6).map(|j| [0.5, 1.0 - f64::from(j) / 6.0]).collect();
    let (mesh, layout) = turned(24, &rings, None);
    assert_lies_flat(&mesh, &layout, "a cylinder band");
}

#[test]
fn a_cone_cut_open_lies_flat() {
    let rings: Vec<[f64; 2]> = (1..=6)
        .map(|j| [0.6 * f64::from(j) / 6.0, 1.0 - f64::from(j) / 6.0])
        .collect();
    let (mesh, layout) = turned(24, &rings, Some(1.0));
    assert_lies_flat(&mesh, &layout, "a cone");
}

#[test]
fn no_triangle_flips_on_the_cutter_s_charts_of_the_bench_shapes() {
    for (name, mesh) in shapes() {
        let surface = surface(&mesh);
        let flattener =
            Flattener::new(surface, None).unwrap_or_else(|r| panic!("{name} refused: {r}"));
        let checked = cut_with(surface, &Steer::default(), |chart, _| {
            flattener.chart(chart).uvs
        })
        .unwrap_or_else(|r| panic!("{name} refused: {r}"));
        let projected = charts(&mesh, &Steer::default());
        for (route, c) in [("the projection", projected), ("LSCM", checked)] {
            let meshes = c.meshes(surface);
            let flats = flattener.charts(&meshes);
            let mut uvs = vec![[[0.0; 2]; 4]; mesh.faces.len()];
            for (chart, flat) in meshes.iter().zip(&flats) {
                chart.write(&flat.uvs, &mut uvs);
                // Its long principal axis on u: the spread along u at least the spread along v.
                let n = flat.uvs.len() as f64;
                let mean =
                    [0, 1].map(|i| flat.uvs.iter().map(|p| f64::from(p[i])).sum::<f64>() / n);
                let spread = |i: usize| -> f64 {
                    (flat.uvs.iter())
                        .map(|p| (f64::from(p[i]) - mean[i]) * (f64::from(p[i]) - mean[i]))
                        .sum()
                };
                assert!(
                    spread(1) <= spread(0) * (1.0 + 1e-4),
                    "{name}, checked on {route}: a chart lies along u ({} against {})",
                    spread(0),
                    spread(1)
                );
            }
            let f = flips(&Layout {
                surface,
                uvs: &uvs,
                chart: &c.chart,
            });
            assert_eq!(
                (f.flipped, f.mirrored),
                (0, 0),
                "{name}, checked on {route}: flipped triangles and mirrored charts"
            );
        }
    }
}

#[test]
fn twins_mirror_exactly_and_a_chart_across_the_plane_mirrors_onto_itself() {
    // The plane runs along edges at 1,944 faces and through faces at 2,166; the protected front
    // of the face is one chart across it either way.
    for faces in [1944, 2166] {
        let mesh = head(faces);
        let hint: Vec<f32> = (mesh.positions.iter())
            .map(|p| if p[2] > 0.7 { -1.0 } else { 0.0 })
            .collect();
        let symmetry = in_x(&mesh);
        let steer = Steer {
            hint: Some(&hint),
            symmetry: Some(symmetry),
            ..Steer::default()
        };
        let c = charts(&mesh, &steer);
        let uvs = laid_out(&mesh, &c, Some(symmetry));
        let f = flips(&Layout {
            surface: surface(&mesh),
            uvs: &uvs,
            chart: &c.chart,
        });
        assert_eq!((f.flipped, f.mirrored), (0, 0), "{faces}: nothing flipped");
        let partner = partners(&mesh);
        let key = |c: &[u32]| {
            let mut k = c.to_vec();
            k.sort_unstable();
            k
        };
        let face_at: BTreeMap<Vec<u32>, usize> = (mesh.faces.iter().enumerate())
            .map(|(f, face)| (key(corners(face)), f))
            .collect();
        let bits = |p: [f32; 2]| p.map(|x| (x + 0.0).to_bits());
        let mut across = 0;
        for (f, face) in mesh.faces.iter().enumerate() {
            let image: Vec<u32> = (corners(face).iter())
                .map(|&v| partner[v as usize])
                .collect();
            let g = face_at[&key(&image)];
            across += usize::from(c.chart[f] == c.chart[g]);
            let mirror = corners(&mesh.faces[g]);
            for (k, &v) in image.iter().enumerate() {
                let j = (mirror.iter().position(|&w| w == v))
                    .unwrap_or_else(|| panic!("face {g} holds point {v}"));
                let [u, w] = uvs[g][j];
                assert_eq!(
                    bits(uvs[f][k]),
                    bits([-u, w]),
                    "{faces}: face {f}'s corner {k} is face {g}'s corner {j} mirrored"
                );
            }
        }
        assert!(across > 0, "{faces}: a chart crosses the plane");
    }
}

#[test]
fn the_flattener_gives_the_same_bits_at_one_and_twenty_four_threads() {
    let mesh = head(2000);
    for symmetry in [None, Some(in_x(&mesh))] {
        let steer = Steer {
            symmetry,
            ..Steer::default()
        };
        let c = charts(&mesh, &steer);
        let at = |threads| gs_par::with_threads(threads, || laid_out(&mesh, &c, symmetry));
        let bits = |uvs: Vec<Corners>| -> Vec<[[u32; 2]; 4]> {
            uvs.iter().map(|c| c.map(|p| p.map(f32::to_bits))).collect()
        };
        assert_eq!(
            bits(at(1)),
            bits(at(24)),
            "the same bits, symmetry {symmetry:?}"
        );
    }
}
