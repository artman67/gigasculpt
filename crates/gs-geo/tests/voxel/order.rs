//! Determinism across thread counts (ARCHITECTURE 8), spatial order of the output (3.1), and
//! the refusals.

use gs_geo::voxel::{
    CellSize, MAX_CELLS, RemeshError, Settings, SignMethod, remesh, signed_distance,
};
use gs_protocol::{AttrDomain, AttrValues, PlainMesh};

use crate::common::*;
use crate::shape::{assert_row_route, run};

/// Every bit of a mesh: positions, faces and attribute columns.
fn bits(mesh: &PlainMesh) -> Vec<u32> {
    let mut out: Vec<u32> = mesh
        .positions
        .iter()
        .flatten()
        .map(|c| c.to_bits())
        .collect();
    out.extend(mesh.faces.iter().flatten());
    for a in mesh.attributes.values() {
        match &a.values {
            AttrValues::F32(v) => out.extend(v.iter().map(|x| x.to_bits())),
            AttrValues::U32(v) => out.extend(v),
        }
    }
    out
}

#[test]
fn the_result_is_bit_identical_on_1_2_3_8_and_24_threads() {
    // Closed parts overlapping (row winding), an open part (capped), the open part with a face
    // doubled (winding number), attributes and a blurred projection, so every parallel stage
    // runs.
    let mut closed = merge(&sphere([0.0; 3], 1.0, 8), &torus(0.9, 0.3, 24, 12));
    let mask = closed.positions.iter().map(|p| p[0] - p[1]).collect();
    closed.attributes.insert(
        "mask".into(),
        column(AttrDomain::Vertex, AttrValues::F32(mask)),
    );
    let open = without(&sphere([0.5, 0.3, 0.0], 0.6, 8), |c| {
        c.iter().all(|p| p[1] >= 0.6)
    });
    let settings = Settings {
        cell: CellSize::Resolution(16),
        project: true,
        blur: 30.0,
    };
    for (what, mesh, method) in [
        ("closed", closed, SignMethod::Winding),
        ("open", open.clone(), SignMethod::CappedWinding),
        ("messy", doubled(&open), SignMethod::GeneralizedWinding),
    ] {
        let runs: Vec<(Vec<u32>, String)> = [1, 2, 3, 8, 24]
            .into_iter()
            .map(|t| {
                gs_par::with_threads(t, || {
                    let out = remesh(&mesh, &settings).unwrap_or_else(|e| panic!("{e}"));
                    assert_eq!(out.sign, method, "{what}: the sign route under test");
                    let field =
                        signed_distance(&mesh, settings.cell).unwrap_or_else(|e| panic!("{e}"));
                    let samples: Vec<u32> =
                        field.samples.iter().map(|s| s.value.to_bits()).collect();
                    let mut all = bits(&out.mesh);
                    all.extend(samples);
                    (all, format!("{:?}", out.work))
                })
            })
            .collect();
        for (t, run) in [1, 2, 3, 8, 24].iter().zip(&runs) {
            assert!(run.0 == runs[0].0, "{what}: bits on {t} threads");
            assert_eq!(run.1, runs[0].1, "{what}: work on {t} threads");
        }
    }
}

#[test]
fn vertices_and_faces_come_out_in_spatial_order() {
    let mesh = sphere([0.1, 0.2, 0.3], 1.0, 12);
    assert_row_route(&mesh);
    let out = run(&mesh, 40, false);
    let p = &out.mesh.positions;
    let steps: Vec<f64> = p
        .windows(2)
        .map(|w| norm(sub(wide(w[1]), wide(w[0]))))
        .collect();
    let mean = steps.iter().sum::<f64>() / steps.len() as f64 / out.grid.voxel;
    assert!(
        mean <= 2.0,
        "consecutive vertices are {mean} voxels apart on average"
    );
    // Shuffled, the same vertices would be about a radius apart (20 voxels here).
    let mut spans: Vec<u32> = out
        .mesh
        .faces
        .iter()
        .map(|f| f.iter().max().unwrap_or(&0) - f.iter().min().unwrap_or(&0))
        .collect();
    spans.sort_unstable();
    let median = spans[spans.len() / 2] as usize;
    assert!(
        median * 50 <= p.len(),
        "a face's corners are {median} indices apart (median)"
    );
    let centres: Vec<P> = out
        .mesh
        .faces
        .iter()
        .map(|f| {
            let c = corners(&out.mesh, f);
            std::array::from_fn(|a| c.iter().map(|q| q[a]).sum::<f64>() / 4.0)
        })
        .collect();
    let mean = centres
        .windows(2)
        .map(|w| norm(sub(w[1], w[0])))
        .sum::<f64>()
        / (centres.len() - 1) as f64
        / out.grid.voxel;
    assert!(
        mean <= 2.0,
        "consecutive faces are {mean} voxels apart on average"
    );
}

/// The refusal for `mesh` at `cell`; panics if the remesh runs.
fn refuse(mesh: &PlainMesh, cell: CellSize) -> RemeshError {
    let settings = Settings {
        cell,
        project: false,
        blur: 0.0,
    };
    match remesh(mesh, &settings) {
        Ok(_) => panic!("{cell:?} should be refused"),
        Err(e) => e,
    }
}

#[test]
fn broken_meshes_are_refused() {
    let cube = cuboid([0.0; 3], [1.0; 3], 1);
    let ok = CellSize::Resolution(8);
    let mut empty = cube.clone();
    empty.faces.clear();
    assert_eq!(refuse(&empty, ok), RemeshError::NoFaces, "no faces");
    let mut repeats = cube.clone();
    repeats.faces = vec![[0, 0, 1, u32::MAX], [2, 2, 2, 2]];
    assert_eq!(
        refuse(&repeats, ok),
        RemeshError::NoFaces,
        "no proper faces"
    );
    let mut index = cube.clone();
    index.faces[3][2] = 99;
    assert_eq!(
        refuse(&index, ok),
        RemeshError::BadFace(3),
        "index past the end"
    );
    let mut marker = cube.clone();
    marker.faces[4][1] = u32::MAX;
    assert_eq!(
        refuse(&marker, ok),
        RemeshError::BadFace(4),
        "marker too early"
    );
    let mut nan = cube.clone();
    nan.positions[2][1] = f32::NAN;
    assert_eq!(refuse(&nan, ok), RemeshError::NotFinite, "NaN");
    for (domain, len) in [(AttrDomain::Vertex, 7), (AttrDomain::Face, 5)] {
        for values in [
            AttrValues::F32(vec![0.0; len]),
            AttrValues::U32(vec![0; len]),
        ] {
            let mut short = cube.clone();
            short.attributes.insert("m".into(), column(domain, values));
            let err = refuse(&short, ok);
            assert_eq!(err, RemeshError::BadAttribute("m".into()), "{domain:?}");
        }
    }
}

#[test]
fn bad_cell_sizes_are_refused() {
    let cube = cuboid([0.0; 3], [1.0; 3], 1);
    for bad in [
        CellSize::Resolution(0),
        CellSize::Size(0.0),
        CellSize::Size(-0.1),
        CellSize::Size(f32::NAN),
    ] {
        assert_eq!(refuse(&cube, bad), RemeshError::BadCellSize, "{bad:?}");
    }
    let mut point = cube.clone();
    point.positions = vec![[0.5; 3]; point.positions.len()];
    let err = refuse(&point, CellSize::Resolution(8));
    assert_eq!(err, RemeshError::BadCellSize, "a mesh that is one point");
    let err = refuse(&cube, CellSize::Resolution(MAX_CELLS + 1));
    assert_eq!(err, RemeshError::TooManyCells, "resolution");
    let err = refuse(&cube, CellSize::Size(1.0 / 2049.0));
    assert_eq!(err, RemeshError::TooManyCells, "size");
}

#[test]
fn each_refusal_says_what_is_wrong() {
    let messages: Vec<String> = [
        RemeshError::NoFaces,
        RemeshError::BadFace(3),
        RemeshError::BadAttribute("m".into()),
        RemeshError::NotFinite,
        RemeshError::BadCellSize,
        RemeshError::TooManyCells,
        RemeshError::BadParts,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    let mut unique = messages.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), messages.len(), "distinct: {messages:?}");
    assert!(
        messages[1].contains('3') && messages[2].contains("'m'"),
        "{messages:?}"
    );
}

#[test]
fn the_cell_cap_is_inclusive() {
    // Two specks a unit apart: 2048 cells along x, but almost no surface, so this stays cheap.
    let speck = 0.002;
    let rod = merge(
        &cuboid([0.0; 3], [speck; 3], 1),
        &cuboid([1.0 - speck, 0.0, 0.0], [1.0, speck, speck], 1),
    );
    for cell in [
        CellSize::Resolution(MAX_CELLS),
        CellSize::Size(1.0 / 2048.0),
    ] {
        let d = signed_distance(&rod, cell).unwrap_or_else(|e| panic!("{cell:?}: {e}"));
        assert!(
            d.grid.samples()[0] >= MAX_CELLS,
            "{cell:?}: {:?}",
            d.grid.bricks
        );
    }
}
