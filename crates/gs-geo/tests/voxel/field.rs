//! The signed distance field against a slow brute-force reference (differential test), the
//! work counters against the amounts their docs promise (work-counter contracts), and the grid
//! against its documented layout, its outer layer always outside.

use std::collections::BTreeSet;

use gs_geo::voxel::{
    CellSize, Distances, GROWTH, Grid, Kept, PAD, PIECE, REACH, RemeshError, Settings, SignMethod,
    remesh, settled_inside, signed_distance,
};
use gs_protocol::PlainMesh;

use crate::common::*;
use crate::shape::{assert_row_route, settings, tetra_with_fin};

fn field(mesh: &PlainMesh, resolution: u32) -> Distances {
    signed_distance(mesh, CellSize::Resolution(resolution))
        .unwrap_or_else(|e| panic!("signed distance failed: {e}"))
}

fn world(grid: &Grid, at: [u32; 3]) -> P {
    std::array::from_fn(|a| grid.origin[a] + grid.voxel * f64::from(at[a]))
}

/// The mesh's triangles, then the caps a remesh put over its holes.
fn capped(mesh: &PlainMesh, caps: &[[u32; 3]]) -> Vec<[P; 3]> {
    let mut tris = triangles(mesh);
    tris.extend(
        caps.iter()
            .map(|t| t.map(|v| wide(mesh.positions[v as usize]))),
    );
    tris
}

/// Checks every sample against brute force over all triangles, caps included: exact distance
/// within reach, the nearest triangle, the sign from the exact winding number, and that every
/// sample within one voxel of the surface was measured.
fn check(what: &str, mesh: &PlainMesh, resolution: u32) -> Distances {
    let d = field(mesh, resolution);
    let tris = capped(mesh, &d.caps);
    let h = d.grid.voxel;
    let reach = f64::from(REACH);
    let exact = |p: P| {
        tris.iter()
            .map(|t| triangle_distance(p, t))
            .fold(f64::MAX, f64::min)
            / h
    };
    let mut measured = BTreeSet::new();
    for s in &d.samples {
        let p = world(&d.grid, s.at);
        let true_distance = exact(p);
        let size = f64::from(s.value.abs());
        if let Some(t) = s.near {
            measured.insert(s.at);
            let own = triangle_distance(p, &tris[t as usize]) / h;
            assert!(
                (size - own.min(2.0)).abs() <= 1e-4,
                "{what} {:?}: value {size} against its triangle's {own}",
                s.at
            );
            if true_distance <= reach {
                assert!(
                    own - true_distance <= 1e-4,
                    "{what} {:?}: triangle {t} is not the nearest",
                    s.at
                );
            }
        } else {
            assert!(
                true_distance >= reach - 1e-4,
                "{what} {:?}: {true_distance} voxels away but not measured",
                s.at
            );
            let two = s.value.abs().to_bits() == 2.0f32.to_bits();
            assert!(two, "{what} {:?}: unmeasured samples read 2", s.at);
        }
        if true_distance >= 1e-3 {
            let w = winding(&tris, p);
            let inside = s.value.is_sign_negative();
            // Rows count any winding but zero as inside; the generalized winding number needs
            // at least a half.
            let level = if d.sign == SignMethod::GeneralizedWinding {
                w
            } else {
                w.abs()
            };
            let clear = (level - 0.5).abs() >= 0.05;
            assert!(
                !clear || inside == (level >= 0.5),
                "{what} {:?}: winding {w} but inside {inside}",
                s.at
            );
        }
    }
    let samples = d.grid.samples();
    for i in 0..samples[0] {
        for j in 0..samples[1] {
            for k in 0..samples[2] {
                let at = [i, j, k];
                if exact(world(&d.grid, at)) <= 1.0 {
                    assert!(
                        measured.contains(&at),
                        "{what} {at:?}: within a voxel but not measured"
                    );
                }
            }
        }
    }
    // And no brick is allocated without a sample in reach: the rest stay uniform.
    for brick in d.samples.chunks(512) {
        let close = brick.iter().any(|s| f64::from(s.value.abs()) <= reach);
        assert!(
            close,
            "{what}: brick at {:?} has no sample in reach",
            brick[0].at
        );
    }
    d
}

/// Triangles' corners in grid units, rounded to `f32` the way the docs of `Work` describe.
fn grid_corners(grid: &Grid, tris: &[[P; 3]]) -> Vec<[[f32; 3]; 3]> {
    tris.iter()
        .map(|t| t.map(|p| std::array::from_fn(|a| ((p[a] - grid.origin[a]) / grid.voxel) as f32)))
        .collect()
}

/// `Work::distance_evaluations` and `Work::binned` as documented: the samples of the reach box
/// of every piece of every non-flat triangle, and the bricks holding them.
fn expected_band(grid: &Grid, tris: &[[P; 3]]) -> (u64, u64) {
    let mut total = (0, 0);
    for t in grid_corners(grid, tris) {
        let e = |i: usize, a: usize| t[i][a] - t[0][a];
        let n = [
            e(1, 1) * e(2, 2) - e(1, 2) * e(2, 1),
            e(1, 2) * e(2, 0) - e(1, 0) * e(2, 2),
            e(1, 0) * e(2, 1) - e(1, 1) * e(2, 0),
        ];
        if n[0] * n[0] + n[1] * n[1] + n[2] * n[2] <= 1e-12 {
            continue;
        }
        let d2 = |i: usize, j: usize| {
            let d: [f32; 3] = std::array::from_fn(|a| t[i][a] - t[j][a]);
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
        };
        let longest = d2(0, 1).max(d2(1, 2)).max(d2(2, 0)).sqrt();
        let cuts = (longest / PIECE).ceil().max(1.0) as u32;
        // The same arithmetic as the docs' formula, factors written the other way round.
        let point = |i: u32, j: u32| -> [f32; 3] {
            let steps = cuts as f32;
            let along = [i as f32 / steps, j as f32 / steps];
            let edge = |from: usize, to: usize, a: usize| t[to][a] - t[from][a];
            [0, 1, 2].map(|a| t[0][a] + edge(0, 1, a) * along[0] + edge(1, 2, a) * along[1])
        };
        // Row i of the cut holds 2i + 1 pieces, pointing up and down in turn.
        let mut add = |piece: [[f32; 3]; 3]| {
            let (samples, bricks) = reach_box(&piece);
            total = (total.0 + samples, total.1 + bricks);
        };
        for i in 0..cuts {
            for m in 0..=i {
                add([point(i, m), point(i + 1, m), point(i + 1, m + 1)]);
                if m < i {
                    add([point(i, m), point(i + 1, m + 1), point(i, m + 1)]);
                }
            }
        }
    }
    total
}

/// The samples within `REACH` of a piece's bounding box, and the 8 x 8 x 8 bricks they lie in.
fn reach_box(piece: &[[f32; 3]; 3]) -> (u64, u64) {
    let (mut samples, mut bricks) = (1, 1);
    for a in 0..3 {
        let lo = piece.iter().map(|p| p[a]).fold(f32::MAX, f32::min);
        let hi = piece.iter().map(|p| p[a]).fold(f32::MIN, f32::max);
        let (first, last) = ((lo - REACH).ceil() as u64, (hi + REACH).floor() as u64);
        samples *= last - first + 1;
        bricks *= last / 8 - first / 8 + 1;
    }
    (samples, bricks)
}

/// `Work::crossing_tests` as documented: the whole (y, z) positions in the bounding box of
/// every triangle's shadow that has area, corners snapped to 1/65536 voxel.
fn expected_crossing_tests(grid: &Grid, tris: &[[P; 3]]) -> u64 {
    let mut total = 0;
    for t in grid_corners(grid, tris) {
        let s = t.map(|p| p.map(|c| (f64::from(c) * 65536.0).round() as i64));
        let area =
            (s[1][1] - s[0][1]) * (s[2][2] - s[0][2]) - (s[1][2] - s[0][2]) * (s[2][1] - s[0][1]);
        if area == 0 {
            continue;
        }
        let mut count = 1;
        for a in 1..3 {
            let lo = s.iter().map(|p| p[a]).min().unwrap_or(0);
            let hi = s.iter().map(|p| p[a]).max().unwrap_or(0);
            count *= (hi.div_euclid(65536) - (lo + 65535).div_euclid(65536) + 1) as u64;
        }
        total += count;
    }
    total
}

#[test]
fn the_field_matches_brute_force_on_small_inputs() {
    let tilted = {
        let mut m = cuboid([-0.8, -0.5, -0.3], [0.8, 0.5, 0.3], 1);
        for p in &mut m.positions {
            let [x, y, z] = wide(*p);
            *p = [
                (0.8 * x - 0.6 * y) as f32,
                (0.6 * x + 0.8 * y) as f32,
                z as f32,
            ];
        }
        m
    };
    let open = without(&sphere([0.0; 3], 1.0, 3), |c| c.iter().all(|p| p[1] >= 0.3));
    let cases = [
        ("fin", tetra_with_fin(), 10, SignMethod::Winding),
        (
            "aligned cube",
            cuboid([-1.0; 3], [1.0; 3], 1),
            8,
            SignMethod::Winding,
        ),
        ("tilted box", tilted, 9, SignMethod::Winding),
        (
            "overlap",
            merge(&sphere([0.0; 3], 1.0, 2), &sphere([0.7, 0.2, 0.1], 0.7, 2)),
            10,
            SignMethod::Winding,
        ),
        ("open", open.clone(), 10, SignMethod::CappedWinding),
        (
            "open, a face doubled",
            doubled(&open),
            10,
            SignMethod::GeneralizedWinding,
        ),
    ];
    for (what, mesh, resolution, method) in cases {
        let d = check(what, &mesh, resolution);
        assert_eq!(d.sign, method, "{what}: sign method");
        assert_eq!(
            d.caps.is_empty(),
            method != SignMethod::CappedWinding,
            "{what}: caps only over holes"
        );
    }
}

/// The bricks inside the grid, not allocated, among the 26 around an allocated brick.
fn uniform_neighbours(d: &Distances) -> u64 {
    let allocated: BTreeSet<[u32; 3]> = d.samples.iter().map(|s| s.at.map(|c| c / 8)).collect();
    let mut around = BTreeSet::new();
    for b in &allocated {
        for k in 0..27 {
            let step = [k % 3, k / 3 % 3, k / 9];
            let n: Option<Vec<u32>> = (0..3)
                .map(|a| {
                    (b[a] + step[a])
                        .checked_sub(1)
                        .filter(|&c| c < d.grid.bricks[a])
                })
                .collect();
            if let Some(n) = n.filter(|n| !allocated.contains(&[n[0], n[1], n[2]])) {
                around.insert(n);
            }
        }
    }
    around.len() as u64
}

#[test]
fn work_counters_count_exactly_what_their_docs_say() {
    let open = without(&sphere([0.0; 3], 1.0, 8), |c| c.iter().all(|p| p[1] >= 0.5));
    for (what, mesh, resolution, holes) in [
        ("fin", tetra_with_fin(), 12, 0),
        ("aligned cube", cuboid([-1.0; 3], [1.0; 3], 2), 8, 0),
        ("sphere", sphere([0.1, 0.2, 0.3], 1.0, 6), 20, 0),
        // More than one 4096-triangle task, so every task boundary counts.
        ("fine sphere", sphere([0.1, 0.2, 0.3], 1.0, 28), 20, 0),
        // Big enough to have bricks wholly inside, which take one uniform sign.
        ("large sphere", sphere([0.1, 0.2, 0.3], 1.0, 8), 48, 0),
        // Capped: the caps count as triangles.
        ("open sphere", open.clone(), 16, 1),
    ] {
        let out =
            remesh(&mesh, &settings(resolution, false)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let w = &out.work;
        let d = field(&mesh, resolution);
        let tris = capped(&mesh, &d.caps);
        assert_eq!(w.holes, holes, "{what}: holes capped");
        assert_eq!(
            (w.distance_evaluations, w.binned),
            expected_band(&out.grid, &tris),
            "{what}: distances and bricks binned"
        );
        assert_eq!(
            w.crossing_tests,
            expected_crossing_tests(&out.grid, &tris),
            "{what}: crossings"
        );
        assert_eq!(
            w.winding_terms, 0,
            "{what}: no winding numbers for closed or capped input"
        );
        assert_eq!(
            (w.flips, w.film_bricks),
            (0, 0),
            "{what}: nothing to repair or film"
        );
        assert_eq!(
            w.sign_queries,
            512 * w.bricks,
            "{what}: one sign per allocated sample"
        );
        assert_eq!(
            d.work.uniform_queries,
            uniform_neighbours(&d),
            "{what}: one sign per unallocated brick next to an allocated one"
        );
        assert!(
            w.cells_checked >= 512 * w.bricks && w.cells_checked <= 8 * 512 * w.bricks,
            "{what}: cells: {w:?}"
        );
        assert_eq!(w.cells_checked % 512, 0, "{what}: whole bricks of cells");
    }
    // An edge used twice the same way: the generalized winding number, no rows.
    let messy = doubled(&open);
    let out = remesh(&messy, &settings(16, false)).unwrap_or_else(|e| panic!("messy: {e}"));
    assert_eq!(
        (out.work.crossing_tests, out.work.holes),
        (0, 0),
        "no rows or caps for messy input"
    );
    let triangles = 2 * messy.faces.len() as u64;
    let per_query = out.work.winding_terms / out.work.sign_queries;
    assert!(
        per_query >= 1 && per_query <= triangles / 2,
        "{per_query} terms per query, {triangles} triangles"
    );
}

#[test]
fn the_sparse_field_grows_with_the_surface_not_the_volume() {
    let mesh = sphere([0.1, 0.2, 0.3], 1.0, 12);
    assert_row_route(&mesh);
    let runs: Vec<Distances> = [16, 32, 64].into_iter().map(|r| field(&mesh, r)).collect();
    for pair in runs.windows(2) {
        let (a, b) = (&pair[0].work, &pair[1].work);
        let bricks = b.bricks as f64 / a.bricks as f64;
        let distances = b.distance_evaluations as f64 / a.distance_evaluations as f64;
        assert!(
            (2.0..=5.5).contains(&bricks),
            "bricks grew {bricks}x for 2x resolution"
        );
        assert!(
            (2.0..=5.5).contains(&distances),
            "distances grew {distances}x"
        );
    }
    // Bricks stay within a small multiple of the surface area over a brick face (64 square
    // voxels), at every resolution; a dense grid would grow with the volume instead.
    let area = surface_area(&mesh);
    for d in &runs {
        let faces = area / (d.grid.voxel * d.grid.voxel) / 64.0;
        let bricks = d.work.bricks as f64;
        assert!(
            bricks <= 3.0 * faces,
            "{bricks} bricks for {faces} brick faces of surface"
        );
    }
    let fine = &runs[2];
    let allocated = 512 * fine.work.bricks;
    assert_eq!(
        fine.samples.len() as u64,
        allocated,
        "every allocated sample is listed"
    );
}

#[test]
fn the_grid_has_its_documented_layout() {
    let mesh = cuboid([-0.4, 0.25, 1.0], [1.6, 0.75, 1.5], 1);
    // The bounding box as the f32 positions store it.
    let lo = [-0.4_f32, 0.25, 1.0].map(f64::from);
    let hi = [1.6_f32, 0.75, 1.5].map(f64::from);
    // Resolution 20 over the longest side, x.
    for (cell, voxel) in [
        (CellSize::Resolution(20), (hi[0] - lo[0]) / 20.0),
        (CellSize::Size(0.15), f64::from(0.15f32)),
    ] {
        let d = signed_distance(&mesh, cell).unwrap_or_else(|e| panic!("{e}"));
        let origin: P = std::array::from_fn(|a| lo[a] - PAD * voxel);
        let bricks: [u32; 3] = std::array::from_fn(|a| {
            let top = (hi[a] - origin[a]) / voxel + PAD;
            (top.floor() as u32 + 1).div_ceil(8)
        });
        assert!(
            (d.grid.voxel - voxel).abs() <= 1e-12,
            "{cell:?}: voxel {}",
            d.grid.voxel
        );
        for (got, want) in d.grid.origin.iter().zip(origin) {
            assert!(
                (got - want).abs() <= 1e-12,
                "{cell:?}: origin {got} against {want}"
            );
        }
        assert_eq!(d.grid.bricks, bricks, "{cell:?}: bricks");
        assert_eq!(d.grid.samples(), bricks.map(|b| 8 * b), "{cell:?}: samples");
    }
}

#[test]
fn a_kept_cell_size_holds_until_the_longest_side_outgrows_its_limit() {
    // Rods thinner than a cell, so even 1500 cells along them stay cheap.
    let rod = |length: f64| cuboid([0.0; 3], [length, 0.002, 0.002], 1);
    let kept = |mesh: &PlainMesh, cell: CellSize| {
        let settings = Settings {
            cell,
            project: false,
            blur: 0.0,
        };
        let out = remesh(mesh, &settings).unwrap_or_else(|e| panic!("{cell:?}: {e}"));
        assert_eq!(
            out.grid.voxel, out.kept.voxel,
            "{cell:?}: keeps the size it used"
        );
        out.kept
    };
    // Setting a size keeps it, with room to grow to 1024 cells, or the cells it was set at.
    for (cell, voxel, limit) in [
        (CellSize::Resolution(100), 1.0 / 100.0, GROWTH),
        (CellSize::Resolution(1500), 1.0 / 1500.0, 1500),
        (CellSize::Size(0.004), f64::from(0.004_f32), GROWTH),
        // 1 / 0.0008 rounded to f32 is just over 1250 cells.
        (CellSize::Size(0.0008), f64::from(0.0008_f32), 1251),
    ] {
        assert_eq!(kept(&rod(1.0), cell), Kept { voxel, limit }, "{cell:?}");
    }
    let at = |voxel, limit| CellSize::Kept(Kept { voxel, limit });
    // A kept size holds while the longest side fits in its limit...
    let fits = Kept {
        voxel: 0.01,
        limit: GROWTH,
    };
    assert_eq!(kept(&rod(2.0), at(0.01, GROWTH)), fits, "grown, still in");
    let fine = Kept {
        voxel: 1.0 / 1500.0,
        limit: 1500,
    };
    assert_eq!(
        kept(&rod(1.0), at(fine.voxel, 1500)),
        fine,
        "set finer than 1024"
    );
    // ...and past it the grid is refit to exactly the limit, which stays.
    let refit = Kept {
        voxel: 1.5 / 1024.0,
        limit: GROWTH,
    };
    assert_eq!(kept(&rod(1.5), at(0.001, GROWTH)), refit, "outgrown");
    // The cap on cells holds for kept sizes too.
    let err = signed_distance(&rod(3.0), at(0.001, 4096)).err();
    assert_eq!(err, Some(RemeshError::TooManyCells), "3000 cells");
}

/// The grid's outer layer stays outside where nothing but the edge-brick rule can reach it
/// (review 0008, T2): a winding inside everywhere, from a box around the whole grid, and one
/// allocated brick in the middle of 3 x 3 x 3. Its faces are all inside, so the film rule never
/// allocates the 26 edge bricks around it; read as uniformly inside, their outer samples would
/// be inside.
#[test]
fn an_edge_brick_is_never_uniformly_inside() {
    let grid = Grid {
        origin: [0.0; 3],
        voxel: 1.0,
        bricks: [3; 3],
    };
    let n = grid.samples().map(|c| c as usize);
    // A box around the whole grid, in grid units, its faces wound outward.
    let corner = |k: usize| [k % 2, k / 2 % 2, k / 4].map(|c| if c == 1 { 25.0 } else { -1.0 });
    let quads = [
        [0, 4, 6, 2],
        [1, 3, 7, 5],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 2, 3, 1],
        [4, 5, 7, 6],
    ];
    let tris = (quads.iter())
        .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
        .map(|t| t.map(corner))
        .collect();
    let inside = settled_inside(grid, tris, [1; 3]);
    assert_eq!(inside.len(), n[0] * n[1] * n[2]);
    for (i, inside) in inside.into_iter().enumerate() {
        let p = [i % n[0], i / n[0] % n[1], i / (n[0] * n[1])];
        let outer = (0..3).any(|a| p[a] == 0 || p[a] + 1 == n[a]);
        assert_eq!(inside, !outer, "sample {p:?}, outer {outer}");
    }
}
