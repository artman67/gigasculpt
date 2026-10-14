//! The brush object: everything after the file header.
//!
//! Its body holds, in order: a head (draw size from version 0x4C, Z and RGB intensity, an
//! unknown float, alpha flags), the alpha picture when there is one, two plain fields (a
//! 2-byte field more from version 0x5A), a mesh flag byte and, when set, a mesh count and the
//! embedded meshes. After that come plain fields, the named setting records, and between groups
//! of records more plain fields and sometimes an extra object.

use super::{Block, BrushHead, MeshEntry, NamedImage, RawObject, Section, Setting, ZbpError};
use crate::container::chunk::{Cursor, Obj, Piece, scan};
use crate::container::record::parse_setting;
use crate::container::{name, picture, planar};

/// First brush object version that stores the draw size in its head.
const DRAW_SIZE_FROM: u16 = 0x4C;
/// First brush object version with an extra 2-byte field before the mesh flag.
const MESH_FIELD_FROM: u16 = 0x5A;

/// The decoded brush object.
pub(crate) struct Body {
    pub head: BrushHead,
    pub alpha: Option<NamedImage>,
    pub meshes: Vec<MeshEntry>,
    pub settings: Vec<Setting>,
    pub blocks: Vec<Block>,
    pub sections: Vec<Section>,
}

pub(crate) fn parse_body(obj: Obj<'_>) -> Result<Body, ZbpError> {
    let mut c = obj.reader();
    let draw_size = if obj.version >= DRAW_SIZE_FROM {
        Some(c.f32("draw size")?)
    } else {
        None
    };
    let head = BrushHead {
        version: obj.version,
        draw_size,
        z_intensity: c.f32("brush Z intensity")?,
        rgb_intensity: c.f32("brush RGB intensity")?,
        unknown: c.f32("brush head")?,
        alpha_flags: c.u32("alpha flags")?,
    };
    let alpha = if c.at_object() {
        Some(picture(&mut c, "alpha")?)
    } else {
        None
    };
    c.u32("mesh area")?;
    if obj.version >= MESH_FIELD_FROM {
        c.u16("mesh area")?;
    }
    let meshes = if c.u8("mesh flag")? == 0 {
        Vec::new()
    } else {
        meshes(&mut c)?
    };
    let offset = c.offset();
    let mut body = Body {
        head,
        alpha,
        meshes,
        settings: Vec::new(),
        blocks: Vec::new(),
        sections: Vec::new(),
    };
    records(c.rest(), offset, &mut body)?;
    Ok(body)
}

/// The mesh count and the entries: a tag, then name, rectangle, mesh and companion objects.
fn meshes(c: &mut Cursor<'_>) -> Result<Vec<MeshEntry>, ZbpError> {
    let count = c.u32("mesh count")?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let tag = c.u32("mesh entry tag")?;
        let name = name(c.object("mesh name")?)?;
        c.object("mesh rectangle")?;
        let mesh = raw(c.object("mesh")?);
        let companion = raw(c.object("mesh companion")?);
        entries.push(MeshEntry {
            tag,
            name,
            mesh,
            companion,
        });
    }
    Ok(entries)
}

fn raw(obj: Obj<'_>) -> RawObject {
    RawObject {
        version: obj.version,
        bytes: obj.body.to_vec(),
    }
}

/// The setting records and what lies between them. `group` counts the plain-field blocks met
/// since the first record, so records with the same name in different groups stay apart.
fn records(data: &[u8], offset: usize, body: &mut Body) -> Result<(), ZbpError> {
    let mut group = 0u8;
    for piece in scan(data, offset)? {
        let after = body.settings.len();
        match piece {
            Piece::Raw { bytes, .. } => {
                group = group.saturating_add(u8::from(after > 0));
                body.blocks.push(Block {
                    after,
                    bytes: bytes.to_vec(),
                });
            }
            Piece::Obj(obj) if obj.version == 5 => {
                body.settings.push(parse_setting(obj, group, false)?);
            }
            Piece::Obj(obj) => {
                let images = planar::find_images(obj)?;
                body.sections.push(Section {
                    after,
                    object: raw(obj),
                    images,
                });
            }
        }
    }
    Ok(())
}
