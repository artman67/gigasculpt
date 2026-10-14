//! The packer (decision 0061 sections 1, 6 to 8 and 10, step UV4) on the bench shapes' charts,
//! each flattened on its mean plane by `Chart::project` as the cutter's check reads them.

use std::collections::BTreeMap;

use gs_geo::uv::metrics::{Layout, cover, density, flips};
use gs_geo::uv::{Charts, Corners, Crowded, Mirror, Pack, Steer, Twins, pack};
use gs_protocol::{NO_VERTEX, PlainMesh, arity, corners, face_triangles};
use gs_testkit::bench_shapes::{head, sphere};

use crate::symmetry::{in_x, partners};
use crate::{charts, grid, shapes, surface};

/// The cutter's charts of `mesh` under `steer`, each on its mean plane.
fn flat(mesh: &PlainMesh, steer: &Steer) -> (Charts, Vec<Corners>) {
    let c = charts(mesh, steer);
    let mut uvs = vec![[[0.0; 2]; 4]; mesh.faces.len()];
    for chart in c.meshes(surface(mesh)) {
        chart.write(&chart.project(surface(mesh)), &mut uvs);
    }
    (c, uvs)
}

fn layout<'a>(mesh: &'a PlainMesh, uvs: &'a [Corners], chart: &'a [u32]) -> Layout<'a> {
    Layout {
        surface: surface(mesh),
        uvs,
        chart,
    }
}

/// The packed layout, which must not be refused.
fn packed(mesh: &PlainMesh, chart: &[u32], uvs: &[Corners], options: &Pack) -> Vec<Corners> {
    pack(&layout(mesh, uvs, chart), options).unwrap_or_else(|crowded| panic!("{crowded}"))
}

/// Asserts no overlap and at least `padding_px` between charts on a `map_size` raster.
fn assert_spaced(name: &str, layout: &Layout, map_size: u32, padding_px: u32) {
    let c = cover(layout, map_size);
    assert_eq!(c.overlap, 0, "{name}: no pixel covered twice");
    assert!(
        c.gap.is_some_and(|gap| gap >= padding_px),
        "{name}: the narrowest gap {:?} is at least {padding_px} px at {map_size}",
        c.gap
    );
}

#[test]
fn packed_charts_never_overlap_and_keep_the_padding_between_them() {
    for (name, mesh) in shapes() {
        let (c, uvs) = flat(&mesh, &Steer::default());
        for (map_size, padding_px) in [(2048, 8), (512, 3)] {
            let options = Pack {
                map_size,
                padding_px,
                ..Pack::default()
            };
            let out = packed(&mesh, &c.chart, &uvs, &options);
            assert_spaced(name, &layout(&mesh, &out, &c.chart), map_size, padding_px);
        }
    }
}

#[test]
fn every_uv_is_inside_the_square_and_every_chart_keeps_its_face_up() {
    for (name, mesh) in shapes() {
        let (c, uvs) = flat(&mesh, &Steer::default());
        let out = packed(&mesh, &c.chart, &uvs, &Pack::default());
        for (f, face) in mesh.faces.iter().enumerate() {
            for uv in &out[f][..arity(*face)] {
                assert!(
                    uv.iter().all(|x| (0.0..=1.0).contains(x)),
                    "{name}: face {f}'s UV {uv:?} is inside 0 to 1"
                );
            }
        }
        let (before, after) = (
            flips(&layout(&mesh, &uvs, &c.chart)),
            flips(&layout(&mesh, &out, &c.chart)),
        );
        assert_eq!(after, before, "{name}: turned, never mirrored");
        assert_eq!(after.mirrored, 0, "{name}: no chart mirrored");
        assert!(
            out != uvs,
            "{name}: the charts are moved into the square, not left where they lay"
        );
    }
}

#[test]
fn every_chart_gets_one_texel_density_without_hints() {
    for (name, mesh) in shapes() {
        let (c, mut uvs) = flat(&mesh, &Steer::default());
        // Each chart flattened at its own size, as a flattener may leave them.
        for (f, corners) in uvs.iter_mut().enumerate() {
            let s = 1.0 + (c.chart[f] % 4) as f32;
            *corners = corners.map(|uv| uv.map(|x| x * s));
        }
        let before = density(&layout(&mesh, &uvs, &c.chart));
        assert!(
            before.p95 / before.p5 > 2.0,
            "{name}: the flat charts start at different densities"
        );
        let out = packed(&mesh, &c.chart, &uvs, &Pack::default());
        let d = density(&layout(&mesh, &out, &c.chart));
        assert!(
            d.p95 / d.p5 <= 1.1,
            "{name}: density p95 over p5 is {}, at most 1.1",
            d.p95 / d.p5
        );
    }
}

/// Each chart's surface area and UV area, each quad as its two triangles by `quad_diagonal`.
fn areas(mesh: &PlainMesh, uvs: &[Corners], chart: &[u32], count: u32) -> Vec<(f64, f64)> {
    let mut out = vec![(0.0, 0.0); count as usize];
    for (f, &face) in mesh.faces.iter().enumerate() {
        let p = |k: usize| mesh.positions[face[k] as usize].map(f64::from);
        for t in face_triangles(face, |k| mesh.positions[face[k] as usize]) {
            let [a, b, c] = t.map(p);
            let (e, g) = (
                [0, 1, 2].map(|i| b[i] - a[i]),
                [0, 1, 2].map(|i| c[i] - a[i]),
            );
            let n = [
                e[1] * g[2] - e[2] * g[1],
                e[2] * g[0] - e[0] * g[2],
                e[0] * g[1] - e[1] * g[0],
            ];
            let [q, r, s] = t.map(|k| uvs[f][k].map(f64::from));
            let uv = (r[0] - q[0]) * (s[1] - q[1]) - (s[0] - q[0]) * (r[1] - q[1]);
            let at = &mut out[chart[f] as usize];
            at.0 += (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0;
            at.1 += uv.abs() / 2.0;
        }
    }
    out
}

#[test]
fn a_chart_painted_at_density_one_gets_four_times_the_uv_area() {
    let mesh = sphere(2000);
    let (c, uvs) = flat(&mesh, &Steer::default());
    let painted = c.chart[0];
    let mut density = vec![0.0; mesh.positions.len()];
    for (f, face) in mesh.faces.iter().enumerate() {
        if c.chart[f] == painted {
            for &v in corners(face) {
                density[v as usize] = 1.0;
            }
        }
    }
    let touched = |chart: u32| {
        (mesh.faces.iter().enumerate()).any(|(f, face)| {
            c.chart[f] == chart && corners(face).iter().any(|&v| density[v as usize] > 0.0)
        })
    };
    let far = (0..c.count)
        .find(|&k| !touched(k))
        .expect("a chart away from the paint");
    let options = Pack {
        density: Some(&density),
        ..Pack::default()
    };
    let out = packed(&mesh, &c.chart, &uvs, &options);
    let a = areas(&mesh, &out, &c.chart, c.count);
    let ratio = |k: u32| a[k as usize].1 / a[k as usize].0;
    let times = ratio(painted) / ratio(far);
    assert!(
        (times - 4.0).abs() < 1e-4,
        "the painted chart has {times} times the UV area per surface area, not 4"
    );
}

/// The head cut under symmetry in x and flattened.
struct Mirrored {
    mesh: PlainMesh,
    c: Charts,
    uvs: Vec<Corners>,
    /// Each chart's twin.
    twin: Vec<u32>,
    /// Each face corner's mirror (face, corner).
    image: Vec<[(usize, usize); 4]>,
}

/// The head with faces across its mirror plane, its charts cut under symmetry in x and flattened
/// as section 7 has the flattener do it: each twin, the higher-numbered chart of a pair, takes
/// its partner's UVs with u negated.
fn mirrored_head() -> Mirrored {
    let mesh = head(2166);
    let steer = Steer {
        symmetry: Some(in_x(&mesh)),
        ..Steer::default()
    };
    let (c, mut uvs) = flat(&mesh, &steer);
    let partner = partners(&mesh);
    let mut by_points: BTreeMap<Vec<u32>, usize> = BTreeMap::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let mut key = corners(face).to_vec();
        key.sort_unstable();
        by_points.insert(key, f);
    }
    let mut twin: Vec<u32> = (0..c.count).collect();
    let mut image = vec![[(0, 0); 4]; mesh.faces.len()];
    for (f, face) in mesh.faces.iter().enumerate() {
        let mut key: Vec<u32> = corners(face).iter().map(|&v| partner[v as usize]).collect();
        key.sort_unstable();
        let g = by_points[&key];
        twin[c.chart[f] as usize] = c.chart[g];
        for (k, &v) in corners(face).iter().enumerate() {
            let j = corners(&mesh.faces[g])
                .iter()
                .position(|&w| w == partner[v as usize]);
            image[f][k] = (
                g,
                j.unwrap_or_else(|| panic!("face {g} holds face {f}'s corner {k}'s partner")),
            );
        }
    }
    for (f, face) in mesh.faces.iter().enumerate() {
        let (k_f, k_t) = (c.chart[f], twin[c.chart[f] as usize]);
        if k_t < k_f {
            for k in 0..arity(*face) {
                let (g, j) = image[f][k];
                uvs[f][k] = [-uvs[g][j][0], uvs[g][j][1]];
            }
        }
    }
    Mirrored {
        mesh,
        c,
        uvs,
        twin,
        image,
    }
}

#[test]
fn twins_sit_side_by_side_as_mirror_images() {
    let Mirrored {
        mesh,
        c,
        uvs,
        twin,
        image,
    } = mirrored_head();
    let pairs = (0..c.count).filter(|&k| twin[k as usize] > k).count();
    let alone = (0..c.count).filter(|&k| twin[k as usize] == k).count();
    assert!(
        pairs > 0 && alone > 0,
        "the head has twins and charts across the plane"
    );
    let options = Pack {
        twins: Some(Twins {
            twin: &twin,
            mirror: Mirror::SideBySide,
        }),
        ..Pack::default()
    };
    let out = packed(&mesh, &c.chart, &uvs, &options);
    assert_spaced("head", &layout(&mesh, &out, &c.chart), 2048, 8);
    let px = |x: f32| f64::from(x) * 2048.0;
    for k in (0..c.count).filter(|&k| twin[k as usize] > k) {
        // Each corner of chart k and its mirror corner in the twin, in pixels.
        let pairs: Vec<([f64; 2], [f64; 2])> = (mesh.faces.iter().enumerate())
            .filter(|&(f, _)| c.chart[f] == k)
            .flat_map(|(f, face)| (0..arity(*face)).map(move |n| (f, n)))
            .map(|(f, n)| {
                let (g, j) = image[f][n];
                (out[f][n].map(px), out[g][j].map(px))
            })
            .collect();
        // Mirrored across a line square to one axis: equal along the other, and a constant sum
        // along this one.
        let across = usize::from((pairs[0].0[0] - pairs[0].1[0]).abs() < 1e-3);
        let along = 1 - across;
        let sum = pairs[0].0[across] + pairs[0].1[across];
        for (a, b) in &pairs {
            assert!(
                (a[along] - b[along]).abs() < 0.02 && (a[across] + b[across] - sum).abs() < 0.02,
                "chart {k} and its twin mirror each other: {a:?} and {b:?}"
            );
        }
        // Side by side: the padding between them along the mirror's axis, and at most one pixel
        // more (a raster's width rounded up).
        let span = |side: usize| {
            let at = |p: &([f64; 2], [f64; 2])| if side == 0 { p.0[across] } else { p.1[across] };
            let lo = pairs.iter().map(at).fold(f64::MAX, f64::min);
            (lo, pairs.iter().map(at).fold(f64::MIN, f64::max))
        };
        let ((a0, a1), (b0, b1)) = (span(0), span(1));
        let gap = if a1 < b0 { b0 - a1 } else { a0 - b1 };
        assert!(
            (8.0 - 0.02..=9.0 + 0.02).contains(&gap),
            "chart {k} and its twin sit {gap} px apart, the padding of 8"
        );
    }
}

#[test]
fn a_twin_takes_its_partners_uvs_under_shared() {
    let Mirrored {
        mesh,
        c,
        uvs,
        twin,
        image,
    } = mirrored_head();
    let options = Pack {
        twins: Some(Twins {
            twin: &twin,
            mirror: Mirror::Shared,
        }),
        ..Pack::default()
    };
    let out = packed(&mesh, &c.chart, &uvs, &options);
    for (f, face) in mesh.faces.iter().enumerate() {
        if twin[c.chart[f] as usize] < c.chart[f] {
            for n in 0..arity(*face) {
                let (g, j) = image[f][n];
                assert_eq!(
                    out[f][n].map(f32::to_bits),
                    out[g][j].map(f32::to_bits),
                    "face {f}'s corner {n} holds its mirror corner's UV"
                );
            }
        }
    }
    // The charts that are not twins keep the padding between them; a shared twin is wound
    // against its faces, as a texture shared across a mirror must be.
    let kept: Vec<usize> = (0..mesh.faces.len())
        .filter(|&f| twin[c.chart[f] as usize] >= c.chart[f])
        .collect();
    let faces: Vec<[u32; 4]> = kept.iter().map(|&f| mesh.faces[f]).collect();
    let (uv_kept, chart_kept): (Vec<Corners>, Vec<u32>) =
        kept.iter().map(|&f| (out[f], c.chart[f])).unzip();
    let sub = PlainMesh {
        positions: mesh.positions.clone(),
        faces,
        attributes: BTreeMap::new(),
    };
    assert_spaced("head", &layout(&sub, &uv_kept, &chart_kept), 2048, 8);
    let twins = (0..c.count).filter(|&k| twin[k as usize] < k).count();
    assert_eq!(
        flips(&layout(&mesh, &out, &c.chart)).mirrored,
        twins,
        "every twin and only the twins are wound against their faces"
    );
}

#[test]
fn the_head_packs_the_same_at_one_and_twenty_four_threads() {
    let Mirrored {
        mesh, c, uvs, twin, ..
    } = mirrored_head();
    let density: Vec<f32> = (mesh.positions.iter())
        .map(|p| if p[1] > 0.3 { 0.5 } else { -0.25 })
        .collect();
    for mirror in [Mirror::SideBySide, Mirror::Shared] {
        let options = Pack {
            density: Some(&density),
            twins: Some(Twins {
                twin: &twin,
                mirror,
            }),
            ..Pack::default()
        };
        let at =
            |threads| gs_par::with_threads(threads, || packed(&mesh, &c.chart, &uvs, &options));
        let bits = |out: &[Corners]| -> Vec<[[u32; 2]; 4]> {
            out.iter()
                .map(|c| c.map(|uv| uv.map(f32::to_bits)))
                .collect()
        };
        let one = bits(&at(1));
        assert!(one != bits(&uvs), "{mirror:?}: the charts are packed");
        assert_eq!(one, bits(&at(24)), "{mirror:?}: the same bits");
    }
}

#[test]
fn charts_that_cannot_fit_with_their_padding_are_refused() {
    // 256 one-face charts, each at least 9 by 9 pixels with half of 8 px of padding around it,
    // cannot fit a 64 px square at any scale.
    let mesh = grid(16);
    let chart: Vec<u32> = (0..mesh.faces.len() as u32).collect();
    let uvs: Vec<Corners> = (mesh.faces.iter())
        .map(|face| {
            let mut c = [[0.0; 2]; 4];
            for (k, &v) in face.iter().enumerate().filter(|&(_, &v)| v != NO_VERTEX) {
                let p = mesh.positions[v as usize];
                c[k] = [p[0], p[2]];
            }
            c
        })
        .collect();
    let options = Pack {
        map_size: 64,
        ..Pack::default()
    };
    assert_eq!(
        pack(&layout(&mesh, &uvs, &chart), &options),
        Err(Crowded {
            charts: 256,
            map_size: 64,
            padding_px: 8,
        }),
        "refused, naming the charts, the size and the padding"
    );
}
