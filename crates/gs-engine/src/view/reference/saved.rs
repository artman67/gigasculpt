//! Reference Views in scene files (decision 0024, section 8). A save keeps, in the session
//! section's `references`, every open reference's source and setup (link, alignment, look, its
//! own camera exactly, its lock and the size it is seen at) and which one is active; a parts
//! reference's frozen copies go beside the scene as Parts named `<view>/<part>` (gs-io's
//! `reference` chunks), never into it. An open builds them again in its job and they land after
//! the document, inside its one logged call: a file reference reads its file again, and opens
//! empty with its setup kept when the file is missing or its content changed since the
//! reference opened. None of it is part of the document, its hash, its undo or its unsaved
//! changes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use gs_core::camera::{Camera, Preset};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::secondary::{Alignment, Link, RefLook, Scene, Secondary, Source};
use super::super::spec::ExactCamera;
use super::super::views::{Views, reference_number};
use super::open::read_file;
use crate::Ctx;
use crate::document::{Document, Part};

/// One open Reference View as a scene file keeps it.
#[derive(Serialize, Deserialize)]
struct Entry {
    /// `ref:1` ...
    view: String,
    source: Source,
    link: Link,
    align: Alignment,
    look: RefLook,
    /// Its own camera, exactly; left out while it is framed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<ExactCamera>,
    locked: bool,
    viewport: [u32; 2],
}

/// The open Reference Views, by number, and the active one.
#[derive(Serialize, Deserialize)]
struct Saved<E> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default)]
    open: Vec<E>,
}

/// The session section's `references` for a save made now; `None` when none is open.
pub(crate) fn setups(views: &Views) -> Option<Value> {
    let open: Vec<Entry> = (views.references().into_iter())
        .filter_map(|view| {
            let setup = views.secondary(&view)?;
            let seen = views.get(&view);
            Some(Entry {
                source: setup.source.clone(),
                link: setup.link,
                align: setup.align,
                look: setup.look.clone(),
                camera: seen.camera.map(ExactCamera::from),
                locked: seen.locked,
                viewport: seen.viewport,
                view,
            })
        })
        .collect();
    if open.is_empty() {
        return None;
    }
    let active = views.active_reference();
    serde_json::to_value(Saved { active, open }).ok()
}

/// What a save keeps beside the scene: each parts reference's own document, by its view's name.
pub(crate) fn frozen(views: &Views) -> Vec<(String, Arc<Document>)> {
    (views.references().into_iter())
        .filter_map(|view| match views.secondary(&view) {
            Some(Secondary {
                source: Source::Parts { .. },
                scene: Scene::Own(doc),
                ..
            }) => Some((view, doc.clone())),
            _ => None,
        })
        .collect()
}

/// The name the frozen copy of `part` in reference `view` is kept under.
pub(crate) fn chunk(view: &str, part: &str) -> String {
    format!("{view}/{part}")
}

/// The references an open brings back, built in its job, landed with the document.
pub(crate) struct Reopened {
    /// In the order they open: the active one last.
    views: Vec<Reopen>,
}

struct Reopen {
    view: String,
    setup: Secondary,
    camera: Option<Camera>,
    locked: bool,
    viewport: [u32; 2],
}

/// The references open after an open: the file's own, in place of any open before.
#[derive(Serialize, JsonSchema)]
pub struct ReopenedReferences {
    /// By number.
    pub open: Vec<String>,
    /// The active one.
    pub active: Option<String>,
    /// References that opened empty, their setup kept, and why: a file reference whose file is
    /// missing or changed, or frozen copies the file could not give back. `reference.open`
    /// with the file again fills one.
    pub empty: Vec<EmptyReference>,
}

/// A reference that opened empty.
#[derive(Serialize, JsonSchema)]
pub struct EmptyReference {
    pub view: String,
    pub reason: String,
}

/// Builds the references a file saved (`saved`, its session section's `references`): a file
/// reference read again, a parts reference from its frozen copies in `kept` (by the name
/// [`chunk`] gives), a scene reference live. An entry this version cannot read is left out and
/// named in `skipped`.
pub(crate) fn reopen(
    cx: &Ctx,
    saved: Option<Value>,
    kept: &mut BTreeMap<String, Result<Part, String>>,
    skipped: &mut Vec<String>,
) -> (Reopened, ReopenedReferences) {
    let saved: Saved<Value> = match saved.map(serde_json::from_value).transpose() {
        Ok(saved) => saved.unwrap_or(Saved {
            active: None,
            open: Vec::new(),
        }),
        Err(e) => {
            skipped.push(format!("the saved references, which do not read: {e}"));
            Saved {
                active: None,
                open: Vec::new(),
            }
        }
    };
    let mut views = Vec::new();
    let mut empty = Vec::new();
    let mut names = BTreeSet::new();
    for value in saved.open {
        let entry = match entry(value, &names) {
            Ok(entry) => entry,
            Err(why) => {
                skipped.push(why);
                continue;
            }
        };
        names.insert(entry.view.clone());
        let built = match &entry.source {
            Source::Scene => Ok(Scene::Live),
            Source::Parts { parts } => frozen_scene(&entry.view, parts, kept),
            Source::File { .. } => file_scene(cx, &entry.view, &entry.source),
        };
        let scene = built.unwrap_or_else(|reason| {
            empty.push(EmptyReference {
                view: entry.view.clone(),
                reason,
            });
            Scene::Own(Arc::new(Document::default()))
        });
        let camera = entry.camera.and_then(|c| c.camera("/camera").ok());
        views.push(Reopen {
            setup: Secondary {
                scene,
                source: entry.source,
                link: entry.link,
                align: entry.align,
                look: entry.look,
            },
            camera,
            locked: entry.locked,
            viewport: entry.viewport.map(|v| v.max(1)),
            view: entry.view,
        });
    }
    let mut open: Vec<String> = views.iter().map(|v| v.view.clone()).collect();
    open.sort_by_key(|v| reference_number(v));
    if let Some(at) = (views.iter()).position(|v| Some(&v.view) == saved.active.as_ref()) {
        let active = views.remove(at);
        views.push(active);
    }
    let report = ReopenedReferences {
        open,
        active: views.last().map(|v| v.view.clone()),
        empty,
    };
    (Reopened { views }, report)
}

/// A saved entry, checked: a reference's name not taken yet, an alignment and a camera that
/// are ones. Else what is wrong, for the open report.
fn entry(value: Value, taken: &BTreeSet<String>) -> Result<Entry, String> {
    let named = (value.get("view").and_then(Value::as_str)).unwrap_or("?");
    let named = format!("saved reference {named}, which does not read");
    let entry: Entry = serde_json::from_value(value).map_err(|e| format!("{named}: {e}"))?;
    let view = &entry.view;
    if reference_number(view).is_none() || taken.contains(view) {
        return Err(format!(
            "saved reference '{view}': not a reference's name, or saved twice"
        ));
    }
    let bad = |e: gs_protocol::CommandError| format!("saved reference {view}: {}", e.message);
    entry.align.check("/align").map_err(bad)?;
    if let Some(camera) = entry.camera {
        camera.camera("/camera").map_err(bad)?;
    }
    Ok(entry)
}

/// A parts reference's scene from its frozen copies, or why it opens empty.
fn frozen_scene(
    view: &str,
    parts: &[String],
    kept: &mut BTreeMap<String, Result<Part, String>>,
) -> Result<Scene, String> {
    let mut own = Document::default();
    for name in parts {
        match kept.remove(&chunk(view, name)) {
            Some(Ok(part)) => {
                own.swap(name, Some(Box::new(part)));
            }
            Some(Err(why)) => return Err(format!("its frozen copy of '{name}' is damaged: {why}")),
            None => return Err(format!("the file holds no frozen copy of '{name}'")),
        }
    }
    Ok(Scene::Own(Arc::new(own)))
}

/// A file reference's scene, its file read again, or why it opens empty: the file is missing,
/// its content changed since the reference opened, or it no longer reads.
fn file_scene(cx: &Ctx, view: &str, source: &Source) -> Result<Scene, String> {
    let Source::File { path, hash, .. } = source else {
        return Ok(Scene::Live);
    };
    match crate::file::content_hash(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("its file {path} is missing"));
        }
        Err(e) => return Err(format!("its file {path} cannot be read: {e}")),
        Ok(now) if &now != hash => {
            return Err(format!(
                "its file {path} changed since the reference opened (its content hash differs)"
            ));
        }
        Ok(_) => {}
    }
    cx.progress(None, format!("reading {view}'s file {path}"));
    let (own, _notes) = read_file(source, cx.job)
        .map_err(|e| format!("its file {path} does not read: {}", e.message))?;
    Ok(Scene::Own(Arc::new(own)))
}

/// Replaces the session's open references with `r`'s, each with its own camera, lock and size;
/// the active one opens last, so it is active.
pub(crate) fn land(views: &mut Views, r: Reopened) {
    for open in views.references() {
        views.close_reference(&open);
    }
    for v in r.views {
        let placeholder = v.camera.unwrap_or_else(|| Camera::preset(Preset::Front));
        views.open(&v.view, v.setup, v.viewport, placeholder);
        views.settle(&v.view, |seen| {
            seen.camera = v.camera;
            seen.locked = v.locked;
        });
    }
}
