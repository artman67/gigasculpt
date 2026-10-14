//! The one name table (decision 0032, section 3): which store column goes to which Blender
//! attribute, in which domain and element type, and the two conversions the engine does for the
//! add-on, Y up to Z up and crease sharpness to Blender's crease value. The add-on mirrors this
//! table in `names.py`; the manifest carries both names for every column, so either side
//! refuses a manifest the other wrote with a different table.

use gs_core::transform::Similarity;
use gs_core::{Quat, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a column's values belong in Blender.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    /// One value per point.
    Point,
    /// One value per face.
    Face,
    /// One value per face corner.
    Corner,
}

/// A column file's element type, little-endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Elem {
    /// A 32-bit float.
    F32,
    /// A 32-bit signed integer.
    I32,
    /// One byte.
    U8,
}

impl Elem {
    /// Bytes per element.
    pub fn size(self) -> usize {
        match self {
            Elem::F32 | Elem::I32 => 4,
            Elem::U8 => 1,
        }
    }
}

/// One row of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    /// The store's name for it.
    pub gs: &'static str,
    /// Blender's.
    pub blender: &'static str,
    pub domain: Domain,
    pub elem: Elem,
    /// Values per element.
    pub width: u32,
}

const fn column(gs: &'static str, blender: &'static str, domain: Domain, elem: Elem) -> Column {
    Column {
        gs,
        blender,
        domain,
        elem,
        width: 1,
    }
}

/// Point positions, Z up.
pub const POSITION: Column = Column {
    width: 3,
    ..column("positions", "position", Domain::Point, Elem::F32)
};
/// Each face corner's point, faces one after another in order.
pub const CORNER_VERT: Column = column("faces", ".corner_vert", Domain::Corner, Elem::I32);
/// Each face's first corner (Blender's `polygons.loop_start`).
pub const LOOP_START: Column = column("faces", "loop_start", Domain::Face, Elem::I32);
/// The mask, the same meaning on both sides.
pub const MASK: Column = column(gs_protocol::MASK, ".sculpt_mask", Domain::Point, Elem::F32);
/// Face groups as face sets.
pub const GROUP: Column = column(
    gs_protocol::GROUP,
    ".sculpt_face_set",
    Domain::Face,
    Elem::I32,
);
/// Hidden faces, 0 or 1.
pub const HIDE: Column = column(gs_protocol::HIDE, ".hide_poly", Domain::Face, Elem::U8);

/// Surface paint as Blender's color attribute `Color`, four values an element, in the domain and
/// type Blender keeps it in.
const fn color(domain: Domain, elem: Elem) -> Column {
    Column {
        width: 4,
        ..column(gs_protocol::COLOR, "Color", domain, elem)
    }
}

/// The colors GigaSculpt sends: the store's sRGB bytes, red first, one point each.
pub const COLOR: Column = color(Domain::Point, Elem::U8);

/// Every form a color comes back in: the active color attribute as Blender keeps it, bytes or
/// floats, at the point or the corner domain (decision 0032, section 8's amendment).
pub const COLORS: [Column; 4] = [
    COLOR,
    color(Domain::Point, Elem::F32),
    color(Domain::Corner, Elem::U8),
    color(Domain::Corner, Elem::F32),
];

/// Every column the exchange carries.
pub const TABLE: [Column; 10] = [
    POSITION,
    CORNER_VERT,
    LOOP_START,
    MASK,
    GROUP,
    HIDE,
    COLORS[0],
    COLORS[1],
    COLORS[2],
    COLORS[3],
];

/// The sculpt columns a part sends in the Mesh form when it has them, and reads back as zeros
/// when the manifest leaves them out.
pub const SCULPT: [Column; 3] = [MASK, GROUP, HIDE];

/// A point in Blender's axes: a quarter turn about x, so (x, y, z) becomes (x, -z, y). Swaps and
/// one negation, so it is exact both ways.
pub fn to_blender(p: [f32; 3]) -> [f32; 3] {
    [p[0], -p[2], p[1]]
}

/// A point from Blender's axes, the inverse of [`to_blender`].
pub fn from_blender(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[2], -p[1]]
}

/// A part's place in Blender's terms: `location`, `rotation_quaternion` as (w, x, y, z) and one
/// scale. The rotation is `C q C^-1`, which only moves and negates components.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub location: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: f64,
}

impl Place {
    /// A part's transform in Blender's axes. Each number is written as the f64 equal to its f32,
    /// so it reads back as the same f32 on either side.
    pub fn of(t: &Similarity) -> Place {
        let [x, y, z, w] = t.rotation.to_array();
        let loc = to_blender(t.translation.to_array());
        Place {
            location: loc.map(f64::from),
            rotation: [w, x, -z, y].map(f64::from),
            scale: f64::from(t.scale),
        }
    }

    /// The transform this place means in the scene's axes. Narrowing f64 to f32 is exact for the
    /// numbers either side writes.
    pub fn similarity(&self) -> Similarity {
        let [w, x, y, z] = self.rotation.map(|v| v as f32);
        Similarity {
            translation: Vec3::from_array(from_blender(self.location.map(|v| v as f32))),
            rotation: Quat::from_array([x, z, -y, w]),
            scale: self.scale as f32,
        }
    }
}

/// Blender's crease value for sharpness `k` (levels the edge stays sharp, 255 for always):
/// `sqrt(k / 10)`, and 1.0 from 10 up (research 19, 2.3).
pub fn crease_value(k: u8) -> f32 {
    if k >= 10 {
        1.0
    } else {
        (f32::from(k) / 10.0).sqrt()
    }
}
