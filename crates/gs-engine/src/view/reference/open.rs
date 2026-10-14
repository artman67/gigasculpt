//! `reference.open`.

use std::sync::Arc;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::MAIN;
use super::super::secondary::{Alignment, Link, RefLook, Scene, Secondary, Source};
use super::super::spec::ViewName;
use super::{ReferenceShown, checked_look, fit_sphere, name, shown, sphere};
use crate::command::ViewArg;
use crate::document::{DocTxn, Document};
use crate::errors;
use crate::file::{FileImport, Format, UpAxis};
use crate::job::JobCell;
use crate::{Command, Ctx, Example, register_command};

/// Open a Reference View: a second 3D view of a reference model that turns with the main view,
/// so it can be copied from the same angle while sculpting.
///
/// Give one source: `file` (OBJ, PLY, STL, or a ZTL tool or ZPR project, read as
/// `file.import` reads it), `parts` (frozen copies of parts of the scene) or `scene` (the
/// scene itself, live). The reference never enters the scene, its undo or its hash. Its camera
/// starts looking from main's angle, framing the whole reference; by default it turns with main
/// while its zoom and slide stay its own (`link`). To line up a reference that faces another
/// way: unlink rotation (`reference.link`), turn it to show the side main shows, then
/// `reference.align`. It becomes the active reference (`reference.next` moves on); the window
/// shows every open one. Logged; never undoable.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceOpen {
    /// "ref:1", "ref:2" ... Left out: ref:1, or the next free number. An open one by that name
    /// is replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size it is seen at, `[width, height]` in pixels; its first framing fits this
    /// shape. Left out: the size the window shows references at, or 1024 by 1024.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// A file to show. A relative path starts at the engine's working folder: give an absolute
    /// path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub file: Option<String>,
    /// The file's format. Left out: from its extension (obj, ply, stl, ztl, zpr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    /// The file's up axis.
    #[serde(default)]
    pub up: UpAxis,
    /// Projects (.zpr): show only this tool, counting from 0. Left out: every tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<u32>,
    /// The BLAKE3 hash of the file's bytes. Filled in by the engine before it runs, so the log
    /// records which content was read; a call whose hash no longer matches is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Parts of the scene to show, as they are now (later edits do not show): a name, `#id` or
    /// `@place`, or a list of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Parts>,
    /// Show the scene itself, live: a second view of what is sculpted, for example zoomed out
    /// whole while main works close.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scene: bool,
    /// What follows main. Left out: rotation only.
    #[serde(default)]
    pub link: Link,
    /// How it is drawn. Left out: the main view's MatCap, no wireframe.
    #[serde(default)]
    pub look: RefLook,
}

/// One part, or several.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Parts {
    One(PartRef),
    Several(Vec<PartRef>),
}

impl Parts {
    fn list(&self) -> Vec<&str> {
        match self {
            Parts::One(p) => vec![p.0.as_str()],
            Parts::Several(ps) => ps.iter().map(|p| p.0.as_str()).collect(),
        }
    }
}

impl ReferenceOpen {
    /// The scene it shows and where that came from, plus what reading a file noted.
    fn source(&self, cx: &Ctx) -> Result<(Scene, Source, Vec<String>), CommandError> {
        let given = [self.file.is_some(), self.parts.is_some(), self.scene];
        if given.iter().filter(|g| **g).count() != 1 {
            return Err(CommandError {
                hint: Some("for example file=/abs/head.obj, parts=head, or scene=true".into()),
                ..errors::args_invalid("", "give exactly one of file, parts and scene")
            });
        }
        if self.scene {
            return Ok((Scene::Live, Source::Scene, Vec::new()));
        }
        let mut own = Document::default();
        if let Some(parts) = &self.parts {
            let mut names = Vec::new();
            for part in parts.list() {
                let found = cx.doc.resolve(part).and_then(|n| {
                    let shared = cx.doc.document().get(&n)?.share();
                    Some((n, shared))
                });
                let Some((name, shared)) = found else {
                    return Err(errors::not_found("part", part, cx.doc.part_names()));
                };
                own.swap(&name, Some(Box::new(shared)));
                names.push(name);
            }
            return Ok((
                Scene::Own(Arc::new(own)),
                Source::Parts { parts: names },
                vec![],
            ));
        }
        let path = self.file.clone().unwrap_or_default();
        // A folder asks for a file name, as Open does, named as this command's `file`.
        crate::file::not_a_folder(std::path::Path::new(&path)).map_err(|e| CommandError {
            path: Some("/file".into()),
            ..e
        })?;
        let source = Source::File {
            path,
            hash: self.hash.clone().unwrap_or_default(),
            format: self.format,
            up: self.up,
            tool: self.tool,
        };
        let (own, notes) = read_file(&source, cx.job)?;
        Ok((Scene::Own(Arc::new(own)), source, notes))
    }
}

/// Reads a file reference's file into a document of its own, as `file.import` reads it with
/// `source`'s options (only the shape: a reference is shaded, never painted or masked); also
/// what reading it noted. Anything but a file source reads as nothing.
pub(super) fn read_file(
    source: &Source,
    job: Option<&JobCell>,
) -> Result<(Document, Vec<String>), CommandError> {
    let mut own = Document::default();
    let Source::File {
        path,
        hash,
        format,
        up,
        tool,
    } = source
    else {
        return Ok((own, Vec::new()));
    };
    let read = FileImport {
        path: path.clone(),
        format: *format,
        name: None,
        scale: 1.0,
        fit: false,
        fit_size: 2.0,
        flip_x: false,
        flip_y: false,
        flip_z: false,
        up: *up,
        merge: None,
        groups: false,
        materials_as_groups: false,
        group_polygons: false,
        uvs: false,
        colors: false,
        mask: false,
        tool: *tool,
        hash: Some(hash.clone()),
    };
    let mut txn = DocTxn::new(&mut own);
    let imported = read.import(&mut txn, job)?;
    txn.finish()?;
    Ok((own, imported.notes))
}

/// What `reference.open` answers: the reference, and what reading its file noted.
#[derive(Serialize, JsonSchema)]
pub struct Opened {
    #[serde(flatten)]
    pub shown: ReferenceShown,
    /// What reading the file changed or left out, in words.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl Command for ReferenceOpen {
    const NAME: &'static str = "reference.open";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Short;
    const VIEW: ViewArg = ViewArg::NewReference;
    const READS_FILE: Option<&'static str> = Some("file");
    type Output = Opened;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "the scene itself, live, in a second view",
                json!({ "scene": true }),
            )
            .after("primitive.sphere", json!({})),
            Example::raw(
                "a frozen copy of one part, turning and zooming with main",
                json!({ "parts": "cube", "link": { "rotation": true, "zoom": true } }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({ "center": [3, 0, 0] })),
            Example::raw(
                "a reference file in its own view (this path does not exist, so it is refused)",
                json!({ "file": "/nonexistent/anatomy.obj", "look": { "matcap": "red-wax" } }),
            )
            .fails_with(ErrorCode::NotFound),
            Example::raw("no source given", json!({})).fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Opened, CommandError> {
        let view = name(self.view.as_ref())?.to_string();
        let [w, h] = super::super::commands::size(self.viewport)?;
        let look = checked_look(cx, self.look.clone())?;
        let (scene, source, notes) = self.source(cx)?;
        // It starts looking from main's angle through main's lens, framing the sphere around
        // all of itself, which stays in the picture at every linked turn.
        let doc = cx.doc.document();
        let main = cx.views.camera_of(MAIN, doc)?;
        let mut own = Alignment::IDENTITY.follow(&main);
        if let Some(around) = sphere(scene.doc(doc)) {
            fit_sphere(&mut own, around, w as f32 / h as f32);
        }
        let setup = Secondary {
            scene,
            source,
            link: self.link,
            align: Alignment::IDENTITY,
            look,
        };
        // A gesture open on this name was closed before the call ran (`end_gestures_before`).
        cx.views.open(&view, setup, [w, h], own);
        Ok(Opened {
            shown: shown(cx, &view)?,
            notes,
        })
    }
}
register_command!(ReferenceOpen, "Open reference");
