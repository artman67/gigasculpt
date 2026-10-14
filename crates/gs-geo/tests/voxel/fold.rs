//! `remesh_fold` (decision 0057 section 3): operands folded left to right by their operations,
//! each read as a solid of its own, checked with the independent volume and winding oracles in
//! `common`.

use gs_geo::operation::Operation::{self, Difference, Intersect, Union};
use gs_geo::voxel::{
    CellSize, CloseHoles, RemeshError, Remeshed, Settings, remesh, remesh_fold, remesh_parts_holes,
};
use gs_protocol::{AttrDomain, AttrValues, PlainMesh};

use crate::common::*;
use crate::shape::{assert_closed_manifold, settings};

/// `operands` merged in order, with a vertex column when `columns`, and their face counts and
/// operations.
fn merged(
    operands: &[(&PlainMesh, Operation)],
    columns: bool,
) -> (PlainMesh, Vec<(usize, Operation)>) {
    let first = operands[0].0.clone();
    let mut all = (operands[1..].iter()).fold(first, |all, o| merge(&all, o.0));
    if columns {
        let n = all.positions.len();
        let mask = (0..n).map(|v| v as f32 / n as f32).collect();
        all.attributes.insert(
            "mask".into(),
            column(AttrDomain::Vertex, AttrValues::F32(mask)),
        );
    }
    let list = operands.iter().map(|o| (o.0.faces.len(), o.1)).collect();
    (all, list)
}

/// `operands` merged in order and folded, holes closed.
fn fold(operands: &[(&PlainMesh, Operation)], settings: &Settings) -> Remeshed {
    let (all, list) = merged(operands, false);
    remesh_fold(&all, &list, settings, CloseHoles::Large).unwrap_or_else(|e| panic!("{e}"))
}

/// Every position bit and corner, and the route, work and sources, to compare runs by.
pub fn bits(out: &Remeshed) -> (Vec<u32>, String) {
    let mut bits: Vec<u32> = (out.mesh.positions.iter().flatten())
        .map(|x| x.to_bits())
        .collect();
    bits.extend(out.mesh.faces.iter().flatten());
    (
        bits,
        format!("{:?} {:?} {:?}", out.sign, out.work, out.sources),
    )
}

fn near(got: f64, want: f64, tolerance: f64, what: &str) {
    assert!(
        (got - want).abs() <= tolerance,
        "{what}: volume {got}, want {want} within {tolerance}"
    );
}

#[test]
fn a_box_minus_a_box_keeps_the_body_less_the_overlap() {
    // Volumes: body 2, cutter 1, overlap 0.5 * 0.5 * 1 = 0.25. A fold that unions every operand
    // (research 26's wrong result 11) gives 2.75 for the difference.
    let body = cuboid([0.0; 3], [2.0, 1.0, 1.0], 4);
    let cutter = cuboid([1.5, 0.25, -0.5], [2.5, 0.75, 1.5], 4);
    let s = settings(64, false);
    for (op, want) in [(Difference, 1.75), (Intersect, 0.25), (Union, 2.75)] {
        let out = fold(&[(&body, Union), (&cutter, op)], &s);
        assert_closed_manifold(&out, &format!("{op:?}"));
        near(volume(&out.mesh), want, 0.02, &format!("{op:?}"));
    }
    // The base's own operation is ignored: it is where the fold starts.
    let based = fold(&[(&body, Difference), (&cutter, Difference)], &s);
    near(volume(&based.mesh), 1.75, 0.02, "a base marked difference");
    // Intersecting with an operand that has no faces leaves nothing.
    let (all, mut list) = merged(&[(&body, Union), (&cutter, Intersect)], false);
    list.push((0, Intersect));
    let empty = remesh_fold(&all, &list, &s, CloseHoles::Large).unwrap();
    assert!(empty.mesh.faces.is_empty(), "nothing in an empty operand");
    list.push((1, Union));
    let err = remesh_fold(&all, &list, &s, CloseHoles::Large).err();
    assert_eq!(err, Some(RemeshError::BadParts), "counts must add up");
}

#[test]
fn spheres_fold_to_the_volumes_their_union_implies() {
    let a = sphere([0.0; 3], 1.0, 12);
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    let s = settings(48, true);
    let alone = |m: &PlainMesh| volume(&fold(&[(m, Union)], &s).mesh);
    let (va, vb) = (alone(&a), alone(&b));
    let union = volume(&fold(&[(&a, Union), (&b, Union)], &s).mesh);
    let minus = fold(&[(&a, Union), (&b, Difference)], &s);
    let both = fold(&[(&a, Union), (&b, Intersect)], &s);
    for (out, what) in [(&minus, "difference"), (&both, "intersect")] {
        assert_closed_manifold(out, what);
    }
    let tolerance = 0.01 * union;
    near(volume(&minus.mesh), union - vb, tolerance, "a minus b");
    near(volume(&both.mesh), va + vb - union, tolerance, "a and b");
    let tris = triangles(&minus.mesh);
    assert!(
        winding(&tris, [0.7, 0.15, -0.05]).abs() < 0.5,
        "the overlap is cut away"
    );
    assert!(
        winding(&tris, [-0.5, 0.0, 0.0]) > 0.5,
        "the rest of a stays"
    );
    let tris = triangles(&both.mesh);
    assert!(
        winding(&tris, [0.7, 0.15, -0.05]) > 0.5,
        "the overlap stays"
    );
    assert!(
        winding(&tris, [-0.5, 0.0, 0.0]).abs() < 0.5,
        "a alone is gone"
    );
}

#[test]
fn a_cutter_beside_the_body_leaves_the_body_bit_for_bit() {
    // The cutter starts half a voxel past the body's +x face, on the same grid (a set cell
    // size, and the cutter adds nothing below the body's lowest corner). Each operand keeps its
    // own distances, so the samples between them still read the body's, where one distance to
    // the nearest triangle of either would read the cutter's.
    let voxel = 1.0 / 32.0;
    let body = cuboid([0.0; 3], [0.99; 3], 4);
    let cutter = cuboid([0.99 + 0.5 * voxel, 0.2, 0.2], [1.5, 0.8, 0.8], 2);
    let s = Settings {
        cell: CellSize::Size(voxel as f32),
        project: false,
        blur: 0.0,
    };
    let alone = remesh(&body, &s).unwrap();
    let cut = fold(&[(&body, Union), (&cutter, Difference)], &s);
    let origin = |out: &Remeshed| out.grid.origin.map(f64::to_bits);
    assert!(origin(&cut) == origin(&alone), "the same grid");
    assert!(cut.mesh.faces == alone.mesh.faces, "the same quads");
    assert!(
        cut.mesh.positions == alone.mesh.positions,
        "in the same places"
    );
    let apart = fold(&[(&body, Union), (&cutter, Intersect)], &s);
    assert!(apart.mesh.faces.is_empty(), "they do not overlap");
}

#[test]
fn a_run_of_unions_is_remesh_parts_bit_for_bit() {
    // Inside out, facing out, open and messy parts, as `parts`' thread test takes them.
    let a = flipped(&sphere([0.0; 3], 1.0, 8));
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    let c = without(&sphere([-0.6, 0.5, 0.3], 0.6, 8), |q| {
        q.iter().all(|p| p[1] >= 0.9)
    });
    let d = doubled(&sphere([0.2, -0.8, 0.0], 0.5, 6));
    let (large, off) = (CloseHoles::Large, CloseHoles::Off);
    let cases = [
        (vec![&a, &b, &c, &d], large, false, Union),
        (vec![&a, &b, &c, &d], off, true, Difference),
        (vec![&a, &b, &c, &d], large, true, Intersect),
        (vec![&c, &d], off, false, Union),
        (vec![&c, &d], large, true, Union),
        (vec![&b], large, false, Difference),
    ];
    for (set, close, project, base) in cases {
        let s = settings(40, project);
        let operands: Vec<(&PlainMesh, Operation)> = (set.iter().enumerate())
            .map(|(k, m)| (*m, if k == 0 { base } else { Union }))
            .collect();
        let (all, list) = merged(&operands, true);
        let counts: Vec<usize> = list.iter().map(|o| o.0).collect();
        let parts = remesh_parts_holes(&all, &counts, &s, close).unwrap();
        let folded = remesh_fold(&all, &list, &s, close).unwrap();
        let what = format!("{} parts, {close:?}, base {base:?}", set.len());
        assert!(bits(&folded) == bits(&parts), "{what}: the same bits");
        assert!(folded.kept == parts.kept, "{what}: the same kept size");
    }
}

#[test]
fn a_cutter_sharing_points_with_the_body_is_not_welded_into_it() {
    // An octahedron whose middle square lies on the body's +x face, on four of its points (bit
    // for bit). Welded into the body it would be one solid, a union; cut, it leaves a pyramid
    // shaped dent of half its volume, 4/3 * 0.4^3 / 2.
    let body = cuboid([0.0; 3], [2.0, 1.0, 1.0], 10);
    let point = |y: f64, z: f64| -> [f32; 3] {
        let at = body.positions.iter().find(|p| {
            let q = wide(**p);
            (q[0] - 2.0).abs() < 1e-6 && (q[1] - y).abs() < 1e-6 && (q[2] - z).abs() < 1e-6
        });
        *at.expect("a point of the body's +x face")
    };
    let positions = vec![
        [2.4, 0.5, 0.5],
        [1.6, 0.5, 0.5],
        point(0.9, 0.5),
        point(0.5, 0.9),
        point(0.1, 0.5),
        point(0.5, 0.1),
    ];
    let t = u32::MAX;
    // The square's corners run counter-clockwise seen from +x; each tip's four faces face out.
    let faces = (0..4)
        .flat_map(|k| {
            let (u, v) = (2 + k, 2 + (k + 1) % 4);
            [[0, u, v, t], [1, v, u, t]]
        })
        .collect();
    let octahedron = PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    };
    let s = settings(64, true);
    near(
        volume(&fold(&[(&octahedron, Union)], &s).mesh),
        0.0853,
        0.003,
        "the octahedron",
    );
    let out = fold(&[(&body, Union), (&octahedron, Difference)], &s);
    assert_closed_manifold(&out, "body minus octahedron");
    near(
        volume(&out.mesh),
        2.0 - 0.0427,
        0.006,
        "the body less the dent",
    );
    let tris = triangles(&out.mesh);
    assert!(
        winding(&tris, [1.8, 0.5, 0.5]).abs() < 0.5,
        "the dent is cut"
    );
    assert!(winding(&tris, [1.0, 0.5, 0.5]) > 0.5, "the body stays");
}

#[test]
fn sources_name_the_operand_each_point_came_from() {
    // A sphere takes a bite out of a box's corner.
    let body = cuboid([-1.0; 3], [1.0; 3], 8);
    let cutter = sphere([1.0; 3], 0.8, 8);
    let (all, list) = merged(&[(&body, Union), (&cutter, Difference)], true);
    let out = remesh_fold(&all, &list, &settings(40, false), CloseHoles::Large).unwrap();
    let voxel = out.grid.voxel;
    let first = body.positions.len() as u32;
    let to_box = |p: P| {
        let q = p.map(|c| c.abs() - 1.0);
        let outside = norm(q.map(|c| c.max(0.0)));
        outside + q[0].max(q[1]).max(q[2]).min(0.0)
    };
    let to_sphere = |p: P| norm(sub(p, [1.0; 3])) - 0.8;
    let mut cut = 0;
    for (v, source) in out.sources.points.iter().enumerate() {
        let (tri, _) = source.unwrap_or_else(|| panic!("point {v} has a source"));
        let in_cutter = tri[0] >= first;
        assert!(
            tri.iter().all(|&c| (c >= first) == in_cutter),
            "point {v}: one operand"
        );
        let p = wide(out.mesh.positions[v]);
        if to_sphere(p).abs() > 2.0 * voxel {
            assert!(
                !in_cutter,
                "point {v}, away from the cutter, names the body"
            );
        }
        if to_box(p).abs() > 2.0 * voxel {
            assert!(in_cutter, "point {v}, away from the box, names the cutter");
        }
        if in_cutter {
            cut += 1;
            assert!(
                to_box(p) <= voxel,
                "point {v} of the cut wall lies in the body"
            );
        } else {
            assert!(
                to_sphere(p) >= -voxel,
                "point {v} of the body is not in the bite"
            );
        }
    }
    assert!(cut > 0, "the cut wall came from the cutter");
}

#[test]
fn the_fold_gives_the_same_bits_on_1_3_and_24_threads() {
    // Every operation, inside out, open and messy operands, rows and winding numbers in one
    // fold, on a grid of more rows than one task takes.
    let a = flipped(&sphere([0.0; 3], 1.0, 8));
    let b = sphere([0.9, 0.2, -0.1], 0.7, 8);
    let c = without(&sphere([-0.6, 0.5, 0.3], 0.6, 8), |q| {
        q.iter().all(|p| p[1] >= 0.9)
    });
    let d = doubled(&sphere([0.2, -0.8, 0.0], 0.9, 6));
    let e = cuboid([-0.3, -0.3, -1.5], [0.3, 0.3, 1.5], 2);
    let operands = [
        (&a, Union),
        (&b, Union),
        (&c, Difference),
        (&d, Intersect),
        (&e, Difference),
    ];
    let (all, list) = merged(&operands, true);
    let runs: Vec<(Vec<u32>, String)> = [1, 3, 24]
        .into_iter()
        .map(|t| {
            gs_par::with_threads(t, || {
                bits(&remesh_fold(&all, &list, &settings(48, true), CloseHoles::Large).unwrap())
            })
        })
        .collect();
    assert!(!runs[0].0.is_empty(), "something is left");
    for (t, run) in [1, 3, 24].iter().zip(&runs) {
        assert!(run.0 == runs[0].0, "bits on {t} threads");
        assert_eq!(run.1, runs[0].1, "work and sources on {t} threads");
    }
}
