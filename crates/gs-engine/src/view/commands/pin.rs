//! Pinning the view arguments (called from `pinning`): which view, the size it is seen at, the
//! point it turns about, and a view named as a camera. After pinning a log line says exactly
//! which camera it changed and how, so it replays the same whoever replays it, with or without a
//! window.

use gs_protocol::{CommandError, ErrorCode};
use serde_json::{Map, Value, json};

use super::super::spec::{ExactCamera, NamedView};
use super::super::views::own_view;
use super::frame::{FrameTarget, NO_LAST_EDIT};
use crate::Ctx;
use crate::command::ViewArg;
use crate::errors::error;

/// `view` names a view (left out: the caller's own); beyond [`ViewArg::Name`], `viewport` left
/// out becomes the size it is seen at; with [`ViewArg::Pivot`], `pivot` left out becomes the
/// point it turns about by its rule, from the pixel `at` for the `under-pointer` rule; with
/// [`ViewArg::Frame`], a `target` left out becomes what the press frames, and a `last-edit`
/// target the `point` target at its last edit point.
pub(crate) fn view(
    kind: ViewArg,
    cx: &Ctx,
    map: &mut Map<String, Value>,
) -> Result<(), CommandError> {
    if let ViewArg::Reference | ViewArg::NewReference = kind {
        return reference(kind, cx, map);
    }
    let name = match map.get("view") {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Null) | None => {
            let own = own_view(cx.origin);
            map.insert("view".into(), own.clone().into());
            own
        }
        Some(_) => return Ok(()),
    };
    cx.views.check_name(&name)?;
    let view = cx.views.get(&name);
    if kind != ViewArg::Name && map.get("viewport").is_none_or(Value::is_null) {
        map.insert("viewport".into(), json!(view.viewport));
    }
    if kind == ViewArg::Pivot && map.get("pivot").is_none_or(Value::is_null) {
        let at = map.get("at").and_then(|a| {
            let n = |i: usize| a.get(i).and_then(Value::as_f64).map(|v| v as f32);
            Some([n(0)?, n(1)?])
        });
        let pivot = cx.views.pivot(&name, cx.doc.document(), at);
        map.insert("pivot".into(), json!(pivot));
    }
    if kind == ViewArg::Frame && map.get("target").is_none_or(Value::is_null) {
        super::frame::press(cx, &name, map);
    }
    let last_edit = Some(json!(FrameTarget::LastEdit));
    if kind == ViewArg::Frame && map.get("target").cloned() == last_edit {
        let point = view
            .last_edit
            .ok_or_else(|| error(ErrorCode::Precondition, NO_LAST_EDIT))?;
        map.insert("target".into(), json!(FrameTarget::Point));
        map.insert("point".into(), json!(point));
    }
    Ok(())
}

/// The Reference View arguments (decision 0020): `view` left out becomes the active reference
/// ([`ViewArg::Reference`], decision 0024; with none open it stays out, for the command's check to
/// refuse after the arguments are checked), or for one to open the lowest free `ref:<n>`, whose
/// `viewport` left out becomes the size it is seen at ([`ViewArg::NewReference`]).
fn reference(kind: ViewArg, cx: &Ctx, map: &mut Map<String, Value>) -> Result<(), CommandError> {
    let opening = kind == ViewArg::NewReference;
    let name = match map.get("view") {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Null) | None if opening => cx.views.free_reference(),
        Some(Value::Null) | None => match cx.views.active_reference() {
            Some(name) => name,
            None => return Ok(()),
        },
        Some(_) => return Ok(()),
    };
    if super::super::views::reference_number(&name).is_none() {
        let message = format!("{name} is no Reference View: they are named ref:1, ref:2 ...");
        return Err(crate::errors::args_invalid("/view", message));
    }
    if !opening {
        cx.views.check_name(&name)?;
    }
    map.insert("view".into(), name.clone().into());
    if opening && map.get("viewport").is_none_or(Value::is_null) {
        map.insert("viewport".into(), json!(cx.views.new_reference_size()));
    }
    Ok(())
}

/// A session view named in the camera argument `arg` becomes that view's exact camera.
/// Standard view names stay: they mean the same camera whenever they are read.
pub(crate) fn camera(
    cx: &Ctx,
    map: &mut Map<String, Value>,
    arg: &str,
) -> Result<(), CommandError> {
    let Some(Value::String(name)) = map.get(arg) else {
        return Ok(());
    };
    if serde_json::from_value::<NamedView>(json!(name)).is_ok() {
        return Ok(());
    }
    let camera = cx.views.camera_of(name, cx.doc.document())?;
    let exact = serde_json::to_value(ExactCamera::from(camera)).unwrap_or_default();
    map.insert(arg.into(), exact);
    Ok(())
}
