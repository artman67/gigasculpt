//! The hasher check. A hash that tests use to say "bit-identical" must change for every
//! single-bit change and every change of connectivity or attribute layout. Research 06 found
//! that replacing the toy's hash with a constant passed every property test until this check
//! existed.
//!
//! [`check_hasher`] works on any hash function: the engine's `mesh.hash` (through a session),
//! or the test kit's own [`crate::compare::canonical`].

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::measure::corners;

/// Checks `hash` against `base` and every variant of it: the same mesh must hash the same
/// twice, and the base and all variants must hash pairwise differently. Returns how many
/// variants were checked, or which two meshes collided.
pub fn check_hasher<H: Ord>(
    base: &PlainMesh,
    mut hash: impl FnMut(&PlainMesh) -> H,
) -> Result<usize, String> {
    let first = hash(base);
    if hash(&base.clone()) != first {
        return Err("the same mesh hashed differently twice".into());
    }
    let mut seen: BTreeMap<H, String> = BTreeMap::new();
    seen.insert(first, "the base mesh".into());
    let all = variants(base);
    for (what, mesh) in &all {
        if let Some(other) = seen.insert(hash(mesh), what.clone()) {
            return Err(format!("{what} hashes the same as {other}"));
        }
    }
    Ok(all.len())
}

/// Every mesh that differs from `base` in one bit of one number (skipping floats that would
/// stop being finite, which JSON cannot carry), or by one change of connectivity or attribute
/// layout that keeps every number the same: corners rotated, winding reversed, two faces or
/// two vertices swapped, the last face or vertex dropped, an attribute renamed, moved to the
/// other domain, retyped with the same bits, removed, or an empty one added.
pub fn variants(base: &PlainMesh) -> Vec<(String, PlainMesh)> {
    let mut out = bit_flips(base);
    out.extend(rewirings(base));
    out.extend(relabelings(base));
    out
}

fn bit_flips(base: &PlainMesh) -> Vec<(String, PlainMesh)> {
    let mut out = Vec::new();
    for bit in 0..32 {
        let flip = |v: f32| f32::from_bits(v.to_bits() ^ (1 << bit));
        for (v, axis) in (0..base.positions.len()).flat_map(|v| (0..3).map(move |a| (v, a))) {
            let mut m = base.clone();
            m.positions[v][axis] = flip(m.positions[v][axis]);
            if m.positions[v][axis].is_finite() {
                out.push((format!("bit {bit} of vertex {v} axis {axis}"), m));
            }
        }
        for (f, slot) in (0..base.faces.len()).flat_map(|f| (0..4).map(move |s| (f, s))) {
            let mut m = base.clone();
            m.faces[f][slot] ^= 1 << bit;
            out.push((format!("bit {bit} of face {f} slot {slot}"), m));
        }
        for (name, attribute) in &base.attributes {
            for i in 0..values_len(&attribute.values) {
                let mut m = base.clone();
                let values = m.attributes.get_mut(name).map(|a| &mut a.values);
                match values {
                    Some(AttrValues::F32(v)) if flip(v[i]).is_finite() => v[i] = flip(v[i]),
                    Some(AttrValues::U32(v)) => v[i] ^= 1 << bit,
                    _ => continue,
                }
                out.push((format!("bit {bit} of attribute {name} value {i}"), m));
            }
        }
    }
    out
}

/// Reorders a face's corners in place.
type Reorder = fn(&mut [u32]);

fn rewirings(base: &PlainMesh) -> Vec<(String, PlainMesh)> {
    let mut out = Vec::new();
    let edit = |change: &dyn Fn(&mut PlainMesh)| {
        let mut m = base.clone();
        change(&mut m);
        m
    };
    let reorders: [(&str, Reorder); 2] = [
        ("corners rotated", |c| c.rotate_left(1)),
        ("winding reversed", <[u32]>::reverse),
    ];
    for f in 0..base.faces.len() {
        for (what, reorder) in reorders {
            let changed = edit(&|m| {
                let n = corners(&m.faces[f]).len();
                reorder(&mut m.faces[f][..n]);
            });
            out.push((format!("face {f} {what}"), changed));
        }
    }
    if let (Some(first), Some(last)) = (base.faces.first(), base.faces.last())
        && first != last
    {
        let swapped = edit(&|m| m.faces.swap(0, base.faces.len() - 1));
        out.push(("first and last faces swapped".into(), swapped));
    }
    if let (Some(first), Some(last)) = (base.positions.first(), base.positions.last())
        && first.map(f32::to_bits) != last.map(f32::to_bits)
    {
        out.push((
            "first and last vertices swapped".into(),
            swap_vertices(base),
        ));
    }
    if !base.faces.is_empty() {
        out.push((
            "last face dropped".into(),
            edit(&|m| {
                m.faces.pop();
            }),
        ));
    }
    if !base.positions.is_empty() {
        let dropped = edit(&|m| {
            m.positions.pop();
        });
        out.push(("last vertex dropped".into(), dropped));
    }
    out
}

/// `base` with its first and last vertices swapped and every face renumbered to match, so
/// the shape is the same and only the numbering differs.
fn swap_vertices(base: &PlainMesh) -> PlainMesh {
    let mut m = base.clone();
    let last = m.positions.len() - 1;
    m.positions.swap(0, last);
    let renumber = |v: u32| match v as usize {
        0 => last as u32,
        v if v == last => 0,
        _ => v,
    };
    for face in &mut m.faces {
        let n = corners(face).len();
        face[..n].iter_mut().for_each(|v| *v = renumber(*v));
    }
    m
}

fn relabelings(base: &PlainMesh) -> Vec<(String, PlainMesh)> {
    let mut out = Vec::new();
    for (name, attribute) in &base.attributes {
        let with = |key: String, attribute: Option<Attribute>| {
            let mut m = base.clone();
            m.attributes.remove(name);
            m.attributes.extend(attribute.map(|a| (key, a)));
            m
        };
        let domain = match attribute.domain {
            AttrDomain::Vertex => AttrDomain::Face,
            AttrDomain::Face => AttrDomain::Vertex,
        };
        let values = match &attribute.values {
            AttrValues::F32(v) => AttrValues::U32(v.iter().map(|x| x.to_bits()).collect()),
            AttrValues::U32(v) => AttrValues::F32(v.iter().map(|&x| f32::from_bits(x)).collect()),
        };
        let renamed = with(format!("{name}_"), Some(attribute.clone()));
        let moved = with(
            name.clone(),
            Some(Attribute {
                domain,
                ..attribute.clone()
            }),
        );
        let domain = attribute.domain;
        let retyped = with(name.clone(), Some(Attribute { domain, values }));
        out.push((format!("attribute {name} renamed"), renamed));
        out.push((format!("attribute {name} moved to the other domain"), moved));
        out.push((format!("attribute {name} retyped"), retyped));
        out.push((
            format!("attribute {name} removed"),
            with(name.clone(), None),
        ));
    }
    let mut extra = base.clone();
    let empty = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(Vec::new()),
    };
    extra.attributes.insert("zz_empty".into(), empty);
    out.push(("an empty attribute added".into(), extra));
    out
}

fn values_len(values: &AttrValues) -> usize {
    match values {
        AttrValues::F32(v) => v.len(),
        AttrValues::U32(v) => v.len(),
    }
}
