//! The plain interchange mesh.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Marks the unused fourth slot of a triangle in `PlainMesh::faces`.
pub const NO_VERTEX: u32 = u32::MAX;

/// The face group column of a `PlainMesh`.
pub const GROUP: &str = "group";
/// The vertex color column.
pub const COLOR: &str = "color";
/// The vertex mask column.
pub const MASK: &str = "mask";
/// The hidden faces column, 1 for a hidden face.
pub const HIDE: &str = "hide";

/// Corners in a face: 3 for a triangle ([`NO_VERTEX`] in the fourth slot), 4 for a quad.
#[inline]
pub fn arity(face: [u32; 4]) -> usize {
    if face[3] == NO_VERTEX { 3 } else { 4 }
}

/// The corners of a face: three for a triangle, four for a quad. Only the fourth slot decides,
/// so a [`NO_VERTEX`] in an earlier slot stays among the corners for a check to find.
#[inline]
pub fn corners(face: &[u32; 4]) -> &[u32] {
    &face[..arity(*face)]
}

/// Which diagonal a quad with corners at `p` is split along, wherever a quad is split: drawn,
/// picked, measured or written out (decision 0040, section 2). 0 for the one from corner 0 to
/// corner 2, 1 for the one from corner 1 to corner 3.
///
/// The shorter one, comparing squared lengths in f32. When the two are exactly as long, the one
/// through the corner that comes first by `|x|`, then `|y|`, `|z|`, `x`, `y`, `z`, with `p`
/// part-local. Mirroring the quad across x, y or z = 0, or starting it at another corner,
/// changes none of these comparisons (rounding included, and -0.0 equals 0.0), so mirror-image
/// quads split along mirror-image diagonals, bit for bit. A quad that crosses a mirror plane is
/// its own image, and no rule splits it like one. Two corners that compare alike in every way
/// sit at one point, and then both splits make the same surface; the first such corner decides.
pub fn quad_diagonal(p: [[f32; 3]; 4]) -> usize {
    let length = |[a, b]: [[f32; 3]; 2]| {
        let [x, y, z] = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        x * x + y * y + z * z
    };
    let (zero_two, one_three) = (length([p[0], p[2]]), length([p[1], p[3]]));
    if zero_two < one_three {
        return 0;
    }
    if one_three < zero_two {
        return 1;
    }
    // Arrays compare in order, element by element.
    let key = |[x, y, z]: [f32; 3]| [x.abs(), y.abs(), z.abs(), x, y, z];
    let first = (1..4).fold(0, |best, k| if key(p[k]) < key(p[best]) { k } else { best });
    first % 2
}

/// The triangles `face` is split into, wherever a face is split, by corner (indices into
/// `face`): a triangle whole, a quad along its [`quad_diagonal`], as `0 1 2, 0 2 3` or as
/// `1 2 3, 1 3 0`. Each keeps the face's winding, and either way a quad's diagonal is the edge
/// opposite corner 1 of its first triangle and corner 2 of its second. `pos(k)` gives the
/// position of corner `k`, 0 to 3; a triangle's are not read.
pub fn face_triangles(face: [u32; 4], pos: impl FnMut(usize) -> [f32; 3]) -> &'static [[usize; 3]] {
    if arity(face) == 3 {
        return &WHOLE;
    }
    quad_triangles(quad_diagonal(std::array::from_fn(pos)))
}

/// A quad's two triangles along `diagonal` (0 or 1, as [`quad_diagonal`] gives it), by corner:
/// what [`face_triangles`] gives a quad split that way.
pub fn quad_triangles(diagonal: usize) -> &'static [[usize; 3]; 2] {
    &SPLITS[diagonal & 1]
}

/// A triangle, by corner.
static WHOLE: [[usize; 3]; 1] = [[0, 1, 2]];
/// A quad's two triangles along each diagonal, by corner.
static SPLITS: [[[usize; 3]; 2]; 2] = [[[0, 1, 2], [0, 2, 3]], [[1, 2, 3], [1, 3, 0]]];

/// A plain mesh for interchange: what observation commands return, what import and export
/// read and write, and what tests build and compare.
///
/// This is NOT the engine's internal mesh storage: the engine keeps each part in a `gs-core`
/// mesh store (nodes in spatial order, decision 0001) and exports a `PlainMesh` when a command
/// reads a part, in the vertex and face order the part was made with.
///
/// Faces are quads `[a, b, c, d]` or triangles `[a, b, c, NO_VERTEX]`, wound counter-clockwise
/// when seen from outside. A quad is drawn, picked, measured and written out as the two
/// triangles [`face_triangles`] gives.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlainMesh {
    /// Vertex positions in scene units.
    pub positions: Vec<[f32; 3]>,
    /// Faces as four vertex indices. A triangle has 4294967295 (`u32::MAX`) in the last slot.
    pub faces: Vec<[u32; 4]>,
    /// Named attribute columns, for example a vertex `mask` or a face `group`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Attribute>,
}

impl PlainMesh {
    /// Every face as the triangles it is drawn as ([`face_triangles`]), by vertex index, in
    /// face order.
    pub fn triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        self.faces.iter().flat_map(|&f| {
            let tris = face_triangles(f, |k| self.positions[f[k] as usize]);
            tris.iter().map(move |t| t.map(|k| f[k]))
        })
    }
}

/// One named attribute column: one value per vertex or per face.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    /// Whether there is one value per vertex or one per face.
    pub domain: AttrDomain,
    /// The values, in vertex or face order.
    pub values: AttrValues,
}

/// What an attribute column is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttrDomain {
    /// One value per vertex.
    Vertex,
    /// One value per face.
    Face,
}

/// The values of an attribute column.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttrValues {
    /// 32-bit floats, for example a mask.
    F32(Vec<f32>),
    /// 32-bit unsigned integers, for example a face group id.
    U32(Vec<u32>),
}
