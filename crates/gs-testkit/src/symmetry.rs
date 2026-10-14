//! Metamorphic helpers: transform a mesh and a command's arguments together, then compare the
//! results (research 06, section 3, relations M1 and M5).
//!
//! - [`CubeSymmetry`]: the 48 symmetries of the cube (permute the axes, flip their signs).
//!   They are exact in floating point, so a command that claims to commute with them can be
//!   held to a tight tolerance.
//! - [`Renumbering`]: shuffle the order of vertices and faces without changing the shape, and
//!   put it back.

use std::ops::Neg;

use gs_protocol::{AttrDomain, AttrValues, PlainMesh};
use serde_json::Value;

use crate::SplitMix64;
use crate::measure::{corners, cross, dot};

/// One of the 48 symmetries of the cube: output axis `i` is input axis `axes[i]`, negated when
/// `flips[i]` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CubeSymmetry {
    /// Which input axis feeds each output axis.
    pub axes: [usize; 3],
    /// Which output axes are negated.
    pub flips: [bool; 3],
}

impl CubeSymmetry {
    /// All 48, the identity first.
    pub fn all() -> Vec<Self> {
        let perms = [
            [0, 1, 2],
            [1, 2, 0],
            [2, 0, 1],
            [0, 2, 1],
            [2, 1, 0],
            [1, 0, 2],
        ];
        let mut out = Vec::new();
        for axes in perms {
            for bits in 0..8 {
                let flips = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
                out.push(Self { axes, flips });
            }
        }
        out
    }

    /// The symmetry applied to a point or vector. Exact: it only moves and negates numbers.
    pub fn apply<T: Copy + Neg<Output = T>>(self, p: [T; 3]) -> [T; 3] {
        std::array::from_fn(|i| {
            let v = p[self.axes[i]];
            if self.flips[i] { -v } else { v }
        })
    }

    /// The symmetry that undoes this one.
    #[must_use]
    pub fn inverse(self) -> Self {
        let mut inverse = self;
        for i in 0..3 {
            inverse.axes[self.axes[i]] = i;
            inverse.flips[self.axes[i]] = self.flips[i];
        }
        inverse
    }

    /// True for the 24 symmetries that mirror (determinant -1). They turn a mesh inside out
    /// unless every face's winding is reversed too.
    pub fn is_reflection(self) -> bool {
        let [x, y, z] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].map(|e| self.apply(e));
        dot(x, cross(y, z)) <= 0.0
    }

    /// The symmetry applied to a mesh: every position moved, and for a reflection every face's
    /// winding reversed, so the outside stays outside.
    pub fn mesh(self, mesh: &PlainMesh) -> PlainMesh {
        let mut out = mesh.clone();
        for p in &mut out.positions {
            *p = self.apply(*p);
        }
        if self.is_reflection() {
            for face in &mut out.faces {
                let n = corners(face).len();
                face[..n].reverse();
            }
        }
        out
    }

    /// The symmetry applied to the vectors in a command's arguments. Each pointer is a JSON
    /// pointer, where a `*` step means every element of an array (`/samples/*/position`).
    /// Values that are not three numbers are left alone.
    pub fn args(self, args: &Value, vectors: &[String]) -> Value {
        let mut out = args.clone();
        for pointer in vectors {
            for value in select(&mut out, pointer) {
                let numbers: Option<Vec<f64>> = value
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).collect());
                if let Some(&[x, y, z]) = numbers.as_deref() {
                    *value = self.apply([x, y, z]).into();
                }
            }
        }
        out
    }
}

/// Every value a JSON pointer reaches, where a `*` step means every element of an array.
pub fn select<'a>(value: &'a mut Value, pointer: &str) -> Vec<&'a mut Value> {
    let mut found = vec![value];
    for step in pointer.split('/').skip(1) {
        found = found
            .into_iter()
            .flat_map(|v| -> Vec<&mut Value> {
                match (step, v) {
                    ("*", Value::Array(items)) => items.iter_mut().collect(),
                    (_, v) => v.pointer_mut(&format!("/{step}")).into_iter().collect(),
                }
            })
            .collect();
    }
    found
}

/// A new order for a mesh's vertices and faces. Entry `new` of each list is the old index.
#[derive(Clone, Debug)]
pub struct Renumbering {
    vertices: Vec<usize>,
    faces: Vec<usize>,
}

impl Renumbering {
    /// A uniformly random order for `mesh`, from `seed`.
    pub fn random(mesh: &PlainMesh, seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed);
        let mut vertices: Vec<usize> = (0..mesh.positions.len()).collect();
        let mut faces: Vec<usize> = (0..mesh.faces.len()).collect();
        rng.shuffle(&mut vertices);
        rng.shuffle(&mut faces);
        Self { vertices, faces }
    }

    /// `mesh` in the new order: same shape, different numbers.
    pub fn apply(&self, mesh: &PlainMesh) -> PlainMesh {
        permute(mesh, &self.vertices, &self.faces)
    }

    /// Puts a mesh in the new order back into the old one.
    pub fn undo(&self, mesh: &PlainMesh) -> PlainMesh {
        permute(mesh, &inverse(&self.vertices), &inverse(&self.faces))
    }
}

fn inverse(order: &[usize]) -> Vec<usize> {
    let mut out = vec![0; order.len()];
    for (new, &old) in order.iter().enumerate() {
        out[old] = new;
    }
    out
}

/// `mesh` with vertex `new` taken from `vertices[new]` and face `new` from `faces[new]`,
/// faces renumbered to match, and attributes moved with their elements.
fn permute(mesh: &PlainMesh, vertices: &[usize], faces: &[usize]) -> PlainMesh {
    let new_index = inverse(vertices);
    let mut out = mesh.clone();
    out.positions = vertices.iter().map(|&old| mesh.positions[old]).collect();
    out.faces = faces
        .iter()
        .map(|&old| {
            let mut face = mesh.faces[old];
            let n = corners(&face).len();
            face[..n]
                .iter_mut()
                .for_each(|v| *v = new_index[*v as usize] as u32);
            face
        })
        .collect();
    for attribute in out.attributes.values_mut() {
        let order = match attribute.domain {
            AttrDomain::Vertex => vertices,
            AttrDomain::Face => faces,
        };
        attribute.values = match &attribute.values {
            AttrValues::F32(v) => AttrValues::F32(order.iter().map(|&old| v[old]).collect()),
            AttrValues::U32(v) => AttrValues::U32(order.iter().map(|&old| v[old]).collect()),
        };
    }
    out
}
