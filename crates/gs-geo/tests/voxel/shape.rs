//! What the remeshed surface must be, checked with the independent oracles in `common`:
//! closed and manifold, close to the input, the right volume, one shell for overlapping parts.

use gs_geo::voxel::{CellSize, Remeshed, Settings, SignMethod, remesh, signed_distance};
use gs_protocol::PlainMesh;
use gs_testkit::{Expect, validate};

use crate::common::*;

pub fn settings(resolution: u32, project: bool) -> Settings {
    Settings {
        cell: CellSize::Resolution(resolution),
        project,
        blur: 0.0,
    }
}

pub fn run(mesh: &PlainMesh, resolution: u32, project: bool) -> Remeshed {
    remesh(mesh, &settings(resolution, project)).unwrap_or_else(|e| panic!("remesh failed: {e}"))
}

/// Panics unless `mesh` takes the exact row-winding route, checked on a coarse grid. Tests of
/// big closed inputs call it first, so a broken closedness check fails them at once instead of
/// sending them down the far slower winding-number route at full size.
pub fn assert_row_route(mesh: &PlainMesh) {
    let coarse = signed_distance(mesh, CellSize::Resolution(4))
        .unwrap_or_else(|e| panic!("signed distance failed: {e}"));
    assert_eq!(
        coarse.sign,
        SignMethod::Winding,
        "closed input takes the row route"
    );
}

pub fn assert_closed_manifold(out: &Remeshed, what: &str) {
    let expect = Expect {
        manifold: true,
        closed: true,
        euler: None,
    };
    if let Err(defects) = validate(&out.mesh, expect) {
        panic!("{what}: {defects:?}");
    }
    assert!(
        out.mesh.faces.iter().all(|f| f[3] != u32::MAX),
        "{what}: all quads"
    );
}

/// True if the mesh is a closed manifold with Euler characteristic `chi`.
pub fn euler_is(mesh: &PlainMesh, chi: i64) -> bool {
    validate(mesh, Expect::closed(chi)).is_ok()
}

/// Two-sided Hausdorff distance in voxels.
pub fn voxels(a: &PlainMesh, out: &Remeshed) -> f64 {
    fast_hausdorff(a, &out.mesh, out.grid.voxel) / out.grid.voxel
}

#[test]
fn closed_input_gives_a_closed_manifold_of_the_same_topology() {
    let cases = [
        ("sphere", sphere([0.1, 0.2, 0.3], 1.0, 12), 20, 2),
        ("torus", torus(1.0, 0.35, 40, 20), 36, 0),
        ("aligned cube", cuboid([-1.0; 3], [1.0; 3], 4), 16, 2),
        ("tilted box", tilted_box(), 18, 2),
    ];
    for (what, mesh, resolution, chi) in cases {
        let out = run(&mesh, resolution, false);
        assert_eq!(out.sign, SignMethod::Winding, "{what}: closed input");
        assert_closed_manifold(&out, what);
        assert!(euler_is(&out.mesh, chi), "{what}: Euler characteristic");
        assert_eq!(components(&out.mesh), 1, "{what}: one piece");
        assert_eq!(out.work.flips, 0, "{what}: nothing to repair");
    }
}

/// A box turned off the grid axes, so its faces cut the samples at odd angles.
fn tilted_box() -> PlainMesh {
    let mut mesh = cuboid(HALF.map(|c| -c), HALF, 3);
    for p in &mut mesh.positions {
        *p = turn(wide(*p)).map(|c| c as f32);
    }
    mesh
}

/// Half the tilted box's size along each of its own axes.
const HALF: P = [0.8, 0.5, 0.3];

/// The tilted box's turn: a rotation about z, then one about x (both by asin 0.6).
fn turn([x, y, z]: P) -> P {
    let (s, c) = (0.6, 0.8);
    let (x, y) = (c * x - s * y, s * x + c * y);
    let (y, z) = (c * y - s * z, s * y + c * z);
    [x, y, z]
}

/// Checks that every vertex well inside a flat face (within a voxel of one face plane and at
/// least 3 voxels from every other) lies on that plane. The planes are `normal . p = offset`.
/// Surface Nets interpolates a linear distance exactly, so only the crossing margin of 1/32
/// voxel is allowed.
pub fn assert_on_planes(out: &Remeshed, planes: &[(P, f64)], what: &str) {
    let h = out.grid.voxel;
    let mut checked = 0;
    for p in &out.mesh.positions {
        let mut d: Vec<f64> = planes
            .iter()
            .map(|(n, off)| (dot(*n, wide(*p)) - off).abs() / h)
            .collect();
        d.sort_by(f64::total_cmp);
        if d[0] <= 1.0 && d[1] >= 3.0 {
            checked += 1;
            assert!(
                d[0] <= 0.04,
                "{what}: vertex {p:?} is {} voxels off its face",
                d[0]
            );
        }
    }
    assert!(checked >= 100, "non-trivial: {checked} vertices checked");
}

#[test]
fn flat_faces_come_out_flat() {
    let out = run(&tilted_box(), 24, false);
    let mut planes = Vec::new();
    for a in 0..3 {
        let mut axis = [0.0; 3];
        axis[a] = 1.0;
        planes.push((turn(axis), HALF[a]));
        planes.push((turn(axis), -HALF[a]));
    }
    assert_on_planes(&out, &planes, "tilted box");
    // Faces lying exactly on sample planes: the surface passes through samples.
    let out = run(&cuboid([-1.0; 3], [1.0; 3], 4), 16, false);
    let planes: Vec<(P, f64)> = (0..6)
        .map(|k| {
            let mut n = [0.0; 3];
            n[k / 2] = 1.0;
            (n, if k % 2 == 0 { 1.0 } else { -1.0 })
        })
        .collect();
    assert_on_planes(&out, &planes, "aligned cube");
}

#[test]
fn shapes_that_touch_at_an_edge_or_a_corner_are_repaired_into_one_manifold() {
    let cube = cuboid([0.0; 3], [1.0; 3], 2);
    for (what, other) in [
        ("edge", cuboid([1.0, 1.0, 0.0], [2.0, 2.0, 1.0], 2)),
        ("corner", cuboid([1.0; 3], [2.0; 3], 2)),
    ] {
        let out = run(&merge(&cube, &other), 8, false);
        assert!(out.work.flips >= 1, "{what}: the touch needs a repair");
        assert_closed_manifold(&out, what);
        assert_eq!(components(&out.mesh), 1, "{what}: joined");
        assert!(euler_is(&out.mesh, 2), "{what}: no tunnel");
    }
}

#[test]
fn two_shells_closer_than_a_voxel_still_give_a_manifold() {
    // Concentric spheres a fraction of a voxel apart, inner one wound inward: a thin shell.
    let outer = sphere([0.0; 3], 1.0, 12);
    let inner = flipped(&sphere([0.013, -0.02, 0.031], 0.93, 12));
    let out = run(&merge(&outer, &inner), 20, false);
    assert_closed_manifold(&out, "thin shell");
}

#[test]
fn the_surface_is_within_half_a_voxel_before_projection_and_near_zero_after() {
    // After projection every vertex sits on the input at its closest point (checked exactly
    // below). What is left of the Hausdorff distance is the flatness of the new quads across a
    // curved surface, about (diagonal^2) / (8 * radius): some 0.05 voxel for this sphere
    // (radius 8 voxels), more for the torus's thin tube (radius 2.6 voxels).
    for (what, mesh, resolution, after) in [
        ("sphere", sphere([0.1, 0.2, 0.3], 1.0, 12), 16, 0.1),
        ("torus", torus(1.0, 0.4, 48, 24), 18, 0.2),
    ] {
        let plain = run(&mesh, resolution, false);
        assert_eq!(plain.sign, SignMethod::Winding, "{what}: closed input");
        let h = voxels(&mesh, &plain);
        assert!(h <= 0.5, "{what}: {h} voxels before projection");
        let projected = run(&mesh, resolution, true);
        assert_closed_manifold(&projected, what);
        let voxel = projected.grid.voxel;
        let near = Nearest::new(triangles(&mesh), voxel);
        // Each vertex lands on the input, at the point closest to where it was.
        for (p, q) in projected.mesh.positions.iter().zip(&plain.mesh.positions) {
            let off = near.distance(wide(*p));
            assert!(
                off <= 1e-4 * voxel,
                "{what}: vertex {p:?} is {off} off the input"
            );
            let moved = norm(sub(wide(*p), wide(*q)));
            let closest = near.distance(wide(*q));
            assert!(
                moved <= closest + 1e-3 * voxel,
                "{what}: moved {moved} where the input is {closest} away"
            );
        }
        let h = voxels(&mesh, &projected);
        assert!(h <= after, "{what}: {h} voxels after projection");
    }
}

#[test]
fn the_volume_is_within_one_percent_at_fine_resolution() {
    let mesh = sphere([0.1, 0.2, 0.3], 1.0, 32);
    assert_row_route(&mesh);
    let out = run(&mesh, 64, false);
    let (got, want) = (volume(&out.mesh), volume(&mesh));
    // The requirement is 1%. The method gets about 0.1% here, so hold it to 0.3%: a surface
    // that drifts a tenth of a voxel in or out breaks that, and not much else would show it.
    assert!(
        (got / want - 1.0).abs() <= 0.003,
        "volume {got} against {want}"
    );
}

#[test]
fn overlapping_spheres_become_one_clean_shell() {
    let (ca, ra, cb, rb) = ([0.0; 3], 1.0, [1.1, 0.2, -0.1], 0.8);
    let a = sphere(ca, ra, 8);
    let b = sphere(cb, rb, 8);
    let both = merge(&a, &b);
    let out = run(&both, 24, false);
    assert_eq!(
        out.sign,
        SignMethod::Winding,
        "two closed spheres are closed"
    );
    assert_closed_manifold(&out, "union");
    assert_eq!(components(&out.mesh), 1, "one shell");
    assert!(euler_is(&out.mesh, 2), "a sphere, no inner walls");
    // Every point of the output is near the input, and every point of the input that is not
    // buried inside the other sphere is near the output.
    let h = out.grid.voxel;
    assert!(
        fast_hausdorff_from(&out.mesh, &both, h) <= 0.5 * h,
        "output on the input"
    );
    let near = Nearest::new(triangles(&out.mesh), h);
    for p in surface_points(&both) {
        let buried = norm(sub(p, ca)) <= ra - 1e-3 || norm(sub(p, cb)) <= rb - 1e-3;
        if !buried {
            assert!(near.distance(p) <= 0.5 * h, "outer surface point {p:?}");
        }
    }
    let d = norm(sub(cb, ca));
    let lens = std::f64::consts::PI
        * (ra + rb - d)
        * (ra + rb - d)
        * (d * d + 2.0 * d * (ra + rb) - 3.0 * (ra - rb) * (ra - rb))
        / (12.0 * d);
    let union = volume(&a) + volume(&b) - lens;
    let got = volume(&out.mesh);
    assert!(
        (got / union - 1.0).abs() <= 0.02,
        "union volume {got} against {union}"
    );
}

#[test]
fn a_cavity_wound_inward_stays_hollow_and_inside_out_input_is_read_right() {
    let outer = sphere([0.0; 3], 1.0, 12);
    let cavity = flipped(&sphere([0.0; 3], 0.5, 8));
    let out = run(&merge(&outer, &cavity), 24, false);
    assert_closed_manifold(&out, "hollow");
    assert_eq!(components(&out.mesh), 2, "outer wall and inner wall");
    let want = volume(&outer) + volume(&cavity);
    assert!(
        (volume(&out.mesh) / want - 1.0).abs() <= 0.03,
        "shell volume"
    );

    let normal = run(&outer, 16, false);
    let inverted = run(&flipped(&outer), 16, false);
    assert_eq!(
        inverted.sign,
        SignMethod::Winding,
        "inside out is still closed"
    );
    assert!(
        inverted.mesh.faces == normal.mesh.faces,
        "the same, outward-facing quads"
    );
    let moved = inverted
        .mesh
        .positions
        .iter()
        .zip(&normal.mesh.positions)
        .map(|(a, b)| norm(sub(wide(*a), wide(*b))))
        .fold(0.0, f64::max);
    assert!(moved <= 1e-5, "the same positions, to rounding: {moved}");
}

/// A closed tetrahedron whose bottom face is split at the midpoint M of edge AB, closed up
/// by the zero-area triangle A M B lying along that edge.
pub fn tetra_with_fin() -> PlainMesh {
    let t = u32::MAX;
    PlainMesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.5, 0.0, 0.0],
        ],
        // A=0 B=1 C=2 D=3 M=4
        faces: vec![
            [0, 2, 4, t],
            [4, 2, 1, t],
            [0, 4, 1, t],
            [0, 1, 3, t],
            [0, 3, 2, t],
            [1, 2, 3, t],
        ],
        attributes: std::collections::BTreeMap::new(),
    }
}

#[test]
fn a_zero_area_triangle_or_a_repeated_corner_does_not_open_a_closed_mesh() {
    let mut mesh = tetra_with_fin();
    // A face that repeats a corner has no area and no proper edges; it is ignored.
    mesh.faces.push([0, 0, 3, u32::MAX]);
    let out = run(&mesh, 16, false);
    assert_eq!(out.sign, SignMethod::Winding, "still closed");
    assert_closed_manifold(&out, "tetrahedron with a fin");
    let h = voxels(&mesh, &out);
    assert!(h <= 1.5, "{h} voxels: only the sharp corners are rounded");
}

#[test]
fn a_surface_just_past_a_bricks_corner_still_meshes_closed() {
    // Three specks far away pin the bounding box's low corner at the origin, so with 1-unit
    // cells the grid starts at -3: sample 16 sits at 13, and brick (1, 1, 1) covers samples
    // 8 to 15. A big sphere (radius 10, smooth, so nothing needs repair) bulges toward that
    // brick: it holds sample (16, 16, 16) half a voxel deep but stays more than a voxel from
    // every sample of brick (1, 1, 1) and of the bricks below it, so none of them is allocated.
    // The corner cell of brick (1, 1, 1) still straddles the surface, and must be meshed from
    // the allocated bricks above it.
    let speck = |at: P| cuboid(at, at.map(|c| c + 0.2), 1);
    let r = 10.0;
    let mut mesh = sphere([13.0 + (r - 0.5) / 3.0_f64.sqrt(); 3], r, 16);
    for at in [[0.0, 40.0, 40.0], [40.0, 0.0, 40.0], [40.0, 40.0, 0.0]] {
        mesh = merge(&mesh, &speck(at));
    }
    let settings = Settings {
        cell: CellSize::Size(1.0),
        project: false,
        blur: 0.0,
    };
    let out = remesh(&mesh, &settings).unwrap_or_else(|e| panic!("{e}"));
    let origin = out.grid.origin.map(f64::to_bits);
    assert_eq!(
        origin,
        [(-3.0_f64).to_bits(); 3],
        "the layout this case depends on"
    );
    assert_eq!(
        out.work.flips, 0,
        "nothing repaired, so no repair allocates bricks"
    );
    assert_closed_manifold(&out, "sphere and specks");
    assert_eq!(components(&out.mesh), 4, "the sphere and three specks");
}

#[test]
fn finding_sources_starts_next_to_the_answer() {
    let out = run(&sphere([0.1, 0.2, 0.3], 1.0, 12), 16, true);
    let per_vertex = out.work.walk_steps as f64 / out.mesh.positions.len() as f64;
    // About 0.15 here: the first guess is usually the closest triangle already.
    assert!(
        per_vertex > 0.0 && per_vertex <= 0.4,
        "{per_vertex} walk steps per vertex"
    );
}
