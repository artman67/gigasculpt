//! Materials (catalog items `material.palette` and `material.matcap-shading`, decision 0043):
//! the session's palette, every material's recipe in it, and its commands.
//!
//! The engine holds the palette as it holds the brush and display settings, so it lists,
//! changes and undoes without a renderer. Whenever a command changes the palette or the active
//! material, the engine hands the whole palette to the renderer
//! ([`RenderHost::materials`](crate::RenderHost::materials)), which draws every look from it.
//! Palette commands are View commands: logged, so a replay draws the same pictures, and never
//! in the scene's undo; the palette keeps its own (`material.undo`, `material.redo`). A scene
//! file keeps the palette and the active material (`scene`, decision 0043's section 7).

mod edit;
mod folder;
mod palette;
pub(crate) mod scene;
mod set;
mod undo;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use palette::PaletteState;
pub use palette::{MaterialPalette, PaletteEntry};
pub use set::MaterialSet;

use crate::errors::error;
use crate::{Command, Ctx, Example, register_command};

/// The palette: every material, the active one, and recipe files that were not read.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct Materials {
    /// Each material, in palette order (`@0` first).
    pub looks: Vec<MaterialInfo>,
    /// The active material's name: what MatCap and Lit shading draw on every part.
    pub selected: String,
    /// Recipe files that were not read, each with the key it got wrong.
    pub refused: Vec<RefusedMaterial>,
}

/// One material.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MaterialInfo {
    /// Its id, `#n` in commands: never handed out again.
    pub id: u64,
    /// What commands take, for example `red-wax`.
    pub name: String,
    /// What the user reads, for example `MatCap Red Wax`.
    pub title: String,
    /// Shipped with the program: it cannot be deleted.
    pub built_in: bool,
    /// The catalog item its recipe implements, for example `material.matcap-red-wax`.
    pub catalog_id: String,
    /// How it draws: `matcap` (a baked look), `standard` (a standard material) or `flat` (the
    /// paint times its base color, unlit, with no settings).
    pub kind: String,
    /// The environment that lights it, for example `studio`.
    pub environment: String,
    /// Its physical keys as `material.set` takes them, every one given, colors linear.
    pub values: MaterialSet,
}

impl MaterialInfo {
    fn of(m: &PaletteEntry) -> Self {
        MaterialInfo {
            id: m.id,
            name: m.name.clone(),
            title: m.recipe.name.clone(),
            built_in: m.built_in,
            catalog_id: m.recipe.catalog_id.clone(),
            kind: m.recipe.kind.name().into(),
            environment: m.recipe.environment.clone(),
            values: MaterialSet::values(&m.recipe),
        }
    }
}

/// A recipe file that was not read.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct RefusedMaterial {
    pub file: String,
    /// Starts with the key it got wrong, for example `diffuse.wrap: invalid type`.
    pub error: String,
}

impl MaterialPalette {
    /// The palette as `material.list` gives it.
    pub fn listed(&self) -> Materials {
        let refused = (self.refused.iter())
            .map(|r| RefusedMaterial {
                file: r.file.clone(),
                error: r.error.clone(),
            })
            .collect();
        Materials {
            looks: self.materials.iter().map(MaterialInfo::of).collect(),
            selected: self.active().name.clone(),
            refused,
        }
    }
}

/// Hands the palette to the renderer after a command changed it from `before`. When the
/// renderer cannot draw it, the palette goes back to `before` and the command is refused.
/// Otherwise each Reference View whose own look named a material follows it by id: to its new
/// name, or to the main view's look when it is gone. Returns those views.
fn hand_over(cx: &mut Ctx, before: PaletteState) -> Result<Vec<String>, CommandError> {
    let palette = &cx.palette.now;
    if let Some(Err(e)) = cx.render.with(|r| r.materials(palette)) {
        *cx.palette = before;
        let palette = &cx.palette.now;
        cx.render.with(|r| r.materials(palette));
        return Err(error(ErrorCode::Precondition, format!("cannot draw: {e}")));
    }
    let mut followed = Vec::new();
    for view in cx.views.references() {
        let named = (cx.views.secondary(&view)).and_then(|s| s.look.matcap.clone());
        let Some(was) = named.as_deref().and_then(|n| before.now.get(n)) else {
            continue;
        };
        let now = (cx.palette.now.materials.iter())
            .find(|m| m.id == was.id)
            .map(|m| m.name.clone());
        if now != named {
            let doc = cx.doc.document();
            cx.views
                .reset_reference(&view, doc, |s| s.look.matcap = now);
            followed.push(view);
        }
    }
    Ok(followed)
}

/// Makes the material `key` names (`#id`, `@place` or name) the active one: what `main`, every
/// agent's view and every snapshot draw. Not a palette step. Returns its name.
pub(crate) fn select(cx: &mut Ctx, key: &str) -> Result<String, CommandError> {
    let id = cx.palette.now.find(key)?.id;
    let before = cx.palette.clone();
    cx.palette.now.active = id;
    hand_over(cx, before)?;
    Ok(cx.palette.now.active().name.clone())
}

/// List every material in the palette in order (its id, name, title, whether it is built in,
/// catalog item, environment and values as `material.set` takes them), the active one, and any
/// recipe file that was refused with the key it got wrong.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialList {}

impl Command for MaterialList {
    const NAME: &'static str = "material.list";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Materials;

    fn examples() -> Vec<Example<Self>> {
        vec![Example::new("every material", Self {})]
    }

    fn run(self, cx: &mut Ctx) -> Result<Materials, CommandError> {
        Ok(cx.palette.now.listed())
    }
}
register_command!(MaterialList, "List materials");

/// Choose the active material: what MatCap and Lit shading draw on every part, in the window
/// and in `view.snapshot` and `view.contact_sheet`.
///
/// Each look is a physical material baked under its environment; the first draw of a look
/// bakes it. Choosing is not a palette step, so `material.undo` never changes it, and the
/// scene's undo never does either.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialSelect {
    /// The material: `#id`, `@place` (from 0) or its name, as `material.list` gives them, for
    /// example `red-wax`.
    #[schemars(length(min = 1))]
    pub name: String,
}

impl Command for MaterialSelect {
    const NAME: &'static str = "material.select";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Short;
    type Output = Materials;

    fn examples() -> Vec<Example<Self>> {
        let named = |name: &str| Self { name: name.into() };
        vec![
            Example::new("draw with MatCap Red Wax", named("red-wax")),
            Example::new("the third material, by id", named("#3")),
            Example::new("the first material, by place", named("@0")),
            Example::new("no such material", named("purple")).fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Materials, CommandError> {
        select(cx, &self.name)?;
        Ok(cx.palette.now.listed())
    }
}
register_command!(MaterialSelect, "Choose material");
