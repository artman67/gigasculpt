//! What the open mesh formats ([`crate::obj`], [`crate::ply`], [`crate::stl`]) read into and
//! write from: named objects, each a [`PlainMesh`] with the columns the file could hold.
//!
//! Readers stream: they hold the mesh they build and one line or one record of the file, never
//! the whole file. They never panic on bad input; every error names the line (text) or the
//! byte offset (binary) where reading stopped. Faces come out as triangles and quads; a polygon
//! with more corners is split into a fan of triangles from a corner chosen by position, so the
//! two mirrored halves of a symmetric model split into mirrored triangles.
//!
//! Columns (the names the engine uses for them too): `group` (per face, `u32`), `color` (per
//! vertex, RGBA packed as `u32` with red in the low byte), `mask` (per vertex, `f32`, 1 is fully
//! masked) and the UVs, kept per face corner in the eight face columns [`UV_COLUMNS`] until the
//! mesh core has a corner domain.

mod build;
mod lines;

use std::collections::BTreeMap;

use gs_core::math::arity;
use gs_protocol::{AttrDomain, AttrValues, PlainMesh, face_triangles};

pub(crate) use build::Builder;
pub(crate) use lines::{Lines, finite, words};

pub use gs_protocol::{COLOR, GROUP, MASK};
/// The UV columns: the u and v of face corner `k` are columns `2k` and `2k + 1`, one value per
/// face (a triangle's corner 3 holds zeros).
pub const UV_COLUMNS: [&str; 8] = [
    "uv0.u", "uv0.v", "uv1.u", "uv1.v", "uv2.u", "uv2.v", "uv3.u", "uv3.v",
];

/// One object of a mesh file.
#[derive(Debug, Clone, Default)]
pub struct MeshObject {
    /// The name the file gives it (OBJ `o`, STL `solid`); empty when it has none.
    pub name: String,
    /// Positions as the file stores them, faces, and the columns the file holds.
    pub mesh: PlainMesh,
    /// Group names by group id, when the format names its groups (OBJ `g`).
    pub group_names: BTreeMap<u32, String>,
}

/// What a reader skipped or changed on the way, for the import report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notes {
    /// Polygons with more than four corners, each split into triangles.
    pub polygons_split: u64,
    /// What the file holds and the reader does not keep, with counts: normals, materials,
    /// unknown lines, PLY properties and elements, and so on.
    pub ignored: BTreeMap<String, u64>,
}

impl Notes {
    pub(crate) fn ignore(&mut self, what: impl Into<String>, count: u64) {
        *self.ignored.entry(what.into()).or_default() += count;
    }
}

/// A file read: its objects in file order, and the notes.
#[derive(Debug, Clone, Default)]
pub struct MeshFile {
    pub objects: Vec<MeshObject>,
    pub notes: Notes,
}

/// Choices a reader makes while it reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadOptions {
    /// OBJ: face groups from material names (`usemtl`) instead of group names (`g`).
    pub materials_as_groups: bool,
    /// Give each polygon with more than four corners a face group of its own before splitting
    /// it.
    pub group_polygons: bool,
}

/// One object to write: its name, the mesh, and the positions to write for it (the mesh's own,
/// or moved by the export options).
#[derive(Debug, Clone, Copy)]
pub struct Object<'a> {
    pub name: &'a str,
    pub mesh: &'a PlainMesh,
    pub positions: &'a [[f32; 3]],
}

/// Choices a writer makes. Formats ignore what they cannot hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent export switches, one per export option"
)]
pub struct WriteOptions {
    /// Split every quad into two triangles (STL always does).
    pub triangles: bool,
    /// Reverse every face's winding.
    pub flip_faces: bool,
    /// OBJ: write UVs when the mesh has them.
    pub uvs: bool,
    /// OBJ: write face groups as `g` lines when the mesh has them.
    pub groups: bool,
    /// OBJ and PLY: write vertex colors when the mesh has them.
    pub colors: bool,
    /// OBJ: write the mask (and colors) as `#MRGB` lines, the form sculpting apps read.
    pub mask: bool,
    /// PLY and STL: binary (little endian) rather than ASCII.
    pub binary: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            triangles: false,
            flip_faces: false,
            uvs: true,
            groups: true,
            colors: true,
            mask: true,
            binary: true,
        }
    }
}

/// A number type a column holds: `f32` or `u32`.
pub trait Number: Sized {
    /// The values, if `values` holds this type.
    fn values(values: &AttrValues) -> Option<&[Self]>;
}

impl Number for f32 {
    fn values(values: &AttrValues) -> Option<&[f32]> {
        if let AttrValues::F32(v) = values {
            Some(v)
        } else {
            None
        }
    }
}

impl Number for u32 {
    fn values(values: &AttrValues) -> Option<&[u32]> {
        if let AttrValues::U32(v) = values {
            Some(v)
        } else {
            None
        }
    }
}

/// A mesh's column by that name, domain and number type, if it has one.
pub fn column<'a, T: Number>(
    mesh: &'a PlainMesh,
    name: &str,
    domain: AttrDomain,
) -> Option<&'a [T]> {
    let a = mesh.attributes.get(name).filter(|a| a.domain == domain)?;
    T::values(&a.values)
}

/// A mesh's UVs, read from its eight UV columns when it has all of them, one per face.
#[derive(Debug, Clone, Copy)]
pub struct Uvs<'a>([&'a [f32]; 8]);

impl<'a> Uvs<'a> {
    pub fn of(mesh: &'a PlainMesh) -> Option<Self> {
        let mut cols: [&[f32]; 8] = [&[]; 8];
        for (col, name) in cols.iter_mut().zip(UV_COLUMNS) {
            *col = column(mesh, name, AttrDomain::Face).filter(|c| c.len() == mesh.faces.len())?;
        }
        Some(Self(cols))
    }

    /// The UVs of face `f`'s four corners.
    pub fn face(&self, f: usize) -> [[f32; 2]; 4] {
        std::array::from_fn(|k| [self.0[2 * k][f], self.0[2 * k + 1][f]])
    }
}

/// The faces to write for `face`: itself, or two triangles when quads are split, as the quad
/// is drawn ([`face_triangles`], by `positions`, the mesh's own). Corners are indices into
/// `face` (so per corner values follow), turned the other way after the first corner when
/// faces are flipped; a second face that is not there is all `None`.
pub(crate) fn split(
    face: &[u32; 4],
    positions: &[[f32; 3]],
    options: WriteOptions,
) -> [[Option<usize>; 4]; 2] {
    let mut out = [[None; 4]; 2];
    let put = |slot: &mut [Option<usize>; 4], part: &[usize]| {
        for (s, &k) in slot.iter_mut().zip(part) {
            *s = Some(k);
        }
        if options.flip_faces {
            slot[1..part.len()].reverse();
        }
    };
    if arity(*face) == 4 && !options.triangles {
        put(&mut out[0], &[0, 1, 2, 3]);
    } else {
        let tris = face_triangles(*face, |k| positions[face[k] as usize]);
        for (slot, t) in out.iter_mut().zip(tris) {
            put(slot, t);
        }
    }
    out
}
