//! `file.export`.

use std::borrow::Cow;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Arc;

use gs_io::mesh::{Object, WriteOptions};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef, PlainMesh, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Format, UpAxis, io_error, mirror};
use crate::document::ExportTransform;
use crate::errors::{self, error};
use crate::mesh::selected;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Write parts to a mesh file: OBJ, PLY or STL.
///
/// One part, or every part: each as an object of its own in OBJ and ASCII STL, all in one mesh
/// in PLY and binary STL. Positions are written as `position * scale + offset`, then turned to
/// the file's up axis, then mirrored. Left out, `scale` and `offset` are what each part
/// remembers from the file it came from (`file.import` reports them), so an imported part goes
/// back at its file's size and place; for any other part, 1 and 0. OBJ keeps face groups (`g Group<id>`), UVs, vertex colors and the
/// mask (as `#MRGB` lines); PLY keeps vertex colors; STL keeps triangles only. An
/// existing file is refused unless `overwrite` is on. Writes nothing to the scene.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent export switches, one per export option"
)]
pub struct FileExport {
    /// The file to write. A relative path starts at the engine's working folder, not the
    /// caller's: give an absolute path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The format. Left out: from the file name's extension (obj, ply or stl).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// The part to write. Left out: every part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Replace the file if it exists.
    #[serde(default)]
    pub overwrite: bool,
    /// Write triangles only (quads split as they are drawn, along the shorter diagonal). STL
    /// always is.
    #[serde(default)]
    pub triangles: bool,
    /// Reverse every face's winding.
    #[serde(default)]
    pub flip_faces: bool,
    /// Multiply every position by this. Left out: the part's file scale, or 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// Then add this. Left out: the part's file offset, or 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Vec3>,
    /// The file's up axis.
    #[serde(default)]
    pub up: UpAxis,
    /// Mirror across x = 0, last (negates every x; the winding stays as it is).
    #[serde(default)]
    pub flip_x: bool,
    /// Mirror across y = 0, last.
    #[serde(default)]
    pub flip_y: bool,
    /// Mirror across z = 0, last.
    #[serde(default)]
    pub flip_z: bool,
    /// OBJ: write UVs.
    #[serde(default = "crate::command::yes")]
    pub uvs: bool,
    /// OBJ: write face groups.
    #[serde(default = "crate::command::yes")]
    pub groups: bool,
    /// OBJ and PLY: write vertex colors.
    #[serde(default = "crate::command::yes")]
    pub colors: bool,
    /// OBJ: write the mask (with colors) as `#MRGB` lines.
    #[serde(default = "crate::command::yes")]
    pub mask: bool,
    /// PLY and STL: write text instead of binary.
    #[serde(default)]
    pub ascii: bool,
}

/// What an export wrote.
#[derive(Serialize, JsonSchema)]
pub struct Exported {
    /// The file written.
    pub path: String,
    /// Its format.
    pub format: Format,
    /// The parts in it, in order.
    pub parts: Vec<String>,
    /// Vertices written (STL: three per triangle).
    pub vertices: u64,
    /// Faces in the parts written (before any split into triangles).
    pub faces: u64,
    /// The file's size.
    pub bytes: u64,
}

impl Command for FileExport {
    const NAME: &'static str = "file.export";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    type Output = Exported;

    fn examples() -> Vec<Example<Self>> {
        let call = |path: &str| Self {
            path: path.into(),
            format: None,
            part: None,
            overwrite: false,
            triangles: false,
            flip_faces: false,
            scale: None,
            offset: None,
            up: UpAxis::Y,
            flip_x: false,
            flip_y: false,
            flip_z: false,
            uvs: true,
            groups: true,
            colors: true,
            mask: true,
            ascii: false,
        };
        let stl = json!({ "path": "/", "format": "stl", "part": "cube", "up": "z", "scale": 10 });
        vec![
            Example::raw(
                "write the cube as a Z-up STL in millimeters (refused here: / is a folder)",
                stl,
            )
            .after("primitive.cube", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::new(
                "an empty scene has nothing to write",
                call("/nonexistent/scene.obj"),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        if cx.doc.part_names().next().is_none() {
            return Err(Unavailable::new("the scene has no parts to write")
                .fix("import or create a part first"));
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<Exported, CommandError> {
        cx.settled()?;
        cx.progress(None, format!("writing {}", self.path));
        let format = Format::resolve(self.format, &self.path)?;
        if matches!(format, Format::Ztl | Format::Zpr) {
            return Err(errors::args_invalid(
                "/format",
                "file.export writes obj, ply and stl",
            ));
        }
        if self.scale.is_some_and(|s| !(s.is_finite() && s != 0.0)) {
            return Err(errors::args_invalid(
                "/scale",
                "must be a number other than 0",
            ));
        }
        // The parts' own names, never the reference the call named one by (review 0003 M1).
        let (names, meshes): (Vec<String>, Vec<Arc<PlainMesh>>) =
            selected(&cx.doc, self.part.as_ref())?.into_iter().unzip();
        let transforms: Vec<Option<ExportTransform>> = (names.iter())
            .map(|n| cx.doc.export_transform(n))
            .collect::<Result<_, _>>()?;
        let positions: Vec<Cow<[[f32; 3]]>> = (meshes.iter().zip(transforms))
            .map(|(m, t)| self.moved(m, t))
            .collect();
        let objects: Vec<Object> = (names.iter().zip(&meshes).zip(&positions))
            .map(|((name, mesh), positions)| Object {
                name,
                mesh,
                positions,
            })
            .collect();
        let options = WriteOptions {
            triangles: self.triangles,
            flip_faces: self.flip_faces,
            uvs: self.uvs,
            groups: self.groups,
            colors: self.colors,
            mask: self.mask,
            binary: !self.ascii,
        };
        let write = |out: &mut File| match format {
            Format::Obj => gs_io::obj::write(out, &objects, &options),
            Format::Ply => gs_io::ply::write(out, &objects, &options),
            _ => gs_io::stl::write(out, &objects, &options),
        };
        save(&self.path, self.overwrite, write)?;
        let bytes = fs::metadata(&self.path)
            .map_err(|e| io_error(&self.path, &e))?
            .len();
        let stl = format == Format::Stl;
        Ok(Exported {
            path: self.path.clone(),
            format,
            vertices: (meshes.iter())
                .map(|m| {
                    if stl {
                        3 * triangles(m)
                    } else {
                        m.positions.len() as u64
                    }
                })
                .sum(),
            faces: meshes.iter().map(|m| m.faces.len() as u64).sum(),
            parts: names,
            bytes,
        })
    }
}
register_command!(FileExport, "Export file");

impl FileExport {
    /// A part's positions as they go into the file, with the part's own file transform where
    /// the call gives none.
    fn moved<'a>(&self, mesh: &'a PlainMesh, file: Option<ExportTransform>) -> Cow<'a, [[f32; 3]]> {
        let flip = [self.flip_x, self.flip_y, self.flip_z];
        let scale = (self.scale).unwrap_or(file.map_or(1.0, |t| t.scale));
        let o = (self.offset).map_or(file.map_or([0.0; 3], |t| t.offset), |o| o.0);
        let identity = scale.to_bits() == 1f32.to_bits()
            && o.iter().all(|&o| o.to_bits() == 0)
            && self.up == UpAxis::Y
            && flip == [false; 3];
        if identity {
            return Cow::Borrowed(&mesh.positions);
        }
        let place = |p: [f32; 3]| {
            let p = std::array::from_fn(|i| p[i] * scale + o[i]);
            mirror(self.up.to_file(p), flip)
        };
        Cow::Owned(mesh.positions.iter().map(|&p| place(p)).collect())
    }
}

/// Triangles a mesh makes: one per triangle, two per quad.
fn triangles(mesh: &PlainMesh) -> u64 {
    (mesh.faces.iter())
        .map(|f| if f[3] == gs_protocol::NO_VERTEX { 1 } else { 2 })
        .sum()
}

/// Writes a file with `write`: straight into a new file, or, to replace one, into a file
/// beside it that then takes its place, so a failed export never leaves half a file. A folder
/// that is not there, or a folder given for the file, is refused first, in words a person
/// reads (`brush.save` and `curve.save` write through here too).
pub(crate) fn save(
    path: &str,
    overwrite: bool,
    write: impl Fn(&mut File) -> io::Result<()>,
) -> Result<(), CommandError> {
    let failed = |e: &io::Error| {
        error(
            ErrorCode::Precondition,
            format!("cannot write \"{path}\": {e}"),
        )
    };
    super::has_folder(Path::new(path))?;
    super::not_a_folder(Path::new(path))?;
    if !overwrite {
        let created = OpenOptions::new().write(true).create_new(true).open(path);
        let mut file = created.map_err(|e| match e.kind() {
            // Naming `overwrite`, so the window asks to replace it (review 0009, B2).
            io::ErrorKind::AlreadyExists => CommandError {
                path: Some("/overwrite".into()),
                hint: Some("pass overwrite: true to replace it".into()),
                ..error(
                    ErrorCode::Precondition,
                    format!("a file exists at \"{path}\""),
                )
            },
            _ => failed(&e),
        })?;
        return write(&mut file).map_err(|e| {
            fs::remove_file(path).ok();
            failed(&e)
        });
    }
    let name = Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let temp = Path::new(path).with_file_name(format!(".{name}.{}.part", std::process::id()));
    let result = File::create(&temp).and_then(|mut file| write(&mut file));
    let result = result.and_then(|()| fs::rename(&temp, path));
    result.map_err(|e| {
        fs::remove_file(&temp).ok();
        failed(&e)
    })
}
