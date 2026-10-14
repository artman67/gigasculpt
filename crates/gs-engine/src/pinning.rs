//! Pinning context-dependent arguments before a command runs: which part, which new part name,
//! which random seed, which file content, which view, image size, pivot and camera. Done here,
//! once, for every command that declares it (`PartArg`, `Command::RANDOM`,
//! `Command::READS_FILE`, `Command::VIEW`, `Command::CAMERA`), so commands never repeat it and
//! the log records exactly what ran.

use gs_protocol::{CommandError, ErrorCode};
use serde_json::{Map, Value};

use crate::command::{CommandSpec, PartArg, ViewArg};
use crate::document::{DocTxn, unfit};
use crate::errors::{self, error};
use crate::{Ctx, Unavailable};

/// Fills in the arguments `spec` declares as context-dependent. Arguments of the wrong type
/// are left alone for the schema check to report.
pub(crate) fn pin(spec: &CommandSpec, cx: &Ctx, args: &mut Value) -> Result<(), CommandError> {
    let Value::Object(map) = args else {
        return Ok(());
    };
    let doc = &cx.doc;
    match spec.part {
        PartArg::None => {}
        PartArg::Existing => pin_existing(doc, map)?,
        PartArg::New(base) => pin_new(doc, map, base)?,
    }
    if spec.random && map.get("seed").is_none_or(Value::is_null) {
        map.insert("seed".into(), cx.rng().seed().into());
    }
    if let Some(path) = spec.reads_file {
        pin_file(map, path)?;
    }
    if spec.view != ViewArg::None {
        crate::view::pin::view(spec.view, cx, map)?;
    }
    if let Some(arg) = spec.camera {
        crate::view::pin::camera(cx, map, arg)?;
    }
    Ok(())
}

/// `hash` must be the content hash of the file the argument `arg` names; left out, it becomes
/// that. A file that cannot be read is left for the command to report.
fn pin_file(map: &mut Map<String, Value>, arg: &str) -> Result<(), CommandError> {
    let Some(Value::String(path)) = map.get(arg) else {
        return Ok(());
    };
    let Ok(hash) = crate::file::content_hash(path) else {
        return Ok(());
    };
    match map.get("hash") {
        Some(Value::String(expected)) if *expected != hash => {
            let reason = Unavailable::new(format!(
                "the file at {path} changed: its content hash is {hash}, the call expected \
                 {expected}"
            ))
            .fix("restore the file the call was made with, or leave hash out to read it as it is now");
            Err(errors::unavailable(reason))
        }
        Some(Value::Null) | None => {
            map.insert("hash".into(), hash.into());
            Ok(())
        }
        Some(_) => Ok(()),
    }
}

/// The names of the arguments `spec` declares as pinned, as the catalog lists them. (A read
/// file's `hash` is pinned too but not listed: the catalog's check of pinned arguments knows
/// parts, names, seeds and the view arguments only.)
pub(crate) fn pinned_names(spec: &CommandSpec) -> Vec<String> {
    let part = match spec.part {
        PartArg::None => None,
        PartArg::Existing => Some("part"),
        PartArg::New(_) => Some("name"),
    };
    let seed = spec.random.then_some("seed");
    let view = match spec.view {
        ViewArg::None => &[][..],
        ViewArg::Name | ViewArg::Reference => &["view"][..],
        ViewArg::View | ViewArg::NewReference => &["view", "viewport"][..],
        ViewArg::Pivot => &["view", "viewport", "pivot"][..],
        ViewArg::Frame => &["view", "viewport", "point"][..],
    };
    let camera = spec.camera;
    (part
        .into_iter()
        .chain(seed)
        .chain(view.iter().copied())
        .chain(camera))
    .map(str::to_string)
    .collect()
}

/// `part` must refer to an existing part (`#id`, `@place`, `@active` or a name), and is pinned to
/// its name; left out, it means the active part (decision 0033).
pub(crate) fn pin_existing(doc: &DocTxn, map: &mut Map<String, Value>) -> Result<(), CommandError> {
    match map.get("part") {
        Some(Value::String(part)) => match doc.resolve(part) {
            Some(name) => {
                map.insert("part".into(), name.into());
                Ok(())
            }
            None => Err(errors::not_found("part", part, doc.part_names())),
        },
        Some(Value::Null) | None => {
            map.insert("part".into(), active_part(doc)?.into());
            Ok(())
        }
        Some(_) => Ok(()),
    }
}

/// `name` must be free and must not read as a part id or place (`#`, `@`); left out, it becomes
/// the first free one of `base`, `base-2`, ...
fn pin_new(doc: &DocTxn, map: &mut Map<String, Value>, base: &str) -> Result<(), CommandError> {
    match map.get("name") {
        Some(Value::String(name)) if let Some(reason) = unfit(name) => {
            Err(errors::unavailable(reason))
        }
        Some(Value::String(name)) if doc.contains(name) => {
            let reason = Unavailable::new(format!("a part named '{name}' already exists"))
                .fix("choose another name, or leave it out to get a free one");
            Err(errors::unavailable(reason))
        }
        Some(Value::Null) | None => {
            map.insert("name".into(), doc.free_name(base).into());
            Ok(())
        }
        Some(_) => Ok(()),
    }
}

/// The part a part argument left out means: the active part, which there is unless the scene
/// is empty (decision 0033).
pub(crate) fn active_part(doc: &DocTxn) -> Result<String, CommandError> {
    doc.active().ok_or_else(|| CommandError {
        hint: Some("create one first, for example with primitive.sphere".into()),
        ..error(ErrorCode::NotFound, "the scene has no parts")
    })
}
