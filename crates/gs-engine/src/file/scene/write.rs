//! Writing a scene file, which `file.save` and `file.quick_save` share (decision 0016): the
//! session section, the scene id, the format and free-space checks, the preview and the safe
//! write itself.

use std::path::Path;

use gs_io::scene::{self, FileKind, PartOut, RealDisk, SceneOut, Written};
use gs_protocol::{CommandError, ErrorCode};
use serde_json::Value;

use super::FreeSpace;
use super::model::{Held, LogPosition, SceneSection, SessionSection, held, kept, part_out, writer};
use crate::Ctx;
use crate::document::Document;
use crate::errors::{self, error};
use crate::view::{Labelled, MAIN, NamedView, RenderRequest, ViewSpec, saved};

/// Scenes above this many triangles get no preview: a preview uploads every Part to the GPU
/// and draws it, which a save should not wait on for a big scene (decision 0016, section 9).
const PREVIEW_TRIANGLES: usize = 2_000_000;
/// The preview's size in pixels.
const PREVIEW_SIZE: u32 = 256;

/// The session section of a save made now, with the open references' setups if `references`.
pub(super) fn session(cx: &Ctx, autosave: Option<String>, references: bool) -> SessionSection {
    let log = cx.config.log_path.as_ref().map(|path| LogPosition {
        path: path.display().to_string(),
        run: cx.log_at.0,
        records: cx.log_at.1,
        engine: cx.catalog.engine_version.clone(),
        catalog: cx.catalog.hash.clone(),
    });
    SessionSection {
        seed: Some(cx.config.seed),
        counter: Some(cx.seq + 1),
        file: autosave
            .as_ref()
            .and(cx.file.path.as_ref())
            .map(|p| p.display().to_string()),
        autosave,
        log,
        references: references.then(|| saved::setups(cx.views)).flatten(),
        isolate: Some(cx.doc.document().isolation().clone()).filter(|i| !i.chosen.is_empty()),
        active: cx.doc.document().active_id(),
        material: Some(cx.palette.now.active().id),
    }
}

/// The scene's id, or a new one made from the session seed, the command number and the path:
/// the same in a replay, different for every path (decision 0016, section 7).
pub(super) fn scene_id(cx: &Ctx, path: &Path) -> String {
    if let Some(id) = &cx.file.id {
        return id.clone();
    }
    let mut h = blake3::Hasher::new();
    h.update(b"gigasculpt scene id");
    h.update(&cx.config.seed.to_le_bytes());
    h.update(&cx.seq.to_le_bytes());
    h.update(path.as_os_str().as_encoded_bytes());
    h.finalize().to_hex()[..16].to_string()
}

/// Where a save writes.
#[derive(Clone, Copy)]
pub(super) struct Target<'a> {
    pub path: &'a Path,
    /// Where to keep the file it replaces.
    pub backup: Option<&'a Path>,
    /// The format of the file it replaces, when that file is the scene's own and replacing it
    /// with one that needs a newer reader was not asked for (decision 0016, section 3).
    pub older: Option<u32>,
}

/// Writes `names` of the scene to `target` safely, in the background pool, and, if
/// `references`, the parts references' frozen copies beside it. Returns what was written and
/// whether it has a preview.
pub(super) fn write(
    cx: &Ctx,
    target: Target,
    names: &[String],
    kind: FileKind,
    session: SessionSection,
    references: bool,
) -> Result<(Written, bool), CommandError> {
    let path = target.path;
    let doc = cx.doc.document();
    let frozen = if references {
        saved::frozen(cx.views)
    } else {
        Vec::new()
    };
    let kept: Vec<(String, Held)> = (frozen.iter())
        .flat_map(|(view, own)| {
            own.parts()
                .map(|(n, p)| (saved::chunk(view, n), kept(n, p)))
        })
        .collect();
    let held: Vec<Held> = (names.iter())
        .filter_map(|n| doc.get(n).map(|p| held(n, p)))
        .collect();
    let parts: Vec<PartOut> = held.iter().map(part_out).collect();
    let needs = scene::oldest_reader(&parts);
    if let Some(old) = target.older.filter(|&old| needs > old) {
        let why = match needs {
            4.. => "detail levels that keep their own UVs",
            3 => "frozen detail levels",
            _ => "detail levels",
        };
        // Naming `upgrade`, so the window asks, offering Save as beside it (review 0009, B3).
        return Err(CommandError {
            path: Some("/upgrade".into()),
            hint: Some("pass upgrade: true to replace it, or save to another path".into()),
            ..error(
                ErrorCode::Precondition,
                format!(
                    "{} was written in format {old}, and the scene now needs format {needs} \
                     ({why}): programs that open it now could not open it after this save",
                    path.display()
                ),
            )
        });
    }
    let records = held.iter().filter_map(|h| h.frozen.as_deref());
    check_space(
        path,
        &cx.config.free_space,
        (held.iter().chain(records)).chain(kept.iter().map(|k| &k.1)),
    )?;
    cx.progress(None, "drawing the preview");
    let preview = preview(cx, doc, names);
    let (scene, session) = sections(cx, path, kind, session);
    let out = SceneOut {
        kind,
        writer: writer(),
        preview: preview.as_deref(),
        scene,
        session,
        parts,
        references: (kept.iter())
            .map(|(c, h)| (c.clone(), part_out(h)))
            .collect(),
    };
    cx.progress(None, format!("writing {}", path.display()));
    let mut written = None;
    let result = gs_par::background(|| {
        scene::replace(&RealDisk, path, target.backup, |w| {
            written = Some(scene::write(w, &out)?);
            Ok(())
        })
    });
    result.map_err(|e| crate::file::file_refused("write", path, &e))?;
    let written = written.ok_or_else(|| errors::internal("the writer did not run"))?;
    Ok((written, preview.is_some()))
}

/// The scene and session sections of a file of `kind` at `path`: a Part file keeps neither the
/// background nor the material palette and its active material.
fn sections(cx: &Ctx, path: &Path, kind: FileKind, session: SessionSection) -> (Value, Value) {
    let scene = kind == FileKind::Scene;
    let section = SceneSection {
        id: Some(scene_id(cx, path)),
        issued: Some(cx.doc.document().issued()),
        background: scene.then(|| serde_json::to_value(cx.display.background).unwrap_or_default()),
        palette: scene.then(|| crate::material::scene::section(cx.palette)),
    };
    let session = SessionSection {
        material: session.material.filter(|_| scene),
        ..session
    };
    (
        serde_json::to_value(section).unwrap_or_default(),
        serde_json::to_value(session).unwrap_or_default(),
    )
}

/// Refuses a save that cannot fit: free space below a third of the raw columns (research 15
/// measured 5 MB per million triangles from about 18 MB raw), as `free` gives it.
fn check_space<'a>(
    path: &Path,
    free: &FreeSpace,
    parts: impl Iterator<Item = &'a Held<'a>>,
) -> Result<(), CommandError> {
    let raw: u64 = parts
        .map(|p| {
            let top = (0..p.store.node_count() as u32)
                .map(|k| {
                    let n = p.store.node(k);
                    (n.pos.len() * 12 + n.faces.len() * 16) as u64
                })
                .sum::<u64>();
            top + p.levels.as_ref().map_or(0, levels_raw)
        })
        .sum();
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    match free.of(folder) {
        Some(free) if free < raw / 3 => Err(CommandError {
            hint: Some("free some space, or save somewhere else".into()),
            ..error(
                ErrorCode::Precondition,
                format!(
                    "{} has {} MB free; the file needs about {} MB",
                    folder.display(),
                    free >> 20,
                    (raw / 3) >> 20
                ),
            )
        }),
        _ => Ok(()),
    }
}

/// A Part's levels' raw size beside its top: each lower level's numbering (4 bytes a point, 5
/// a face) and the active level's edited positions.
fn levels_raw(l: &gs_core::StoredLevels) -> u64 {
    let numberings: u64 = (l.lower.iter().flat_map(|level| &level.nodes))
        .map(|&[v, f]| u64::from(v) * 4 + u64::from(f) * 5)
        .sum();
    let edited = (l.pending.iter().flat_map(|p| &p.positions))
        .map(|(_, col)| col.len() as u64 * 12)
        .sum::<u64>();
    numberings + edited
}

/// The scene through the window's view (or from the front), 256 pixels, as PNG, when a
/// renderer is attached and the scene is small enough to draw.
fn preview(cx: &Ctx, doc: &Document, names: &[String]) -> Option<Vec<u8>> {
    let triangles: usize = (names.iter())
        .filter_map(|n| doc.get(n))
        .map(|p| p.store().face_count() * 2)
        .sum();
    if triangles == 0 || triangles > PREVIEW_TRIANGLES {
        return None;
    }
    let shown: Vec<&crate::document::Part> = names.iter().filter_map(|n| doc.get(n)).collect();
    let parts: Vec<&gs_core::MeshStore> = shown.iter().map(|part| part.store()).collect();
    // The window's view as it is, or, without one, the front view framing the scene.
    let camera = (cx.views.camera_of(MAIN, doc).ok()).or_else(|| {
        ViewSpec::Named(NamedView::Front)
            .resolve(doc, cx.views, 1.0)
            .ok()
    })?;
    let preview = Labelled {
        label: "preview".into(),
        camera,
    };
    let places = shown.iter().map(|part| part.transform()).collect();
    // The thumbnail keeps the default MatCap look, whatever the session displays.
    let request = RenderRequest::new(vec![preview], [PREVIEW_SIZE; 2], places);
    let shot = cx.render.with(|r| r.render(&parts, &request))?;
    shot.ok().map(|s| s.png)
}
