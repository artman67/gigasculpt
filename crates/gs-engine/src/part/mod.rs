//! Parts: creating them from plain mesh data, reading them back and listing them, and the part
//! list's commands (decision 0028): copying, deleting, renaming and moving parts, their eyes,
//! Isolate and Show Paint (decision 0039), and splitting, merging, extracting and remeshing them; and writing a part's transform
//! into its points (decision 0029). Every list edit is one undo step that swaps whole parts or
//! changes their properties; the geometry is `gs_geo::parts`, pure functions.

mod apply;
mod copy;
mod create;
mod delete;
mod extract;
mod get;
mod list;
mod merge;
mod mirror;
mod operation;
mod paint_visible;
mod remesh;
mod rename;
mod select;
mod split;
mod visible;

pub(crate) use merge::empty;
pub(crate) use mirror::{mirrored, plane};
pub(crate) use visible::drawn;

use std::collections::BTreeMap;

use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CommandError, GROUP, HIDE, MASK, NO_VERTEX, PartRef,
    PlainMesh,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::document::{VertexOrder, unfit};
use crate::{Ctx, Unavailable, errors};

/// A part that was added.
#[derive(Serialize, JsonSchema)]
pub struct NewPart {
    /// Its name.
    pub part: String,
    /// Its stable id: `#id` names it in any command, and it never changes.
    pub id: u64,
    /// Its place in the part list, from 0: `@index` names it.
    pub index: u64,
    /// Its vertex count.
    pub vertices: u64,
    /// Its face count.
    pub faces: u64,
}

/// One triangle in the z = 0 plane, for examples.
pub(crate) fn triangle() -> PlainMesh {
    PlainMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        faces: vec![[0, 1, 2, NO_VERTEX]],
        attributes: BTreeMap::new(),
    }
}

/// Two quads side by side in the z = 0 plane, for examples: face groups 1 and 2, the second
/// face hidden, and the mask on its far edge (that face's corner mean is 0.5).
pub(crate) fn strip() -> PlainMesh {
    let column = |domain, values| Attribute { domain, values };
    let attributes = [
        (GROUP, column(AttrDomain::Face, AttrValues::U32(vec![1, 2]))),
        (HIDE, column(AttrDomain::Face, AttrValues::U32(vec![0, 1]))),
        (
            MASK,
            column(
                AttrDomain::Vertex,
                AttrValues::F32(vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
            ),
        ),
    ];
    PlainMesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
        ],
        faces: vec![[0, 1, 4, 3], [1, 2, 5, 4]],
        attributes: attributes.map(|(name, c)| (name.to_string(), c)).into(),
    }
}

/// The default mask threshold: a face is masked when its corners' mean mask is at least this.
pub(crate) fn half() -> f32 {
    0.5
}

/// Adds a new part under the name the engine pinned (see `PartArg::New`), numbered as `order`
/// says. A missing name means the command's `PART` flag does not match its arguments: a bug,
/// reported as `E_INTERNAL`.
pub(crate) fn add_part(
    cx: &mut Ctx,
    name: Option<String>,
    mesh: PlainMesh,
    order: VertexOrder,
) -> Result<NewPart, CommandError> {
    let part =
        name.ok_or_else(|| errors::internal("the engine did not pin the new part's name"))?;
    cx.doc.insert_part(&part, mesh, order)?;
    report(cx, &part)
}

/// Part `name` as a command reports it: its id, place and size.
pub(crate) fn report(cx: &Ctx, name: &str) -> Result<NewPart, CommandError> {
    let (id, index) = (cx.doc.place(name)).ok_or_else(|| errors::internal("the part has no id"))?;
    let store = cx.doc.get_part(name)?.store();
    Ok(NewPart {
        part: name.to_string(),
        id,
        index: index as u64,
        vertices: store.vertex_count() as u64,
        faces: store.face_count() as u64,
    })
}

/// Pins a part reference (`#id`, `@place`, `@active` or a name) to the name of the part it means.
pub(crate) fn pin_ref(cx: &Ctx, r: &mut PartRef) -> Result<(), CommandError> {
    let name = cx.doc.resolve(&r.0);
    r.0 = name.ok_or_else(|| errors::not_found("part", &r.0, cx.doc.part_names()))?;
    Ok(())
}

/// Pins the name of a new part: a given name must be free and fit; left out, it becomes the
/// first free one of `base`, `base-2`, ... (as `PartArg::New` does for a command's `name`).
pub(crate) fn pin_name(
    cx: &Ctx,
    name: &mut Option<String>,
    base: &str,
) -> Result<(), CommandError> {
    if let Some(given) = name {
        return refused_name(cx, given).map_or(Ok(()), |r| Err(errors::unavailable(r)));
    }
    *name = Some(cx.doc.free_name(base));
    Ok(())
}

/// Pins the name of a copy of `part` (see [`pin_name`]): left out, the part's own name with
/// -2, -3, ...
pub(crate) fn pin_copy_name(
    cx: &Ctx,
    part: Option<&PartRef>,
    name: &mut Option<String>,
) -> Result<(), CommandError> {
    pin_name(cx, name, part.map_or("part", |p| p.0.as_str()))
}

/// Pins an optional part argument as `PartArg::Existing` pins `part`: a reference to the name
/// of the part it means; left out, the active part (decision 0033).
pub(crate) fn pin_part(cx: &Ctx, part: &mut Option<PartRef>) -> Result<(), CommandError> {
    if let Some(r) = part {
        return pin_ref(cx, r);
    }
    *part = Some(PartRef(crate::pinning::active_part(&cx.doc)?));
    Ok(())
}

/// Pins every reference in `parts` (see [`pin_ref`]).
pub(crate) fn pin_refs(cx: &Ctx, parts: &mut [PartRef]) -> Result<(), CommandError> {
    parts.iter_mut().try_for_each(|r| pin_ref(cx, r))
}

/// The pinned names of `parts`.
pub(crate) fn names(parts: &[PartRef]) -> Vec<String> {
    parts.iter().map(|r| r.0.clone()).collect()
}

/// Why `name` cannot name a part that does not exist yet: it reads as an id or a place, or a
/// part has it (names stay unique, decision 0015).
pub(crate) fn refused_name(cx: &Ctx, name: &str) -> Option<Unavailable> {
    unfit(name).or_else(|| cx.doc.contains(name).then(|| taken(name)))
}

/// Why a new part cannot be named `name`: a part has it. It names `name` as its path, so the
/// window finds it by that rather than by its words.
pub(crate) fn taken(name: &str) -> Unavailable {
    Unavailable::new(format!("a part named '{name}' already exists"))
        .fix("choose another name, or leave it out to get a free one")
        .asks("name")
}

/// A pinned name, or `E_INTERNAL` if the command's `pin` did not pin it.
pub(crate) fn pinned_name(name: Option<String>) -> Result<String, CommandError> {
    name.ok_or_else(|| errors::internal("the command did not pin the new part's name"))
}

/// Puts the parts `new`, in this order, directly below part `anchor` in the list; every other
/// part keeps its place among the rest.
pub(crate) fn place_below(cx: &mut Ctx, anchor: &str, new: &[String]) -> Result<(), CommandError> {
    let mut order: Vec<String> = (cx.doc.list_names().into_iter())
        .filter(|name| !new.contains(name))
        .collect();
    let at = (order.iter().position(|name| name == anchor)).map_or(order.len(), |i| i + 1);
    order.splice(at..at, new.iter().cloned());
    cx.doc.arrange(&order)
}

/// The parts that show (decision 0028), in list order.
pub(crate) fn shown(cx: &Ctx) -> Vec<String> {
    let doc = cx.doc.document();
    let showing = doc.showing();
    (doc.list().into_iter())
        .filter(|(_, part)| part.shown(showing))
        .map(|(name, _)| name.to_string())
        .collect()
}
