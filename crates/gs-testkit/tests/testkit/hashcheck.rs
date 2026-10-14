//! The hasher check accepts a sound hash and rejects every realistic way a hash goes wrong.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, PlainMesh};
use gs_testkit::compare::{canonical, identical};
use gs_testkit::hashcheck::{check_hasher, variants};
use gs_testkit::measure::stats;
use gs_testkit::{Expect, validate};

use crate::data::tetra;

#[test]
fn the_canonical_form_passes_the_hasher_check() {
    let checked = check_hasher(&tetra(), canonical);
    assert!(
        checked.as_ref().is_ok_and(|&n| n > 1000),
        "non-trivial: {checked:?}"
    );
}

#[test]
fn every_documented_kind_of_variant_is_made() {
    let names: Vec<String> = variants(&tetra()).into_iter().map(|(n, _)| n).collect();
    for kind in [
        "bit 31 of vertex 3 axis 2",
        "bit 0 of face 0 slot 3",
        "bit 5 of attribute group value 3",
        "bit 1 of attribute mask value 0",
        "face 2 corners rotated",
        "face 3 winding reversed",
        "first and last faces swapped",
        "first and last vertices swapped",
        "last face dropped",
        "last vertex dropped",
        "attribute mask renamed",
        "attribute group moved to the other domain",
        "attribute mask retyped",
        "attribute group removed",
        "an empty attribute added",
    ] {
        assert!(names.iter().any(|n| n == kind), "missing: {kind}");
    }
    let flips = names.iter().filter(|n| n.starts_with("bit ")).count();
    // 12 coordinates, 16 face slots and 8 attribute values, at 32 bits each. The four
    // coordinates between 1 and 2 (-1.25 three times, 1.5 once) have exponent 127, one flip
    // of bit 30 away from infinity, so those four flips are skipped.
    assert_eq!(
        flips,
        32 * (12 + 16 + 8) - 4,
        "every bit, minus flips that are not finite"
    );
}

/// JSON cannot carry NaN or infinity, so no variant contains one, even where a single flip of
/// a float between 1 and 2 would make one.
#[test]
fn variants_never_hold_numbers_that_are_not_finite() {
    let mut base = tetra();
    if let Some(mask) = base.attributes.get_mut("mask") {
        mask.values = AttrValues::F32(vec![1.5, 1.25, -1.75, 1.0]);
    }
    for (what, mesh) in variants(&base) {
        let mut floats: Vec<f32> = mesh.positions.iter().flatten().copied().collect();
        for attribute in mesh.attributes.values() {
            if let AttrValues::F32(v) = &attribute.values {
                floats.extend(v);
            }
        }
        assert!(floats.iter().all(|v| v.is_finite()), "{what} is not finite");
    }
}

/// The variants are real changes, and the rewired ones keep the shape: only numbering and
/// layout differ, so a hash that sees them sees order, not just geometry.
#[test]
fn variants_differ_from_the_base_and_rewirings_keep_the_shape() {
    let base = tetra();
    let area = stats(&base).area;
    for (what, mesh) in variants(&base) {
        assert!(!identical(&mesh, &base), "{what} is not a change");
        if what.contains("rotated") || what.contains("swapped") {
            assert_eq!(
                validate(&mesh, Expect::closed(2)),
                Ok(()),
                "{what} is still a tetrahedron"
            );
            assert!(
                (stats(&mesh).volume - 1.0).abs() < 1e-12,
                "{what} keeps the volume"
            );
        }
        if what.contains("reversed") {
            assert_eq!(
                validate(&mesh, Expect::ANY),
                Ok(()),
                "{what} is still well formed"
            );
            assert!(
                (stats(&mesh).area - area).abs() < 1e-12,
                "{what} keeps the area"
            );
        }
    }
}

/// A hash that is right for vertices but ignores something else.
type Hash = fn(&PlainMesh) -> Vec<u8>;

fn without_faces(m: &PlainMesh) -> Vec<u8> {
    canonical(&PlainMesh {
        faces: Vec::new(),
        ..m.clone()
    })
}

fn without_sign_bits(m: &PlainMesh) -> Vec<u8> {
    let mut m = m.clone();
    m.positions.iter_mut().flatten().for_each(|c| *c = c.abs());
    canonical(&m)
}

fn order_blind(m: &PlainMesh) -> Vec<u8> {
    let mut words: Vec<u32> = m.positions.iter().flatten().map(|c| c.to_bits()).collect();
    words.extend(m.faces.iter().flatten());
    words.sort_unstable();
    let mut out: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    out.extend(canonical(&PlainMesh {
        attributes: m.attributes.clone(),
        ..PlainMesh::default()
    }));
    out
}

fn attribute_names_blind(m: &PlainMesh) -> Vec<u8> {
    let mut renamed = m.clone();
    renamed.attributes = m
        .attributes
        .values()
        .enumerate()
        .map(|(i, a)| (i.to_string(), a.clone()))
        .collect::<BTreeMap<_, _>>();
    canonical(&renamed)
}

fn attribute_types_blind(m: &PlainMesh) -> Vec<u8> {
    let mut m = m.clone();
    for a in m.attributes.values_mut() {
        if let AttrValues::F32(v) = &a.values {
            a.values = AttrValues::U32(v.iter().map(|x| x.to_bits()).collect());
        }
    }
    canonical(&m)
}

fn domain_blind(m: &PlainMesh) -> Vec<u8> {
    let mut m = m.clone();
    m.attributes
        .values_mut()
        .for_each(|a| a.domain = AttrDomain::Vertex);
    canonical(&m)
}

fn last_vertex_blind(m: &PlainMesh) -> Vec<u8> {
    let mut m = m.clone();
    m.positions.pop();
    canonical(&m)
}

#[test]
fn hashes_that_miss_something_are_rejected() {
    let bad: [(&str, Hash); 7] = [
        ("ignores faces", without_faces),
        ("ignores sign bits", without_sign_bits),
        ("ignores order", order_blind),
        ("ignores attribute names", attribute_names_blind),
        ("ignores attribute types", attribute_types_blind),
        ("ignores attribute domains", domain_blind),
        ("ignores the last vertex", last_vertex_blind),
    ];
    for (what, hash) in bad {
        let verdict = check_hasher(&tetra(), hash);
        assert!(verdict.is_err(), "a hash that {what} passed");
    }
    let constant = check_hasher(&tetra(), |_| 0);
    assert_eq!(
        constant,
        Err("bit 0 of vertex 0 axis 0 hashes the same as the base mesh".into()),
        "constant"
    );
    let mut calls = 0u64;
    let counter = check_hasher(&tetra(), |_| {
        calls += 1;
        calls
    });
    assert_eq!(
        counter,
        Err("the same mesh hashed differently twice".into()),
        "unstable"
    );
}
