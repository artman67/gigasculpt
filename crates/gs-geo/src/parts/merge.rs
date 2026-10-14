//! Joining meshes into one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gs_protocol::{AttrDomain, AttrValues, Attribute, GROUP, NO_VERTEX, PlainMesh};

/// Why meshes could not be merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    /// Two inputs hold a column of this name with different domains or value types.
    Column(String),
    /// Moving a mesh's face group ids up past those taken would pass `u32::MAX`.
    Groups,
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Column(name) => write!(
                f,
                "two of the meshes hold a column '{name}' of different kinds (per vertex and \
                 per face, or numbers and whole numbers)"
            ),
            Self::Groups => write!(f, "the face group ids are too large to keep apart"),
        }
    }
}

impl std::error::Error for MergeError {}

/// `meshes` as one mesh: their vertices and faces one after the other, in the order given.
///
/// A column any input has is in the result. Where an input lacks it (or has fewer values than
/// elements) it reads `empty(name, column)`, as raw bits (an `f32`'s bit pattern or a `u32`),
/// given the column as the first input holding it has it: the caller knows each column's empty
/// value (white for a color, 0 for the rest). Face groups (the face column `group`) keep their
/// ids unless they collide: an input whose non-zero ids meet ids already taken has every
/// non-zero id moved up by the largest id so far, so split then merge gives back the original
/// ids.
pub fn merge(
    meshes: &[&PlainMesh],
    empty: impl Fn(&str, &Attribute) -> u32,
) -> Result<PlainMesh, MergeError> {
    let mut kinds: BTreeMap<&str, (&Attribute, bool)> = BTreeMap::new();
    for mesh in meshes {
        for (name, column) in &mesh.attributes {
            let float = matches!(column.values, AttrValues::F32(_));
            let (first, f) = *kinds.entry(name).or_insert((column, float));
            if (first.domain, f) != (column.domain, float) {
                return Err(MergeError::Column(name.clone()));
            }
        }
    }
    let shifts = group_shifts(meshes)?;
    let mut out = PlainMesh::default();
    for mesh in meshes {
        let base = out.positions.len() as u32;
        let shift = |v: u32| if v == NO_VERTEX { v } else { v + base };
        out.positions.extend_from_slice(&mesh.positions);
        out.faces
            .extend(mesh.faces.iter().map(|face| face.map(shift)));
    }
    for (name, (first, float)) in kinds {
        let domain = first.domain;
        let mut all = joined(meshes, name, domain, empty(name, first));
        let values = if float {
            AttrValues::F32(all.into_iter().map(f32::from_bits).collect())
        } else {
            if name == GROUP && domain == AttrDomain::Face {
                let mut at = 0;
                for (mesh, &by) in meshes.iter().zip(&shifts) {
                    for id in &mut all[at..at + mesh.faces.len()] {
                        if *id != 0 {
                            *id += by;
                        }
                    }
                    at += mesh.faces.len();
                }
            }
            AttrValues::U32(all)
        };
        out.attributes
            .insert(name.to_string(), Attribute { domain, values });
    }
    Ok(out)
}

/// Column `name` of every mesh as raw bits, one after the other, each padded with `pad` or cut
/// to its element count.
fn joined(meshes: &[&PlainMesh], name: &str, domain: AttrDomain, pad: u32) -> Vec<u32> {
    let mut all = Vec::new();
    for mesh in meshes {
        let count = match domain {
            AttrDomain::Vertex => mesh.positions.len(),
            AttrDomain::Face => mesh.faces.len(),
        };
        let bits = |i: usize| match mesh.attributes.get(name).map(|c| &c.values) {
            Some(AttrValues::F32(v)) => v.get(i).map(|x| x.to_bits()),
            Some(AttrValues::U32(v)) => v.get(i).copied(),
            None => None,
        };
        all.extend((0..count).map(|i| bits(i).unwrap_or(pad)));
    }
    all
}

/// How far each mesh's non-zero face group ids move: 0, or the largest id taken before it when
/// its ids meet one of those.
fn group_shifts(meshes: &[&PlainMesh]) -> Result<Vec<u32>, MergeError> {
    let mut taken = BTreeSet::new();
    let mut shifts = Vec::with_capacity(meshes.len());
    for mesh in meshes {
        let ids: BTreeSet<u32> = match mesh.attributes.get(GROUP) {
            Some(Attribute {
                domain: AttrDomain::Face,
                values: AttrValues::U32(v),
            }) => (v.iter().take(mesh.faces.len()).copied())
                .filter(|&id| id != 0)
                .collect(),
            _ => BTreeSet::new(),
        };
        let by = if ids.is_disjoint(&taken) {
            0
        } else {
            taken.last().copied().unwrap_or(0)
        };
        for id in ids {
            taken.insert(id.checked_add(by).ok_or(MergeError::Groups)?);
        }
        shifts.push(by);
    }
    Ok(shifts)
}
