//! What the add-on sent, decoded: each manifest entry's column files read, checked and turned
//! back into the scene's axes and the store's columns.

use std::collections::BTreeMap;

use gs_core::transform::Similarity;
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, ErrorCode, NO_VERTEX, PlainMesh,
};

use super::Form;
use super::color;
use super::manifest::{ColumnFile, Entry, plain_name};
use super::names::{self, Column, Domain};
use crate::errors::error;

/// One object as the add-on sent it, in the scene's axes.
pub struct Incoming {
    /// The part it was sent as, if any.
    pub id: Option<u64>,
    /// Blender's name for it.
    pub name: String,
    pub form: Form,
    pub levels: u32,
    pub transform: Similarity,
    /// Points, faces (n-gons split into triangles) and the sculpt and color columns the manifest
    /// lists: `mask` settled, `group`, `hide` as 0 or 1, and `color` as the store's opaque words.
    pub mesh: PlainMesh,
    /// Polygons of more than four corners split into triangles.
    pub split: u64,
}

/// A refusal naming the object `name` came as.
fn refused(name: &str, what: impl std::fmt::Display) -> CommandError {
    error(
        ErrorCode::Precondition,
        format!("the object '{name}' from Blender {what}"),
    )
}

/// Decodes entry `entry` from the folder's files, `read` by file name.
pub fn decode(read: &BTreeMap<String, Vec<u8>>, entry: &Entry) -> Result<Incoming, CommandError> {
    let name = entry.name.as_str();
    let mut files: Files = BTreeMap::new();
    for c in &entry.columns {
        let (row, bytes) = column(read, entry, c)?;
        if files.insert(row.blender, (row, bytes)).is_some() {
            return Err(refused(name, format!("lists {} twice", c.blender)));
        }
    }
    let need = |c: Column| {
        let file = files.get(c.blender).map(|&(_, bytes)| bytes);
        file.ok_or_else(|| refused(name, format!("has no {}", c.blender)))
    };
    let positions: Vec<[f32; 3]> = (floats(need(names::POSITION)?).as_chunks::<3>().0.iter())
        .map(|&p| names::from_blender(p))
        .collect();
    if positions.iter().flatten().any(|x| !x.is_finite()) {
        return Err(refused(name, "has a point that is not a finite number"));
    }
    let corner_vert = ints(need(names::CORNER_VERT)?);
    let polygons = polygons(
        name,
        &corner_vert,
        &ints(need(names::LOOP_START)?),
        positions.len(),
    )?;
    let transform = entry.transform.similarity();
    if !transform.is_valid() {
        return Err(refused(
            name,
            "has a place that is not a move, a turn and one scale above zero",
        ));
    }
    let mut faces = Vec::with_capacity(polygons.len());
    let mut from = Vec::with_capacity(polygons.len());
    let mut split = 0;
    for (p, corners) in polygons.iter().enumerate() {
        let before = faces.len();
        fan(&positions, corners, &mut faces);
        split += u64::from(faces.len() - before > 1);
        from.resize(faces.len(), p);
    }
    let attributes = sculpt(name, &files, &from, &corner_vert, positions.len())?;
    Ok(Incoming {
        id: entry.id.as_deref().and_then(|id| id.parse().ok()),
        name: entry.name.clone(),
        form: entry.form,
        levels: entry.levels.max(1),
        transform,
        mesh: PlainMesh {
            positions,
            faces,
            attributes,
        },
        split,
    })
}

/// An object's column files by Blender name, each with its row of the table.
type Files<'a> = BTreeMap<&'static str, (Column, &'a [u8])>;

/// The store's columns from the sculpt and color files listed: `mask` settled, `group`, `hide`
/// as 0 or 1 on each face of the split (`from` gives each face's polygon), and `color` as the
/// store's words, one a point (`corner_vert` gives each corner's point).
fn sculpt(
    name: &str,
    files: &Files<'_>,
    from: &[usize],
    corner_vert: &[i32],
    points: usize,
) -> Result<BTreeMap<String, Attribute>, CommandError> {
    let take = |c: Column| files.get(c.blender).map(|&(_, bytes)| bytes);
    let mut attributes = BTreeMap::new();
    if let Some(bytes) = take(names::MASK) {
        let mask = floats(bytes)
            .into_iter()
            .map(gs_core::mask::settle)
            .collect();
        attributes.insert(names::MASK.gs.into(), vertex(AttrValues::F32(mask)));
    }
    if let Some(bytes) = take(names::GROUP) {
        let ids = ints(bytes);
        if let Some(bad) = ids.iter().find(|&&g| g < 0) {
            return Err(refused(
                name,
                format!("has face set {bad}; groups are 0 or more"),
            ));
        }
        let ids = from.iter().map(|&p| ids[p].unsigned_abs()).collect();
        attributes.insert(names::GROUP.gs.into(), face(AttrValues::U32(ids)));
    }
    if let Some(bytes) = take(names::HIDE) {
        let hide = from.iter().map(|&p| u32::from(bytes[p] != 0)).collect();
        attributes.insert(names::HIDE.gs.into(), face(AttrValues::U32(hide)));
    }
    if let Some(&(row, bytes)) = files.get(names::COLOR.blender) {
        let words =
            color::points(row, bytes, corner_vert, points).map_err(|why| refused(name, why))?;
        attributes.insert(names::COLOR.gs.into(), vertex(AttrValues::U32(words)));
    }
    Ok(attributes)
}

fn vertex(values: AttrValues) -> Attribute {
    Attribute {
        domain: AttrDomain::Vertex,
        values,
    }
}

fn face(values: AttrValues) -> Attribute {
    Attribute {
        domain: AttrDomain::Face,
        values,
    }
}

/// Column file `c` of `entry`, checked against the name table and its length.
fn column<'a>(
    read: &'a BTreeMap<String, Vec<u8>>,
    entry: &Entry,
    c: &ColumnFile,
) -> Result<(Column, &'a [u8]), CommandError> {
    let name = entry.name.as_str();
    let Some(row) = c.row() else {
        return Err(refused(
            name,
            format!(
                "lists the column {} as {} that this engine's name table does not have",
                c.blender, c.gs
            ),
        ));
    };
    let Some(bytes) = read.get(&c.file).filter(|_| plain_name(&c.file)) else {
        return Err(refused(name, format!("names the file {:?}", c.file)));
    };
    let count = match row.domain {
        Domain::Point => entry.points,
        Domain::Face => entry.faces,
        Domain::Corner => entry.corners,
    };
    let want = count * u64::from(row.width) * row.elem.size() as u64;
    if bytes.len() as u64 != want {
        return Err(refused(
            name,
            format!("has {} bytes in {}, {want} expected", bytes.len(), c.file),
        ));
    }
    Ok((row, bytes))
}

/// A column's four-byte little-endian values, each made by `from` (`f32::from_le_bytes` or
/// `i32::from_le_bytes`).
fn words<T>(bytes: &[u8], from: fn([u8; 4]) -> T) -> Vec<T> {
    bytes.as_chunks::<4>().0.iter().map(|&b| from(b)).collect()
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    words(bytes, f32::from_le_bytes)
}

fn ints(bytes: &[u8]) -> Vec<i32> {
    words(bytes, i32::from_le_bytes)
}

/// Every face's corners, from Blender's corner list and face starts: starts from 0 up, at
/// least three corners a face, every corner one of the `points`.
fn polygons(
    name: &str,
    corners: &[i32],
    starts: &[i32],
    points: usize,
) -> Result<Vec<Vec<u32>>, CommandError> {
    let at = |v: i32| usize::try_from(v).ok();
    if let Some(v) = corners.iter().find(|&&v| at(v).is_none_or(|v| v >= points)) {
        return Err(refused(
            name,
            format!("has a corner at point {v} of {points}"),
        ));
    }
    let mut out = Vec::with_capacity(starts.len());
    for (f, &start) in starts.iter().enumerate() {
        let start = at(start).filter(|&s| f > 0 || s == 0);
        let end = starts.get(f + 1).map_or(Some(corners.len()), |&e| at(e));
        let range = match (start, end) {
            (Some(s), Some(e)) if s + 3 <= e && e <= corners.len() => s..e,
            _ => {
                return Err(refused(
                    name,
                    format!("has face {f} with fewer than three corners or out of order"),
                ));
            }
        };
        out.push(corners[range].iter().map(|&v| v.unsigned_abs()).collect());
    }
    Ok(out)
}

/// Adds polygon `corners` to `faces`: a triangle or quad as it is, more corners as a fan of
/// triangles from the corner lowest in y, then z, then distance from x = 0, as `file.import`
/// splits a polygon, so a polygon and its mirror image across x = 0 split alike.
fn fan(positions: &[[f32; 3]], corners: &[u32], faces: &mut Vec<[u32; 4]>) {
    let n = corners.len();
    if n <= 4 {
        let mut face = [NO_VERTEX; 4];
        face[..n].copy_from_slice(corners);
        faces.push(face);
        return;
    }
    let at = |k: usize| positions[corners[k] as usize];
    let lower = |a: &usize, b: &usize| {
        let (p, q) = (at(*a), at(*b));
        (p[1].total_cmp(&q[1]))
            .then(p[2].total_cmp(&q[2]))
            .then(p[0].abs().total_cmp(&q[0].abs()))
    };
    let apex = (0..n).min_by(lower).unwrap_or(0);
    let corner = |k: usize| corners[(apex + k) % n];
    faces.extend((1..n - 1).map(|k| [corner(0), corner(k), corner(k + 1), NO_VERTEX]));
}
