//! A Part's UV hint columns as a scene file holds them (decision 0061, amended 2026-10-13, step
//! UV7): a hint a hand-edited file holds out of range, off the grid or as not a number loads
//! settled, and the mask loads exactly as saved.

use gs_core::{AttrDef, AttrKind, AttrVec, ColumnSet, Domain, MeshStore};
use gs_io::scene::{Kept, read_bytes};

use crate::sample::{file, store};

/// Steps per unit of the 2^-24 grid.
const STEP: f32 = 16_777_216.0;

/// What a hand-edited file holds in a hint column, and what each value loads as.
const EDITS: [(f32, f32); 8] = [
    (3.0, 1.0),
    (-7.5, -1.0),
    (f32::NAN, 0.0),
    (f32::INFINITY, 1.0),
    (-0.0, 0.0),
    (0.2, 3_355_443.0 / STEP),
    (-0.25, -0.25),
    (1.0 / STEP, 1.0 / STEP),
];

/// Writes `values` over and over into every node's `def` column of `s`, as no writer would.
fn fill(s: &mut MeshStore, def: AttrDef, values: &[f32]) {
    let a = s.add_attr(def).expect("attribute");
    let nodes: Vec<u32> = (0..s.node_count() as u32).collect();
    s.write(&nodes, ColumnSet::attr(a)).each(|node| {
        if let AttrVec::F32(v) = node.attr(a) {
            for (l, x) in v.iter_mut().enumerate() {
                *x = values[l % values.len()];
            }
        }
    });
}

#[test]
fn a_hint_out_of_range_or_not_a_number_loads_settled_and_the_mask_as_saved() {
    let mut s = store(6, 1);
    let (saved, loads): (Vec<f32>, Vec<f32>) = EDITS.into_iter().unzip();
    for name in ["uv_hint", "uv_density"] {
        fill(
            &mut s,
            AttrDef::new(name, Domain::Vertex, AttrKind::Scalar),
            &saved,
        );
    }
    let mask = [1.5, 0.2, -0.5];
    fill(&mut s, AttrDef::mask(), &mask);
    let scene = read_bytes(&file(&[s], None), Kept::None).expect("the file reads");
    let part = scene.parts.into_iter().next().expect("one part");
    let part = part.expect("the part reads");
    let read = MeshStore::from_exact(part.config, part.attrs, part.nodes).expect("a store");
    for (name, want) in [
        ("uv_hint", &loads[..]),
        ("uv_density", &loads),
        ("mask", &mask),
    ] {
        let a = read.attr_id(name).expect("the column");
        for k in 0..read.node_count() as u32 {
            let Some(AttrVec::F32(v)) = read.node(k).attr(a) else {
                panic!("node {k} holds {name}");
            };
            for (l, x) in v.iter().enumerate() {
                let w = want[l % want.len()];
                assert_eq!(x.to_bits(), w.to_bits(), "{name}: node {k} point {l} {x}");
            }
        }
    }
}
