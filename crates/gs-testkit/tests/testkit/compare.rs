//! The comparer never lets NaN, a length mismatch or a sign of zero pass as "close".

use std::collections::BTreeMap;

use gs_protocol::{AttrValues, PlainMesh};
use gs_testkit::compare::{
    Observed, canonical, identical, max_abs_diff, scene_bytes, vertex_diff, worst,
};

use crate::data::tetra;

const INF: f64 = f64::INFINITY;

#[test]
fn the_worst_of_two_differences_counts_nan_as_infinite() {
    assert_eq!(worst(1.0, 2.0), 2.0, "larger second");
    assert_eq!(worst(3.0, 2.0), 3.0, "larger first");
    assert_eq!(worst(f64::NAN, 0.0), INF, "NaN first");
    assert_eq!(worst(0.0, f64::NAN), INF, "NaN second");
}

#[test]
fn number_lists_compare_by_their_largest_difference() {
    assert_eq!(
        max_abs_diff(&[1.0, 2.0, 3.0], &[1.0, 2.5, 3.0]),
        0.5,
        "one differs"
    );
    assert_eq!(
        max_abs_diff(&[-1.0, 4.0], &[1.0, 3.0]),
        2.0,
        "absolute, largest"
    );
    assert_eq!(max_abs_diff(&[], &[]), 0.0, "nothing to compare");
    assert_eq!(max_abs_diff(&[1.0], &[1.0, 2.0]), INF, "different lengths");
    assert_eq!(
        max_abs_diff(&[f32::NAN, 1.0], &[0.0, 1.0]),
        INF,
        "NaN on the left"
    );
    assert_eq!(
        max_abs_diff(&[1.0, 0.0], &[1.0, f32::NAN]),
        INF,
        "NaN on the right"
    );
    assert_eq!(
        max_abs_diff(&[f32::INFINITY], &[f32::INFINITY]),
        INF,
        "infinities"
    );
}

#[test]
fn vertex_differences_name_the_worst_vertex() {
    let base = tetra();
    let mut moved = base.clone();
    moved.positions[2][1] += 0.25;
    moved.positions[1][0] -= 0.125;
    let diff = vertex_diff(&base, &moved);
    assert_eq!(
        (diff.max, diff.vertex),
        (0.25, Some(2)),
        "vertex 2 moved most"
    );
    let same = vertex_diff(&base, &base);
    assert_eq!(
        (same.max, same.vertex),
        (0.0, Some(3)),
        "ties go to the last vertex"
    );
    let mut broken = base.clone();
    broken.positions[0][2] = f32::NAN;
    let diff = vertex_diff(&base, &broken);
    assert_eq!(
        (diff.max, diff.vertex),
        (INF, Some(0)),
        "NaN is infinitely far"
    );
    let mut fewer = base.clone();
    fewer.positions.pop();
    let diff = vertex_diff(&base, &fewer);
    assert_eq!(
        (diff.max, diff.vertex),
        (INF, None),
        "different vertex counts"
    );
    let empty = vertex_diff(&PlainMesh::default(), &PlainMesh::default());
    assert_eq!((empty.max, empty.vertex), (0.0, None), "no vertices");
}

#[test]
fn tolerances_are_ten_times_the_worst_observed() {
    let mut observed = Observed::default();
    assert_eq!(observed.tolerance(), 0.0, "nothing observed: exact");
    observed.note(2.0e-7);
    observed.note(1.0e-7);
    assert_eq!(observed.max, 2.0e-7, "keeps the largest");
    assert!(
        (observed.tolerance() - 2.0e-6).abs() < 1e-18,
        "ten times it"
    );
    observed.note(f64::NAN);
    assert_eq!(observed.tolerance(), INF, "a NaN poisons it");
}

#[test]
fn identical_means_bit_identical() {
    let base = tetra();
    assert!(identical(&base, &tetra()), "equal data");
    let mut zero = base.clone();
    zero.positions[0] = [0.0, 0.0, 0.0];
    let mut negative_zero = zero.clone();
    negative_zero.positions[0][1] = -0.0;
    assert!(!identical(&zero, &negative_zero), "-0.0 is not 0.0");
    let mut retyped = base.clone();
    if let Some(mask) = retyped.attributes.get_mut("mask")
        && let AttrValues::F32(values) = &mask.values
    {
        mask.values = AttrValues::U32(values.iter().map(|v| v.to_bits()).collect());
    }
    assert!(!identical(&base, &retyped), "same bits, different type");
    assert_eq!(
        canonical(&base),
        canonical(&tetra()),
        "canonical bytes are stable"
    );
}

#[test]
fn scenes_compare_by_names_and_contents() {
    let scene = |parts: &[(&str, PlainMesh)]| -> BTreeMap<String, PlainMesh> {
        parts
            .iter()
            .map(|(n, m)| ((*n).to_string(), m.clone()))
            .collect()
    };
    let mut other = tetra();
    other.positions[0][0] = 9.0;
    let all = [
        scene_bytes(&scene(&[])),
        scene_bytes(&scene(&[("a", tetra())])),
        scene_bytes(&scene(&[("b", tetra())])),
        scene_bytes(&scene(&[("a", tetra()), ("b", other.clone())])),
        scene_bytes(&scene(&[("a", other), ("b", tetra())])),
    ];
    for (i, x) in all.iter().enumerate() {
        for y in &all[i + 1..] {
            assert_ne!(x, y, "scene {i} matches a different scene");
        }
    }
    assert_eq!(
        all[1],
        scene_bytes(&scene(&[("a", tetra())])),
        "same scene, same bytes"
    );
}

/// Every list is counted, so the same numbers split differently between lists never compare
/// equal.
#[test]
fn the_canonical_form_counts_every_list() {
    let words: Vec<f32> = (1..=12u32).map(f32::from_bits).collect();
    let as_positions = PlainMesh {
        positions: words.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
        ..PlainMesh::default()
    };
    let as_faces = PlainMesh {
        faces: vec![[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]],
        ..PlainMesh::default()
    };
    assert!(!identical(&as_positions, &as_faces), "positions or faces");
    let split = |a: Vec<u32>, b: Vec<u32>| {
        let mut m = PlainMesh::default();
        for (name, values) in [("a", a), ("b", b)] {
            let values = AttrValues::U32(values);
            let domain = gs_protocol::AttrDomain::Vertex;
            m.attributes
                .insert(name.into(), gs_protocol::Attribute { domain, values });
        }
        m
    };
    assert!(
        !identical(&split(vec![1, 2], vec![3]), &split(vec![1], vec![2, 3])),
        "values split differently between two attributes"
    );
}
