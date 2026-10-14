//! Open input. Holes in an otherwise clean surface are capped across their rims and signed by
//! the exact row winding; input no cap can fix (an edge used by three faces, or twice the same
//! way) is left to the generalized winding number. Checked with the independent oracles in
//! `common`: closed and manifold, the input kept, and the new surface inside the rim's bounds.

use std::collections::BTreeMap;

use gs_geo::voxel::{Remeshed, SignMethod};
use gs_protocol::{NO_VERTEX, PlainMesh};

use crate::common::*;
use crate::shape::{assert_closed_manifold, assert_on_planes, euler_is, run, voxels};

/// The edges only one face uses, as segments.
pub fn rim(mesh: &PlainMesh) -> Vec<[P; 2]> {
    let mut uses: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for f in &mesh.faces {
        let c: Vec<u32> = f.iter().copied().filter(|&v| v != NO_VERTEX).collect();
        for k in 0..c.len() {
            let (a, b) = (c[k], c[(k + 1) % c.len()]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    (uses.iter())
        .filter(|(_, n)| **n == 1)
        .map(|(e, _)| [e.0, e.1].map(|v| wide(mesh.positions[v as usize])))
        .collect()
}

/// Checks that everything the remesh of `open` made away from the input (the patch) lies in
/// the bounding box of the input's rim, give or take a voxel: a patch across the rim, nothing
/// leaking out, at least `least` points of it. Then that `keep` (the input, or what of it must
/// survive) is kept: within half a voxel of the result, except within two voxels of the rim,
/// where the corner between surface and patch is rounded off like any sharp edge.
fn assert_patch_within_rim(
    open: &PlainMesh,
    keep: &PlainMesh,
    out: &Remeshed,
    least: usize,
    what: &str,
) {
    let h = out.grid.voxel;
    let rim = rim(open);
    let ends = || rim.iter().flatten();
    let lo: P = std::array::from_fn(|a| ends().map(|p| p[a]).fold(f64::MAX, f64::min));
    let hi: P = std::array::from_fn(|a| ends().map(|p| p[a]).fold(f64::MIN, f64::max));
    let near = Nearest::new(triangles(open), h);
    let mut patch = 0;
    for p in surface_points(&out.mesh) {
        if near.distance(p) <= 0.5 * h {
            continue;
        }
        patch += 1;
        let within = (0..3).all(|a| p[a] >= lo[a] - h && p[a] <= hi[a] + h);
        assert!(within, "{what}: {p:?} is neither surface nor patch");
    }
    assert!(patch >= least, "{what}: {patch} patch points");
    // Checked second: bucketing a badly broken result for this query would be slow.
    let result = Nearest::new(triangles(&out.mesh), h);
    for p in surface_points(keep) {
        let corner = rim
            .iter()
            .any(|e| segment_distance(p, e[0], e[1]) < 2.0 * h);
        let allowed = if corner { 1.5 * h } else { 0.5 * h };
        let off = result.distance(p);
        assert!(
            off <= allowed,
            "{what}: input point {p:?} is {off} off the result"
        );
    }
}

/// Runs a remesh that must cap `holes` simple loops, and checks the result is one closed
/// manifold sphere.
fn capped(mesh: &PlainMesh, resolution: u32, holes: u64, what: &str) -> Remeshed {
    let out = run(mesh, resolution, false);
    assert_eq!(out.sign, SignMethod::CappedWinding, "{what}: capped");
    assert_eq!(out.work.holes, holes, "{what}: simple loops capped");
    assert_eq!(
        out.work.film_bricks, 0,
        "{what}: caps leave no film to find"
    );
    assert_closed_manifold(&out, what);
    assert_eq!(components(&out.mesh), 1, "{what}: one piece");
    assert!(euler_is(&out.mesh, 2), "{what}: closed over, no tunnel");
    out
}

/// The unit quad sphere of `sphere([0; 3], 1.0, n)` without the quads of its top (+y) side
/// whose cell `(i, j)` (counted along x and along z from the -1 side) `cut` picks.
fn holed(n: u32, cut: impl Fn(i32, i32) -> bool) -> PlainMesh {
    let cell = |c: f64| (f64::midpoint(c, 1.0) * f64::from(n)).floor() as i32;
    let mut mesh = without(&cuboid([-1.0; 3], [1.0; 3], n), |c| {
        let mid: P = std::array::from_fn(|a| c.iter().map(|p| p[a]).sum::<f64>() / 4.0);
        c.iter().all(|p| p[1] >= 1.0) && cut(cell(mid[0]), cell(mid[2]))
    });
    for p in &mut mesh.positions {
        let q = wide(*p);
        let s = 1.0 / norm(q);
        *p = q.map(|c| (c * s) as f32);
    }
    mesh
}

#[test]
fn a_hole_is_capped_across_its_rim() {
    let open = without(&sphere([0.0; 3], 1.0, 12), |c| {
        c.iter().all(|p| p[1] >= 0.55)
    });
    let out = capped(&open, 24, 1, "open sphere");
    assert_patch_within_rim(&open, &open, &out, 50, "open sphere");
}

#[test]
fn a_flat_rim_gets_a_flat_patch() {
    // A box open at the top, turned so the opening faces (1, 1, 1): its patch, like its faces,
    // must come out flat.
    let axes: [P; 3] =
        [[1.0, -1.0, 0.0], [1.0, 1.0, 1.0], [-1.0, -1.0, 2.0]].map(|a| a.map(|c| c / norm(a)));
    let open = turned_open_box(&axes);
    let out = capped(&open, 32, 1, "open box");
    let planes: Vec<(P, f64)> = (0..6)
        .map(|k| (axes[k / 2], if k % 2 == 0 { 1.0 } else { -1.0 }))
        .collect();
    assert_on_planes(&out, &planes, "open box");
}

/// The box from -1 to 1 without its +y side, turned so its own axes point along `axes`.
fn turned_open_box(axes: &[P; 3]) -> PlainMesh {
    let mut open = without(&cuboid([-1.0; 3], [1.0; 3], 4), |c| {
        c.iter().all(|p| p[1] >= 1.0 - 1e-9)
    });
    for p in &mut open.positions {
        let q = wide(*p);
        *p = std::array::from_fn(|a| (0..3).map(|k| q[k] * axes[k][a]).sum::<f64>() as f32);
    }
    open
}

#[test]
fn a_rim_that_is_not_flat_gets_a_patch_within_its_bounds() {
    // A saddle-shaped rim: high where x is large, low where z is, from about y = 0.06 to 0.55.
    let open = without(&sphere([0.0; 3], 1.0, 16), |c| {
        c.iter()
            .all(|p| p[1] >= 0.35 + 0.3 * (p[0] * p[0] - p[2] * p[2]))
    });
    let out = capped(&open, 32, 1, "saddle");
    assert_patch_within_rim(&open, &open, &out, 50, "saddle");
}

#[test]
fn two_holes_that_share_a_vertex_are_capped_one_by_one() {
    // Two quads that only touch at a corner.
    let open = holed(8, |i, j| (i, j) == (3, 3) || (i, j) == (4, 4));
    for (what, mesh) in [
        ("two holes", open.clone()),
        ("corner first", pinch_first(&open)),
    ] {
        let out = capped(&mesh, 24, 2, what);
        let h = voxels(&holed(8, |_, _| false), &out);
        assert!(h <= 0.55, "{what}: {h} voxels from the whole sphere");
    }
}

/// The mesh renumbered so the vertex its rim passes through twice comes first: the walk along
/// the rim then starts there and must go round both loops from it. (Numbered as built, the
/// walk reaches it partway round one loop.)
fn pinch_first(mesh: &PlainMesh) -> PlainMesh {
    let mut uses = vec![0; mesh.positions.len()];
    for f in &mesh.faces {
        for k in 0..4 {
            let (a, b) = (f[k], f[(k + 1) % 4]);
            let shared = mesh
                .faces
                .iter()
                .any(|g| (0..4).any(|j| g[j] == b && g[(j + 1) % 4] == a));
            if !shared {
                uses[a as usize] += 1;
            }
        }
    }
    let pinch = (0..uses.len())
        .find(|&v| uses[v] == 2)
        .unwrap_or_else(|| panic!("no vertex on two rim loops")) as u32;
    let swap = |v: u32| match v {
        0 => pinch,
        _ if v == pinch => 0,
        _ => v,
    };
    let mut out = mesh.clone();
    out.positions.swap(0, pinch as usize);
    out.faces = mesh.faces.iter().map(|f| f.map(swap)).collect();
    out
}

#[test]
fn a_hole_whose_rim_touches_itself_is_capped_as_simple_loops() {
    // Seven quads in a ring round the quad (4, 3), open where its two ends, (3, 3) and (4, 4),
    // touch at a corner. That quad is left hanging by that corner, and the hole's rim passes
    // through it twice: the outer rim and the quad's own make two simple loops.
    let ring = [(3, 3), (3, 2), (4, 2), (5, 2), (5, 3), (5, 4), (4, 4)];
    let open = holed(8, |i, j| ring.contains(&(i, j)));
    // The hanging quad lies inside the outer loop, so the patch across that loop covers it.
    // The rest of the input is kept.
    let rest = holed(8, |i, j| ring.contains(&(i, j)) || (i, j) == (4, 3));
    for (what, mesh) in [
        ("touching rim", open.clone()),
        ("corner first", pinch_first(&open)),
    ] {
        let out = capped(&mesh, 24, 2, what);
        assert_patch_within_rim(&mesh, &rest, &out, 10, what);
    }
}

#[test]
fn a_sheet_becomes_the_solid_its_rim_closes() {
    // The upper half of a sphere, one big hole: a half ball with a flat bottom.
    let full = sphere([0.0; 3], 1.0, 8);
    let sheet = without(&full, |c| c.iter().all(|p| p[1] <= 0.0));
    let out = capped(&sheet, 32, 1, "sheet");
    let (got, want) = (volume(&out.mesh), volume(&full) / 2.0);
    assert!(
        (got / want - 1.0).abs() <= 0.01,
        "volume {got} against {want}"
    );
    let h = out.grid.voxel;
    let mut bottom = 0;
    for p in out.mesh.positions.iter().map(|&p| wide(p)) {
        if p[1].abs() <= h && p[0] * p[0] + p[2] * p[2] <= (1.0 - 3.0 * h) * (1.0 - 3.0 * h) {
            bottom += 1;
            assert!(p[1].abs() <= 0.04 * h, "{p:?} is off the flat bottom");
        }
    }
    assert!(bottom >= 100, "non-trivial: {bottom} bottom vertices");
}

#[test]
fn what_no_cap_can_fix_takes_the_generalized_winding_number() {
    // An open sphere with a fin on one edge (an edge used by three faces), and a closed sphere
    // with one face turned over (its edges used twice the same way).
    let open = without(&sphere([0.0; 3], 1.0, 12), |c| {
        c.iter().all(|p| p[1] >= 0.55)
    });
    let mut fin = open.clone();
    let f = fin.faces[0];
    let tip = std::array::from_fn(|a| {
        0.6 * (fin.positions[f[0] as usize][a] + fin.positions[f[1] as usize][a])
    });
    fin.positions.push(tip);
    fin.faces
        .push([f[0], f[1], fin.positions.len() as u32 - 1, NO_VERTEX]);
    let mut turned = sphere([0.0; 3], 1.0, 24);
    turned.faces[0] = [
        turned.faces[0][0],
        turned.faces[0][3],
        turned.faces[0][2],
        turned.faces[0][1],
    ];
    for (what, mesh) in [("fin", fin), ("turned face", turned)] {
        let out = run(&mesh, 20, false);
        assert_eq!(out.sign, SignMethod::GeneralizedWinding, "{what}");
        assert_eq!(out.work.holes, 0, "{what}: no caps");
        assert_closed_manifold(&out, what);
        assert!(euler_is(&out.mesh, 2), "{what}: one closed sphere");
    }
}

#[test]
fn a_wide_hole_no_cap_can_fix_is_closed_by_a_film_across_bricks_that_had_no_triangles() {
    // The turned open box with one face doubled: no caps. The film over its opening is about
    // 20 cells across, mostly in bricks far from any triangle, which the sign stage must
    // allocate. Facing (1, 1, 1), much of the film cannot be reached from the rim by steps
    // along -x, -y and -z alone, so a sign change must be followed across every face of a
    // brick.
    let axes: [P; 3] =
        [[1.0, -1.0, 0.0], [1.0, 1.0, 1.0], [-1.0, -1.0, 2.0]].map(|a| a.map(|c| c / norm(a)));
    let open = doubled(&turned_open_box(&axes));
    let out = run(&open, 32, false);
    assert_eq!(out.sign, SignMethod::GeneralizedWinding, "a doubled face");
    assert!(
        out.work.film_bricks >= 1,
        "the film needed bricks: {:?}",
        out.work
    );
    assert_closed_manifold(&out, "open box");
    assert!(euler_is(&out.mesh, 2), "closed over");
    // Every vertex is on the input or on the film: the plane of the opening, inside it.
    let h = out.grid.voxel;
    let near = Nearest::new(triangles(&open), h);
    for p in out.mesh.positions.iter().map(|&p| wide(p)) {
        let film = (dot(p, axes[1]) - 1.0).abs() <= 2.0 * h
            && dot(p, axes[0]).abs() <= 1.0
            && dot(p, axes[2]).abs() <= 1.0;
        assert!(
            film || near.distance(p) <= 0.5 * h,
            "{p:?} is neither surface nor film"
        );
    }
}

/// The mesh with every face given its own corners, as STL files store them, zero written as
/// -0 in every other face, and one more face: a zero-area triangle whose first two corners are
/// copies of face 0's first corner and the third a copy of its second.
fn separate(mesh: &PlainMesh) -> PlainMesh {
    let mut out = PlainMesh {
        positions: Vec::new(),
        faces: Vec::new(),
        attributes: BTreeMap::new(),
    };
    for (k, f) in mesh.faces.iter().enumerate() {
        let first = out.positions.len() as u32;
        for &v in f {
            let signed = |c: f32| if k % 2 == 1 && c == 0.0 { -0.0 } else { c };
            out.positions.push(mesh.positions[v as usize].map(signed));
        }
        out.faces.push([first, first + 1, first + 2, first + 3]);
    }
    let first = out.positions.len() as u32;
    let [a, b] = [0, 1].map(|k| mesh.positions[mesh.faces[0][k] as usize]);
    out.positions.extend([a, a, b]);
    out.faces.push([first, first + 1, first + 2, NO_VERTEX]);
    out
}

#[test]
fn a_mesh_stored_as_separate_triangles_reads_as_its_shared_vertex_mesh() {
    let shared = sphere([0.0; 3], 1.0, 8);
    let apart = separate(&shared);
    let zeros = (apart.positions.iter().flatten()).filter(|c| c.to_bits() == (-0.0f32).to_bits());
    assert!(zeros.count() >= 10, "non-trivial: negative zeros written");
    let (a, b) = (run(&shared, 16, false), run(&apart, 16, false));
    assert_eq!(b.sign, SignMethod::Winding, "closed");
    assert!(
        a.mesh.positions == b.mesh.positions && a.mesh.faces == b.mesh.faces,
        "the same remesh, bit for bit"
    );
    // With a hole: capped, the extra face no obstacle.
    let open = without(&shared, |c| c.iter().all(|p| p[1] >= 0.5));
    capped(&separate(&open), 16, 1, "open, separate triangles");
}

#[test]
fn an_inside_region_that_reaches_the_grid_edge_is_closed_inside_the_grid() {
    // An open domed patch over itself (a part and its duplicate, merged): every edge is used
    // twice the same way, so no cap can fix it, and the generalized winding number counts as
    // inside across a lens on the patch's hollow side that reaches farther than the grid's few
    // voxels of margin. Facing up, the lens leaves the grid's low side (this panicked with a
    // subtraction overflow in Surface Nets); turned over, its high side.
    let patch = gs_testkit::fixtures::patch(6);
    let mut turned = patch.clone();
    for p in &mut turned.positions {
        *p = [p[0], -p[1], -p[2]];
    }
    for (what, sheet) in [("facing up", patch), ("facing down", turned)] {
        let doubled = merge(&sheet, &sheet);
        let out = run(&doubled, 32, false);
        assert_eq!(out.sign, SignMethod::GeneralizedWinding, "{what}");
        assert_closed_manifold(&out, what);
        let (grid, h) = (out.grid, out.grid.voxel);
        let ends: [(f64, f64); 3] = std::array::from_fn(|a| {
            let n = f64::from(grid.samples()[a]);
            (grid.origin[a], grid.origin[a] + (n - 1.0) * h)
        });
        for p in out.mesh.positions.iter().map(|&p| wide(p)) {
            for (a, &(lo, hi)) in ends.iter().enumerate() {
                assert!(
                    p[a] > lo && p[a] < hi,
                    "{what}: {p:?} lies on or past the grid's outer samples"
                );
            }
        }
        // Not vacuous: the surface runs along the edge the lens reaches.
        let y = |p: &[f32; 3]| f64::from(p[1]);
        let edge = if what == "facing up" {
            ends[1].0
        } else {
            ends[1].1
        };
        let along = (out.mesh.positions.iter()).filter(|p| (y(p) - edge).abs() <= h);
        assert!(
            along.count() >= 100,
            "{what}: the lens did not reach the edge"
        );
    }
}
