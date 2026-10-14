//! `material.copy`, `material.rename` and `material.delete` (decision 0043, section 4): one
//! palette step each, handed to the renderer. `material.set` (`set.rs`) reports as they do.

use std::sync::Arc;

use gs_core::material::Recipe;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{MaterialInfo, MaterialPalette, PaletteEntry, hand_over};
use crate::errors::args_invalid;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// A material after `material.copy`, `material.rename`, `material.delete` or `material.set`.
#[derive(Serialize, JsonSchema)]
pub struct MaterialChanged {
    /// The material: the new copy, the one renamed, deleted or changed.
    pub material: MaterialInfo,
    /// The active material's name; the first material's when the active one was deleted.
    pub active: String,
    /// Reference Views whose own look followed the material: to its new name, or back to the
    /// main view's look when it was deleted.
    pub references: Vec<String>,
}

/// `entry` with the title `title`, its recipe shared when the title stays.
fn titled(entry: &PaletteEntry, title: Option<String>) -> Arc<Recipe> {
    match title {
        Some(title) if title != entry.recipe.name => Arc::new(Recipe {
            name: title,
            ..(*entry.recipe).clone()
        }),
        _ => Arc::clone(&entry.recipe),
    }
}

/// `title` refused when it is blank.
fn checked_title(title: Option<String>) -> Result<Option<String>, CommandError> {
    match title {
        Some(t) if t.trim().is_empty() => Err(args_invalid("/title", "a title is not blank")),
        other => Ok(other),
    }
}

/// Records the palette's `materials` as a step of `command`, hands it over and reports
/// `material`.
pub(super) fn changed(
    cx: &mut Ctx,
    materials: Vec<PaletteEntry>,
    command: &'static str,
    material: &PaletteEntry,
) -> Result<MaterialChanged, CommandError> {
    let before = cx.palette.clone();
    cx.palette.record(materials, command, cx.origin);
    let references = hand_over(cx, before)?;
    Ok(MaterialChanged {
        material: MaterialInfo::of(material),
        active: cx.palette.now.active().name.clone(),
        references,
    })
}

/// Copy a material into a new user material after the other user materials (before Flat Color
/// and the built-ins shipped with it), to change without touching the original. The copy is not
/// chosen: `material.select` it to draw with it. One
/// step of the palette's own undo (`material.undo`).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialCopy {
    /// The material to copy: `#id`, `@place` (from 0) or its name. Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// The copy's name: unique, not starting with `#` or `@`, not `main`. Left out: the
    /// material's name and `-copy` (then `-copy-2` and on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The copy's title, what the window shows. Left out: the material's title and ` copy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Command for MaterialCopy {
    const NAME: &'static str = "material.copy";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = MaterialChanged;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("copy the active material", json!({})),
            Example::raw(
                "copy red wax as my-wax",
                json!({ "material": "red-wax", "name": "my-wax", "title": "My Wax" }),
            ),
            Example::raw("a name that is taken", json!({ "name": "red-wax" }))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MaterialChanged, CommandError> {
        let palette = &cx.palette.now;
        let source = match &self.material {
            Some(key) => palette.find(key)?,
            None => palette.active(),
        };
        let name = match self.name {
            Some(name) => name,
            // One more candidate than there are materials: one of them is free.
            None => (1..=palette.materials.len() + 1)
                .map(|n| match n {
                    1 => format!("{}-copy", source.name),
                    n => format!("{}-copy-{n}", source.name),
                })
                .find(|name| palette.get(name).is_none())
                .unwrap_or_default(),
        };
        palette.check_name(&name, None)?;
        let title = checked_title(self.title)?;
        let title = title.unwrap_or_else(|| format!("{} copy", source.recipe.name));
        let recipe = titled(source, Some(title));
        let mut materials = palette.materials.clone();
        let at = palette.user_end();
        let copy = PaletteEntry {
            id: cx.palette.next_id(),
            name,
            built_in: false,
            recipe,
        };
        materials.insert(at, copy.clone());
        changed(cx, materials, Self::NAME, &copy)
    }
}
register_command!(MaterialCopy, "Copy material");

/// Rename a material, retitle it, or both. Built-in materials may be renamed too; a Reference
/// View whose own look names it keeps naming it. One step of the palette's own undo.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialRename {
    /// The material: `#id`, `@place` (from 0) or its name. Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Its new name, what commands take: unique, not starting with `#` or `@`, not `main`.
    /// Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its new title, what the window shows. Left out: as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Command for MaterialRename {
    const NAME: &'static str = "material.rename";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = MaterialChanged;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "rename red wax",
                json!({ "material": "red-wax", "name": "wax", "title": "Wax" }),
            ),
            Example::raw(
                "name a copy",
                json!({ "material": "#14", "name": "my-gray" }),
            )
            .after("material.copy", json!({})),
            Example::raw("retitle the active material", json!({ "title": "My Gray" })),
            Example::raw("neither a name nor a title", json!({ "material": "@0" }))
                .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MaterialChanged, CommandError> {
        if self.name.is_none() && self.title.is_none() {
            return Err(args_invalid("", "give a new name, a new title or both"));
        }
        let palette = &cx.palette.now;
        let old = match &self.material {
            Some(key) => palette.find(key)?,
            None => palette.active(),
        };
        let name = self.name.unwrap_or_else(|| old.name.clone());
        palette.check_name(&name, Some(old.id))?;
        let renamed = PaletteEntry {
            name,
            recipe: titled(old, checked_title(self.title)?),
            ..old.clone()
        };
        if renamed == *old {
            let unchanged = MaterialChanged {
                material: MaterialInfo::of(old),
                active: palette.active().name.clone(),
                references: Vec::new(),
            };
            cx.noop_reason("the material already has that name and title, so no step is made");
            return Ok(unchanged);
        }
        let mut materials = palette.materials.clone();
        for m in materials.iter_mut().filter(|m| m.id == renamed.id) {
            m.clone_from(&renamed);
        }
        changed(cx, materials, Self::NAME, &renamed)
    }
}
register_command!(MaterialRename, "Rename material");

/// Delete a user material (one `material.copy` made or the user folder held). Built-in
/// materials stay. When it was the active one, the first material becomes active; a Reference
/// View that drew it draws the main view's look. One step of the palette's own undo, which
/// brings it back with its id.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialDelete {
    /// The material: `#id`, `@place` (from 0) or its name. Left out: the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

impl MaterialDelete {
    /// The material it names, or the active one.
    fn target<'p>(&self, palette: &'p MaterialPalette) -> Result<&'p PaletteEntry, CommandError> {
        match &self.material {
            Some(key) => palette.find(key),
            None => Ok(palette.active()),
        }
    }
}

impl Command for MaterialDelete {
    const NAME: &'static str = "material.delete";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = MaterialChanged;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("delete a copy", json!({ "material": "gray-copy" }))
                .after("material.copy", json!({})),
            Example::raw("a built-in material", json!({ "material": "red-wax" }))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    /// A built-in material stays, so the window's dry run greys Delete on one. A material that
    /// is not there is `run`'s to refuse, as not found.
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        match self.target(&cx.palette.now) {
            Ok(m) if m.built_in => Err(Unavailable::new(format!(
                "{} is built in, and built-in materials stay",
                m.name
            ))
            .fix("delete a copy instead; material.copy makes one")),
            _ => Ok(()),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<MaterialChanged, CommandError> {
        let palette = &cx.palette.now;
        let gone = self.target(palette)?.clone();
        let mut materials = palette.materials.clone();
        materials.retain(|m| m.id != gone.id);
        changed(cx, materials, Self::NAME, &gone)
    }
}
register_command!(MaterialDelete, "Delete material");
