//! `file.import`.

use std::fs::File;
use std::io::BufReader;

use gs_core::{Aabb, NodeId};
use gs_io::mesh::{MeshFile, ReadOptions};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::clean::{Keep, drop_repeated, keep_columns, merge, place};
use super::{Format, UpAxis, io_error, mirror, ztl};
use crate::document::{DocTxn, ExportTransform, VertexOrder, unfit};
use crate::errors::{self, error};
use crate::job::JobCell;
use crate::mesh::value_problems;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Import a mesh file as new parts: OBJ, PLY, STL, or a ZTL tool (.ztl) or ZPR project (.zpr).
///
/// Every object in the file becomes a part: an OBJ object (`o`), an ASCII STL solid, a part
/// of a ZTL tool at its highest stored subdivision level. Parts are named after their
/// objects, or after the file, with -2, -3 ... added when a name is taken, and `part ` put in
/// front of a name that would read as a part id or place (`#2`, `@0`). Face groups, UVs
/// (kept per face corner in the columns uv0.u to uv3.v), vertex colors and the mask come along
/// when the file holds them. Normals are recomputed, polygons with more than four corners are
/// split into triangles, and faces with no area are dropped. One undo step.
///
/// The result lists each part with the export scale and offset that write it back at the size
/// and place it had in the file (for a ZTL part: as its stored export scale says), and notes
/// everything changed or left out, such as subdivision levels the scene cannot hold yet. Each
/// part also remembers that scale and offset: `file.export` uses them when the call gives none.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent import switches, one per import option"
)]
pub struct FileImport {
    /// The file to read. A relative path starts at the engine's working folder, not the
    /// caller's: give an absolute path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The format. Left out: from the file name's extension (obj, ply, stl, ztl, zpr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// The new part's name, or, when the file holds several objects, the start of each new
    /// part's name. Left out: the objects' own names, else the file's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// Multiply every position by this (for example 0.01 for a file in centimeters).
    #[serde(default = "one")]
    pub scale: f32,
    /// Scale and center the new parts together so their largest side is `fit_size`.
    #[serde(default)]
    pub fit: bool,
    /// The largest side after `fit`, in scene units.
    #[serde(default = "two")]
    pub fit_size: f32,
    /// Mirror across x = 0 (negates every x; the winding stays as it is).
    #[serde(default)]
    pub flip_x: bool,
    /// Mirror across y = 0.
    #[serde(default)]
    pub flip_y: bool,
    /// Mirror across z = 0.
    #[serde(default)]
    pub flip_z: bool,
    /// The file's up axis.
    #[serde(default)]
    pub up: UpAxis,
    /// Merge vertices at the same position into one. Left out: on for STL, which shares no
    /// vertices between triangles, off for the rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<bool>,
    /// Keep face groups (OBJ groups, ZTL face groups).
    #[serde(default = "crate::command::yes")]
    pub groups: bool,
    /// OBJ: face groups from material names (`usemtl`) instead of group names.
    #[serde(default)]
    pub materials_as_groups: bool,
    /// Give each polygon with more than four corners a face group of its own before splitting
    /// it.
    #[serde(default)]
    pub group_polygons: bool,
    /// Keep UVs.
    #[serde(default = "crate::command::yes")]
    pub uvs: bool,
    /// Keep vertex colors.
    #[serde(default = "crate::command::yes")]
    pub colors: bool,
    /// Keep the mask.
    #[serde(default = "crate::command::yes")]
    pub mask: bool,
    /// Projects (.zpr): import only this tool, counting from 0. Left out: every tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<u32>,
    /// The BLAKE3 hash of the file's bytes, as 64 hex digits. Filled in by the engine before
    /// the import runs, so the log records which content was read; a call whose hash no longer
    /// matches the file is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

fn one() -> f32 {
    1.0
}

fn two() -> f32 {
    2.0
}

/// What an import added and noticed.
#[derive(Serialize, JsonSchema)]
pub struct Imported {
    /// The file read.
    pub path: String,
    /// Its format.
    pub format: Format,
    /// The parts added, in file order.
    pub parts: Vec<ImportedPart>,
    /// What the import changed or left out, in words.
    pub notes: Vec<String>,
}

/// One part an import added.
#[derive(Serialize, JsonSchema)]
pub struct ImportedPart {
    /// Its name in the scene.
    pub part: String,
    /// The object's name in the file (for a ZTL part, the name its list shows).
    pub source: String,
    /// Projects: the tool it came from, counting from 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<u32>,
    /// Vertex count.
    pub vertices: u64,
    /// Face count.
    pub faces: u64,
    /// The columns it holds (group, color, mask, uv0.u ...).
    pub columns: Vec<String>,
    /// Pass this as file.export's `scale` (and `offset`) to write the part at the size and
    /// place it had in the file.
    pub export_scale: f32,
    /// Pass this as file.export's `offset`.
    pub export_offset: [f32; 3],
    /// ZTL parts: the subdivision levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<Levels>,
}

/// The subdivision levels of a ZTL part, and which of them the import kept. Levels here count
/// from 0, the base, as the file does (level commands count from 1).
#[derive(Serialize, JsonSchema)]
pub struct Levels {
    /// How many levels the part has, the base included.
    pub count: u32,
    /// The level it was at when saved.
    pub saved_at: u32,
    /// The levels the file stores positions for.
    pub stored: Vec<u32>,
    /// The level the part shows: the level it was saved at when the levels are kept, else the
    /// highest stored one.
    pub imported: u32,
    /// Levels that exist but were not kept (when keeping them was not possible).
    pub not_kept: Vec<u32>,
}

/// One object ready to become a part.
pub(super) struct Object {
    pub source: String,
    pub tool: Option<u32>,
    /// Its mesh: for a ZTL part with levels, the top level.
    pub mesh: PlainMesh,
    /// The file's own export scale and offset (ZTL parts), in the file's axes.
    pub export: Option<(f32, [f32; 3])>,
    pub levels: Option<Levels>,
    /// What keeps a ZTL part's detail levels (decision 0014).
    pub stack: Option<Stack>,
}

/// A ZTL part's detail levels, beside its top level: the base faces and, when it was saved
/// below the top, that level as stored.
pub(super) struct Stack {
    pub base: Vec<[u32; 4]>,
    pub base_count: u32,
    pub count: usize,
    pub saved: Option<(usize, PlainMesh)>,
}

/// An import in progress: the parts added and the notes.
pub(super) struct Run<'a> {
    pub args: &'a FileImport,
    pub format: Format,
    /// How many objects the file holds (when it holds several, a given name starts each
    /// part's name).
    pub objects: usize,
    pub parts: Vec<ImportedPart>,
    pub notes: Vec<String>,
    /// Each part's file export transform, kept for the fit.
    export: Vec<(f32, [f32; 3])>,
}

impl Run<'_> {
    /// Turns `object` into a part and adds it to `doc`.
    pub fn add(&mut self, doc: &mut DocTxn, object: Object) -> Result<(), CommandError> {
        let args = self.args;
        let Object {
            source,
            tool,
            mut mesh,
            export,
            mut levels,
            stack,
        } = object;
        let keep = Keep {
            groups: args.groups,
            uvs: args.uvs,
            colors: args.colors,
            mask: args.mask,
        };
        keep_columns(&mut mesh, keep);
        place(&mut mesh, self.flip(), args.up, args.scale);
        let label = if source.is_empty() {
            "the file".to_string()
        } else {
            source.clone()
        };
        let merging = args.merge.unwrap_or(self.format == Format::Stl);
        let stack = self.levels_kept(stack, &mut levels, &label, keep, merging);
        if merging {
            let merged = merge(&mut mesh);
            if merged > 0 {
                self.notes.push(format!(
                    "{label}: merged {merged} vertices that shared a position"
                ));
            }
        }
        let dropped = drop_repeated(&mut mesh);
        if dropped > 0 {
            self.notes.push(format!(
                "{label}: dropped {dropped} faces that used a vertex twice"
            ));
        }
        if mesh.faces.is_empty() {
            self.notes
                .push(format!("{label}: no faces left, not imported"));
            return Ok(());
        }
        refuse_broken(&mesh, &label)?;
        let part = self.free_name(doc, &source);
        let shown = (stack.as_ref())
            .and_then(|s| s.saved.as_ref())
            .map_or(&mesh, |s| &s.1);
        let added = ImportedPart {
            vertices: shown.positions.len() as u64,
            faces: shown.faces.len() as u64,
            columns: mesh.attributes.keys().cloned().collect(),
            part,
            source,
            tool,
            export_scale: 1.0,
            export_offset: [0.0; 3],
            levels,
        };
        // The scene's own checks speak of the mesh as an argument; here the file is at fault.
        let held = match stack {
            Some(stack) => insert_levels(doc, &added.part, &mesh, &stack),
            None => doc.insert_part(&added.part, mesh, VertexOrder::AsGiven),
        };
        held.map_err(|e| CommandError {
            message: format!("{label} cannot become a part: {}", e.message),
            code: match e.code {
                ErrorCode::ArgsInvalid => ErrorCode::Precondition,
                code => code,
            },
            ..e
        })?;
        self.parts.push(added);
        self.export.push(export.unwrap_or((1.0, [0.0; 3])));
        Ok(())
    }

    /// A ZTL part's detail levels as kept: its saved level placed like its top, or `None` with a
    /// note (and `levels` saying so) when merging points or a base face using a point twice
    /// would renumber the points the levels are built on.
    fn levels_kept(
        &mut self,
        mut stack: Option<Stack>,
        levels: &mut Option<Levels>,
        label: &str,
        keep: Keep,
        merging: bool,
    ) -> Option<Stack> {
        let args = self.args;
        if let Some((_, saved)) = stack.as_mut().and_then(|s| s.saved.as_mut()) {
            keep_columns(saved, keep);
            place(saved, self.flip(), args.up, args.scale);
        }
        let repeats = |s: &Stack| {
            s.base.iter().any(|f| {
                let c = crate::mesh::corners(f);
                (0..c.len()).any(|i| c[i + 1..].contains(&c[i]))
            })
        };
        let Some(why) = (stack.as_ref()).and_then(|s| {
            (merging.then_some("merging points would renumber them"))
                .or(repeats(s).then_some("a base face uses a point twice"))
        }) else {
            return stack;
        };
        self.notes
            .push(format!("{label}: detail levels not kept: {why}"));
        if let (Some(l), Some(s)) = (levels.as_mut(), stack.take()) {
            l.imported = s.count as u32 - 1;
            l.not_kept = (0..l.imported).collect();
        }
        None
    }

    fn flip(&self) -> [bool; 3] {
        [self.args.flip_x, self.args.flip_y, self.args.flip_z]
    }

    /// The name for an object called `source`: the given name or the object's, else the
    /// file's, made fit and free by the document's naming rule (see `DocTxn::free_name`).
    /// Every part added so far is in the document already.
    fn free_name(&mut self, doc: &DocTxn, source: &str) -> String {
        let stem = std::path::Path::new(&self.args.path).file_stem();
        let file = stem.map_or("import".into(), |s| s.to_string_lossy().into_owned());
        let base = match (&self.args.name, source) {
            (Some(name), source) if self.objects > 1 && !source.is_empty() => {
                format!("{name}-{source}")
            }
            (Some(name), _) => name.clone(),
            (None, "") => file,
            (None, source) => source.to_string(),
        };
        let part = doc.free_name(&base);
        if let Some(why) = unfit(&base) {
            (self.notes).push(format!("{}, so the part is called '{part}'", why.reason));
        }
        part
    }

    /// Sets each part's export transform, after the fit (scale `f` about center `c`) if any:
    /// writing `p * scale + offset` and undoing the import's axes gives back the file.
    fn exports(&mut self, f: f32, c: [f32; 3]) {
        let k = self.args.scale;
        let (flip, up) = (self.flip(), self.args.up);
        for (part, &(s, o)) in self.parts.iter_mut().zip(&self.export) {
            let o = up.to_scene(mirror(o, flip));
            part.export_scale = 1.0 / (f * k * s);
            part.export_offset = std::array::from_fn(|i| (c[i] / k - o[i]) / s);
        }
    }
}

impl Command for FileImport {
    const NAME: &'static str = "file.import";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const READS_FILE: Option<&'static str> = Some("path");
    type Output = Imported;

    fn examples() -> Vec<Example<Self>> {
        let call = |path: &str| Self {
            path: path.into(),
            format: None,
            name: None,
            scale: 1.0,
            fit: false,
            fit_size: 2.0,
            flip_x: false,
            flip_y: false,
            flip_z: false,
            up: UpAxis::Y,
            merge: None,
            groups: true,
            materials_as_groups: false,
            group_polygons: false,
            uvs: true,
            colors: true,
            mask: true,
            tool: None,
            hash: None,
        };
        let stl = json!({ "path": "/nonexistent/bracket.stl", "up": "z", "scale": 0.1 });
        vec![
            Example::new(
                "import every part of a ZTL file (this path does not exist, so it is refused)",
                call("/nonexistent/head.ztl"),
            )
            .fails_with(ErrorCode::NotFound),
            Example::raw(
                "import a Z-up STL in millimeters as centimeters (refused: no such file)",
                stl,
            )
            .fails_with(ErrorCode::NotFound),
            Example::new(
                "a file whose format is not known from its name needs `format`",
                call("notes.txt"),
            )
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn check(&self, _: &Ctx) -> Result<(), Unavailable> {
        self.name.as_deref().and_then(unfit).map_or(Ok(()), Err)
    }

    fn run(self, cx: &mut Ctx) -> Result<Imported, CommandError> {
        cx.progress(None, format!("reading {}", self.path));
        self.import(&mut cx.doc, cx.job)
    }
}
register_command!(FileImport, "Import file");

impl FileImport {
    /// Reads the file into `doc` as new parts, reporting progress to `job` and stopping when it
    /// is cancelled. The Reference View reads its files through this too, into a document of
    /// its own (decision 0020).
    pub(crate) fn import(
        &self,
        doc: &mut DocTxn,
        job: Option<&JobCell>,
    ) -> Result<Imported, CommandError> {
        let format = Format::resolve(self.format, &self.path)?;
        for (value, path) in [(self.scale, "/scale"), (self.fit_size, "/fit_size")] {
            if !(value.is_finite() && value > 0.0) {
                return Err(errors::args_invalid(path, "must be a number above 0"));
            }
        }
        let file = File::open(&self.path).map_err(|e| io_error(&self.path, &e))?;
        let mut run = Run {
            args: self,
            format,
            objects: 0,
            parts: Vec::new(),
            notes: Vec::new(),
            export: Vec::new(),
        };
        let unreadable = |e: gs_io::ReadError| {
            error(
                ErrorCode::Precondition,
                format!("cannot read {} as {format:?}: {e}", self.path),
            )
        };
        if let Format::Ztl | Format::Zpr = format {
            ztl::import(file, &mut run, (doc, job), &unreadable)?;
        } else {
            let file = read(format, file, self).map_err(unreadable)?;
            run.objects = file.objects.len();
            run.notes.extend(notes(&file));
            for object in file.objects {
                let mesh = object.mesh;
                let (source, tool, export, levels) = (object.name, None, None, None);
                run.add(
                    doc,
                    Object {
                        source,
                        tool,
                        mesh,
                        export,
                        levels,
                        stack: None,
                    },
                )?;
            }
        }
        if run.parts.is_empty() {
            return Err(CommandError {
                hint: Some("check the file in another program".into()),
                ..error(
                    ErrorCode::Precondition,
                    format!("{} holds no faces to import", self.path),
                )
            });
        }
        let (f, c) = if self.fit {
            fit(doc, &run.parts, self.fit_size)?
        } else {
            (1.0, [0.0; 3])
        };
        run.exports(f, c);
        for part in &run.parts {
            let export = ExportTransform {
                scale: part.export_scale,
                offset: part.export_offset,
            };
            doc.set_export(&part.part, export)?;
        }
        let Run { parts, notes, .. } = run;
        Ok(Imported {
            path: self.path.clone(),
            format,
            parts,
            notes,
        })
    }
}

/// Adds a ZTL part with its detail levels: built from the base faces and the top level, with the
/// level it was saved at, when below the top, active and its stored points pending (decision
/// 0014, section 10).
fn insert_levels(
    doc: &mut DocTxn,
    name: &str,
    top: &PlainMesh,
    stack: &Stack,
) -> Result<(), CommandError> {
    let settings = gs_core::Settings {
        creases: Vec::new(),
        pin_corners: false,
        corners: crate::level::uv_corners(),
    };
    let refused = |e: gs_core::MeshError| error(ErrorCode::Precondition, e.to_string());
    let cfg = gs_core::NodeConfig::default();
    let mut levels = gs_core::Levels::from_top(
        &stack.base,
        stack.base_count,
        stack.count,
        top,
        &settings,
        cfg,
    )
    .map_err(refused)?;
    let mut stored = vec![top];
    if let Some((k, saved)) = &stack.saved {
        levels.set(*k).map_err(refused)?;
        levels.load_active(saved).map_err(refused)?;
        stored.push(saved);
    }
    doc.insert_levels(name, levels, &stored)
}

/// Refuses a mesh with values a part cannot hold (positions or attribute values that are not
/// finite, columns of the wrong length), naming each problem: the engine's own mesh check, in
/// its form that needs no edge tables, so it suits meshes of any size.
fn refuse_broken(mesh: &PlainMesh, label: &str) -> Result<(), CommandError> {
    let found = value_problems(mesh, None);
    if found.is_empty() {
        return Ok(());
    }
    let named: Vec<String> = (found.iter())
        .map(|p| {
            let kind = serde_json::to_value(p.kind).unwrap_or_default();
            let kind = kind.as_str().unwrap_or_default().to_string();
            match &p.attribute {
                Some(a) => format!("{kind} in {a} x{}", p.count),
                None => format!("{kind} x{}", p.count),
            }
        })
        .collect();
    Err(error(
        ErrorCode::Precondition,
        format!("{label} cannot become a part: {}", named.join(", ")),
    ))
}

/// Reads an open-format file.
fn read(format: Format, file: File, args: &FileImport) -> Result<MeshFile, gs_io::ReadError> {
    let options = ReadOptions {
        materials_as_groups: args.materials_as_groups,
        group_polygons: args.group_polygons,
    };
    let src = BufReader::new(file);
    match format {
        Format::Obj => gs_io::obj::read(src, &options),
        Format::Ply => gs_io::ply::read(src, &options),
        _ => gs_io::stl::read(src),
    }
}

/// A reader's notes in words.
fn notes(file: &MeshFile) -> Vec<String> {
    let n = &file.notes;
    let split = (n.polygons_split > 0).then(|| {
        format!(
            "split {} polygons with more than four corners into triangles",
            n.polygons_split
        )
    });
    let ignored = (n.ignored.iter()).map(|(what, count)| format!("ignored {what}: {count}"));
    split.into_iter().chain(ignored).collect()
}

/// Scales and centers the new parts together so their largest side is `size` (every level of a
/// part with detail levels moves together); returns the scale and the old center.
fn fit(
    doc: &mut DocTxn,
    parts: &[ImportedPart],
    size: f32,
) -> Result<(f32, [f32; 3]), CommandError> {
    let mut b = Aabb::EMPTY;
    for p in parts {
        // A part with detail levels is as big as its top, the truth for every level.
        let part = doc.get_part(&p.part)?;
        let store = part.levels().map_or(part.store(), gs_core::Levels::top);
        for n in 0..store.node_count() as NodeId {
            b = b.union(store.bounds(n));
        }
    }
    let side = (b.hi - b.lo).to_array().into_iter().fold(0.0f32, f32::max);
    let center = (b.lo + b.hi) * 0.5;
    // Faces with area give the parts a size.
    let f = size / side;
    for p in parts {
        doc.map_new_part(&p.part, |q| (q - center) * f)?;
    }
    Ok((f, center.to_array()))
}
