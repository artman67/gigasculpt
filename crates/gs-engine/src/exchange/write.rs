//! `exchange.write`.

use std::path::Path;

use gs_core::Levels;
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, ErrorCode, Kind, NO_VERTEX, PartRef, PlainMesh,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::manifest::{self, ColumnFile, Entry, Manifest};
use super::names::{self, Column, Elem, Place};
use super::{Form, folder, top};
use crate::document::Part;
use crate::errors::{self, error};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Write parts into an exchange folder for the Blender add-on (decision 0032): one raw
/// little-endian file per column, in Blender's axes (Z up) and under Blender's names, then a
/// `manifest.json` listing them, written last. A part with detail levels goes as its top level,
/// edits pending on a lower level carried up on a copy. In the Mesh form every part carries its
/// mask, face groups and hidden faces, and a painted part its colors, which the add-on makes
/// the active and render color `Color` (with Show Paint off they still go, and the manifest
/// says so); in the Levels form a part with more than one level carries its points, faces and
/// base creases only, for the add-on to rebuild as a Multires modifier. Points stay in part
/// space: the part's transform goes in the manifest. Writes nothing to the scene.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExchangeWrite {
    /// The folder, in full: the add-on's `to-blender` folder. It must exist.
    #[schemars(length(min = 1))]
    pub folder: String,
    /// The parts to send. Left out: every part, in list order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<PartRef>>,
    /// How parts with detail levels go.
    #[serde(default)]
    pub form: Form,
}

/// What a write put into the folder.
#[derive(Serialize, JsonSchema)]
pub struct Written {
    /// The parts written, in order.
    pub parts: Vec<Sent>,
    /// Bytes in the column files.
    pub bytes: u64,
}

/// One part written.
#[derive(Serialize, JsonSchema)]
pub struct Sent {
    pub part: String,
    /// Its stable id, which the Blender object keeps as `gigasculpt_part`.
    pub id: u64,
    /// The form it went in: a part with one level always goes in the Mesh form.
    pub form: Form,
    /// Its detail levels, the base included.
    pub levels: u32,
    pub points: u64,
    pub faces: u64,
}

impl Command for ExchangeWrite {
    const NAME: &'static str = "exchange.write";
    const KIND: Kind = Kind::Query;
    type Output = Written;

    fn examples() -> Vec<Example<Self>> {
        let call = || Self {
            folder: "/nonexistent/exchange/to-blender".into(),
            parts: None,
            form: Form::Mesh,
        };
        vec![
            Example::new("an empty scene has nothing to send", call())
                .fails_with(ErrorCode::Precondition),
            Example::new(
                "send the cube (refused here: the add-on makes the folder first)",
                call(),
            )
            .after("primitive.cube", json!({}))
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        let none = || {
            Unavailable::new("the scene has no parts to send").fix("import or create a part first")
        };
        cx.doc.part_names().next().map(|_| ()).ok_or_else(none)
    }

    fn run(self, cx: &mut Ctx) -> Result<Written, CommandError> {
        cx.settled()?;
        let dir = folder(&self.folder)?;
        let list = cx.doc.document().list();
        let chosen: Vec<(&str, &Part)> = match &self.parts {
            None => list,
            Some(refs) => (refs.iter())
                .map(|r| {
                    let name = cx.doc.resolve(&r.0);
                    let found = list.iter().find(|(n, _)| Some(*n) == name.as_deref());
                    found
                        .copied()
                        .ok_or_else(|| errors::not_found("part", &r.0, cx.doc.part_names()))
                })
                .collect::<Result<_, _>>()?,
        };
        let (mut entries, mut parts, mut bytes) = (Vec::new(), Vec::new(), 0);
        for (k, (name, part)) in chosen.into_iter().enumerate() {
            let (entry, sent, n) = send(&dir, k, name, part, self.form)?;
            entries.push(entry);
            parts.push(sent);
            bytes += n;
        }
        let written = Manifest {
            format: manifest::FORMAT.into(),
            version: manifest::VERSION,
            writer: format!("GigaSculpt {}", env!("CARGO_PKG_VERSION")),
            parts: entries,
        };
        manifest::put_manifest(&dir, &written)?;
        Ok(Written { parts, bytes })
    }
}
register_command!(ExchangeWrite, "Send to Blender");

/// Writes part `name`, the `k`th in the folder: its column files, and its manifest entry.
fn send(
    dir: &Path,
    k: usize,
    name: &str,
    part: &Part,
    form: Form,
) -> Result<(Entry, Sent, u64), CommandError> {
    let (mesh, levels) = top(part)?;
    let count = levels.as_ref().map_or(1, Levels::count) as u32;
    let form = if count > 1 { form } else { Form::Mesh };
    let mut corner_vert = Vec::new();
    let mut loop_start = Vec::with_capacity(mesh.faces.len());
    for f in &mesh.faces {
        loop_start.push(corner_vert.len() as u32);
        corner_vert.extend(f.iter().copied().filter(|&v| v != NO_VERTEX));
    }
    let positions: Vec<f32> = (mesh.positions.iter())
        .flat_map(|&p| names::to_blender(p))
        .collect();
    let mut sent = vec![
        (names::POSITION, Values::F32(&positions)),
        (names::CORNER_VERT, Values::U32(&corner_vert)),
        (names::LOOP_START, Values::U32(&loop_start)),
    ];
    if form == Form::Mesh {
        sent.extend(painted(name, &mesh)?);
    }
    let (columns, bytes) = put(dir, k, sent)?;
    let creases = match (&levels, form) {
        (Some(l), Form::Levels) => (l.stored().creases.into_iter())
            .map(|(a, b, s)| (a, b, f64::from(names::crease_value(s))))
            .collect(),
        _ => Vec::new(),
    };
    let props = part.props();
    let id = props
        .id
        .ok_or_else(|| errors::internal("a part without an id"))?;
    let entry = Entry {
        id: Some(id.to_string()),
        name: name.into(),
        rank: props.rank,
        hidden: props.hidden,
        paint_hidden: props.paint_hidden,
        form,
        levels: count,
        transform: Place::of(&props.transform),
        points: mesh.positions.len() as u64,
        faces: mesh.faces.len() as u64,
        corners: corner_vert.len() as u64,
        creases,
        columns,
    };
    let sent = Sent {
        part: name.into(),
        id,
        form,
        levels: count,
        points: entry.points,
        faces: entry.faces,
    };
    Ok((entry, sent, bytes))
}

/// Writes part `k`'s column files, `k-<Blender name>.bin`, and returns their manifest lines and
/// how many bytes they hold.
fn put(
    dir: &Path,
    k: usize,
    sent: Vec<(Column, Values<'_>)>,
) -> Result<(Vec<ColumnFile>, u64), CommandError> {
    let (mut columns, mut bytes) = (Vec::new(), 0);
    for (c, values) in sent {
        let f = format!("{k}-{}.bin", c.blender.trim_start_matches('.'));
        bytes += match values {
            Values::F32(v) => manifest::put_column(dir, &f, v, f32::to_le_bytes)?,
            Values::U32(v) => manifest::put_column(dir, &f, v, u32::to_le_bytes)?,
            Values::U8(v) => manifest::put_bytes(dir, &f, &v)?,
        };
        columns.push(ColumnFile::of(c, f));
    }
    Ok((columns, bytes))
}

/// The Mesh form's columns past the points and faces: the sculpt columns part `name` has, then
/// its colors when it has a color column. A color word's little-endian bytes are the four
/// sRGB bytes, red first, that the `Color` file holds.
fn painted<'a>(name: &str, mesh: &'a PlainMesh) -> Result<Vec<(Column, Values<'a>)>, CommandError> {
    let mut out = Vec::new();
    for c in names::SCULPT {
        if let Some(a) = mesh.attributes.get(c.gs) {
            out.push((c, sculpt(name, c, &a.values)?));
        }
    }
    let color = mesh.attributes.get(names::COLOR.gs);
    if let Some(Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::U32(v),
    }) = color
    {
        out.push((names::COLOR, Values::U32(v)));
    }
    Ok(out)
}

/// A column's values as they go into its file.
enum Values<'a> {
    F32(&'a [f32]),
    /// Written as little-endian `u32`: the same bits as the `i32` Blender reads for a face or
    /// corner index (every one below 2^31), and a color's four bytes, red first.
    U32(&'a [u32]),
    U8(Vec<u8>),
}

/// Sculpt column `c` of part `name` as its file holds it. Refuses a face group of 2^31 or more,
/// which Blender's face sets cannot hold.
fn sculpt<'a>(name: &str, c: Column, values: &'a AttrValues) -> Result<Values<'a>, CommandError> {
    match (values, c.elem) {
        (AttrValues::F32(v), Elem::F32) => Ok(Values::F32(v)),
        (AttrValues::U32(v), Elem::I32) => match v.iter().find(|&&g| g > i32::MAX as u32) {
            Some(g) => Err(error(
                ErrorCode::Precondition,
                format!(
                    "part '{name}' has face group {g}; Blender's face sets go up to {}",
                    i32::MAX
                ),
            )),
            None => Ok(Values::U32(v)),
        },
        (AttrValues::U32(v), Elem::U8) => {
            Ok(Values::U8(v.iter().map(|&x| u8::from(x != 0)).collect()))
        }
        _ => Err(errors::internal("a sculpt column of the wrong type")),
    }
}
