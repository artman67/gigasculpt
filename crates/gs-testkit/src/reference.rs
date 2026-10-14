//! Slow, independent references for what the engine reports about an edit, ([`stroke`]) for
//! one dab of each brush kernel, and ([`alpha`]) for reading an alpha. They share data with the
//! engine, never code, and are written for clarity rather than speed.

pub mod alpha;
pub mod shading;
pub mod stroke;
pub mod tonemap;

use std::collections::BTreeSet;

use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};

use crate::measure::bounds;
use crate::session::Scene;

/// What an edit should report, worked out from full copies of every part before and after.
#[derive(Debug, PartialEq)]
pub struct Effect {
    /// Parts that changed, sorted.
    pub parts: Vec<String>,
    /// True if any part was added, removed, or had its faces or vertex count changed.
    pub topology_changed: bool,
    /// Vertices touched: moved, or with a changed value in a vertex column, or a corner of a
    /// face with a changed value in a face column; every vertex of a part whose topology
    /// changed, or whose columns were added, removed or reshaped.
    pub verts_touched: u64,
    /// Bounds of those vertices, at their new positions (old ones for a removed part).
    pub bbox: Option<[[f32; 3]; 2]>,
}

/// The effect of going from `before` to `after`.
pub fn effect(before: &Scene, after: &Scene) -> Effect {
    let names: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut parts = Vec::new();
    let mut topology_changed = false;
    let mut touched: Vec<[f32; 3]> = Vec::new();
    for name in names {
        let (old, new) = (before.get(name), after.get(name));
        let hit: Vec<[f32; 3]> = match (old, new) {
            (Some(old), Some(new))
                if old.faces == new.faces && old.positions.len() == new.positions.len() =>
            {
                let marked = touched_in_place(old, new);
                (new.positions.iter().zip(marked))
                    .filter(|(_, hit)| *hit)
                    .map(|(p, _)| *p)
                    .collect()
            }
            _ => {
                topology_changed = true;
                parts.push(name.clone());
                touched.extend(new.or(old).into_iter().flat_map(|m| m.positions.clone()));
                continue;
            }
        };
        if !hit.is_empty() {
            parts.push(name.clone());
        }
        touched.extend(hit);
    }
    Effect {
        parts,
        topology_changed,
        verts_touched: touched.len() as u64,
        bbox: bounds(&touched),
    }
}

/// For a part with the same faces and vertex count before and after: whether each vertex was
/// touched.
fn touched_in_place(old: &PlainMesh, new: &PlainMesh) -> Vec<bool> {
    let n = new.positions.len();
    let names: BTreeSet<&String> = old.attributes.keys().chain(new.attributes.keys()).collect();
    let reshaped = names.iter().any(|name| {
        let shape = |m: &PlainMesh| m.attributes.get(*name).map(shape_of);
        shape(old) != shape(new)
    });
    if reshaped {
        return vec![true; n];
    }
    let mut hit: Vec<bool> = (0..n)
        .map(|v| bits(old.positions[v]) != bits(new.positions[v]))
        .collect();
    for name in names {
        let (a, b) = (&old.attributes[name], &new.attributes[name]);
        for i in 0..shape_of(a).2 {
            if word(&a.values, i) == word(&b.values, i) {
                continue;
            }
            match a.domain {
                AttrDomain::Vertex => hit[i] = true,
                AttrDomain::Face => {
                    for &v in new.faces[i].iter().filter(|&&v| v != NO_VERTEX) {
                        hit[v as usize] = true;
                    }
                }
            }
        }
    }
    hit
}

/// A column's domain, value type and length.
fn shape_of(a: &Attribute) -> (AttrDomain, bool, usize) {
    match &a.values {
        AttrValues::F32(v) => (a.domain, true, v.len()),
        AttrValues::U32(v) => (a.domain, false, v.len()),
    }
}

/// Value `i` of a column as its raw 32-bit word.
fn word(values: &AttrValues, i: usize) -> u32 {
    match values {
        AttrValues::F32(v) => v[i].to_bits(),
        AttrValues::U32(v) => v[i],
    }
}

fn bits(p: [f32; 3]) -> [u32; 3] {
    p.map(f32::to_bits)
}
