//! The looks a session draws: the material palette the engine last handed over (decision
//! 0043), and the looks baked and uploaded for one device. The window and headless sessions
//! both keep one, so they draw the same look the same way.

use std::sync::Arc;

use gs_core::material::{MaterialKind, Rgb};
use gs_engine::{MaterialPalette, Materials};
use gs_render::matcap::{Recipe, bake};
use gs_render::{CLAY_LOOK, GpuMatCap, Renderer};

/// The palette, the shipped clay look, and the looks drawn so far on the GPU: the active one,
/// and a Reference View's own (decision 0020).
pub struct MatCaps {
    palette: MaterialPalette,
    /// What Clay and Zebra light with: the shipped clay look, whatever the palette holds.
    clay: Option<Arc<Recipe>>,
    /// Keyed by what a bake depends on (`Recipe::bake_key`), not by name, so a recipe that
    /// changes under its name is never drawn stale and a retitle never bakes again.
    uploaded: Vec<(Arc<Recipe>, GpuMatCap)>,
}

/// Looks kept on the GPU at once: the main view's and a reference's, with room to switch.
const KEPT: usize = 4;

impl Default for MatCaps {
    /// The shipped palette, with MatCap Gray active (ARCHITECTURE 6).
    fn default() -> Self {
        let palette = MaterialPalette::shipped();
        let clay = palette.get(CLAY_LOOK).map(|m| Arc::clone(&m.recipe));
        Self {
            palette,
            clay,
            uploaded: Vec::new(),
        }
    }
}

impl MatCaps {
    /// The active material's name.
    pub fn selected(&self) -> &str {
        &self.palette.active().name
    }

    /// The palette as the engine last handed it over.
    pub fn palette(&self) -> &MaterialPalette {
        &self.palette
    }

    /// Every material, the active one and the refused files, as `material.list` reports them.
    pub fn list(&self) -> Materials {
        self.palette.listed()
    }

    /// The recipe of the material called `name` (`None`: the active one): the material Lit
    /// mode draws and the environment it lights with.
    pub fn recipe(&self, name: Option<&str>) -> Result<&Recipe, String> {
        let entry = match name {
            Some(name) => self.palette.get(name),
            None => Some(self.palette.active()),
        };
        let name = name.unwrap_or_default();
        entry
            .map(|m| &*m.recipe)
            .ok_or_else(|| format!("there is no material named {name}"))
    }

    /// The base color of the material called `name` (`None`: the active one) when it is flat:
    /// MatCap and Lit shading draw it as the paint times that color, with nothing baked
    /// (decision 0043, step (c)).
    pub fn flat(&self, name: Option<&str>) -> Option<Rgb> {
        let recipe = self.recipe(name).ok()?;
        (recipe.kind == MaterialKind::Flat).then_some(recipe.material.base_color.0)
    }

    /// The shipped clay look, which Clay and Zebra draw.
    pub fn clay(&self) -> Result<&Recipe, String> {
        let missing = || format!("the clay look {CLAY_LOOK} is not shipped");
        self.clay.as_deref().ok_or_else(missing)
    }

    /// Takes the palette the engine handed over: every look is drawn from it from now on.
    pub fn show(&mut self, palette: &MaterialPalette) {
        self.palette.clone_from(palette);
    }

    /// The active material on `renderer`'s device, baked (once per process) and uploaded the
    /// first time it is drawn there.
    pub fn gpu(&mut self, renderer: &Renderer) -> Result<&GpuMatCap, String> {
        let active = Arc::clone(&self.palette.active().recipe);
        self.upload(renderer, active)
    }

    /// The material called `name` on `renderer`'s device, uploaded the first time it is
    /// drawn; the least recently drawn of more than a few is dropped.
    pub fn gpu_of(&mut self, renderer: &Renderer, name: &str) -> Result<&GpuMatCap, String> {
        let recipe = (self.palette.get(name)).map(|m| Arc::clone(&m.recipe));
        let recipe = recipe.ok_or_else(|| format!("there is no material named {name}"))?;
        self.upload(renderer, recipe)
    }

    fn upload(&mut self, renderer: &Renderer, recipe: Arc<Recipe>) -> Result<&GpuMatCap, String> {
        let same = |(r, _): &(Arc<Recipe>, GpuMatCap)| r.bake_key() == recipe.bake_key();
        let entry = if let Some(at) = self.uploaded.iter().position(same) {
            self.uploaded.remove(at)
        } else {
            let baked = bake(&recipe).map_err(|e| e.to_string())?;
            (recipe, renderer.upload_bake(&baked))
        };
        self.uploaded.truncate(KEPT - 1);
        self.uploaded.insert(0, entry);
        Ok(&self.uploaded[0].1)
    }
}
