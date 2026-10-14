//! Pictures of what a control picks (decision 0025, section 13; decision 0043, step (d)), made
//! by the window the first time one shows: a material is its ball, baked from the palette's own
//! recipe as gs-render bakes it and shown through the default view transform, and a flat
//! material a disc of its color as the view draws it; an alpha is its picture as the shipped
//! recipe draws it; an environment is a mirror ball of it. Anything else has none, and its tile
//! is drawn as text.
//!
//! A picture is kept by what it is made from, not by name, so an edited material shows as
//! edited and a retitle makes nothing again; past `KEPT`, the oldest goes. A pass makes
//! `PER_PASS` at most and asks for another while some are still to make, so opening a tab never
//! stalls the window.

use std::sync::Arc;

use gs_core::material::MaterialKind;
use gs_engine::PaletteEntry;
use gs_protocol::Step;
use gs_render::display::{Display, encode};
use gs_render::matcap::{Options, Recipe, bake_under};
use gs_render::shading::Environment;

use crate::theme::{Theme, Themes};

/// Pixels across a picture.
const SIDE: u32 = 128;

/// The most pictures a pass makes.
const PER_PASS: usize = 2;

/// The most pictures kept: every shipped one and room for edits.
const KEPT: usize = 128;

/// What a picture is made from, by the command its control runs.
pub enum Source {
    Material(Arc<Recipe>),
    Environment(String),
    Alpha(String),
}

impl Source {
    /// The theme of `themes` a tile showing it (`ui.layout` with `theme`) is drawn from: its
    /// colors, with no picture.
    pub fn theme(step: &Step, themes: &Themes) -> Option<&'static Theme> {
        let name = step.args.get("theme")?.as_str()?;
        (step.command == "ui.layout").then(|| themes.find(name))?
    }

    /// What the control running `step` is pictured from: a `view.look`'s material as the
    /// palette `materials` holds it.
    pub fn of(step: &Step, materials: &[PaletteEntry]) -> Option<Source> {
        let arg = |name: &str| step.args.get(name)?.as_str().map(str::to_string);
        match step.command.as_str() {
            "view.look" => {
                let name = arg("matcap")?;
                let entry = materials.iter().find(|m| m.name == name)?;
                Some(Source::Material(Arc::clone(&entry.recipe)))
            }
            "view.environment" => arg("name").map(Source::Environment),
            "brush.set" => arg("alpha").map(Source::Alpha),
            _ => None,
        }
    }

    /// What it is kept by: a material's kind and what its bake depends on.
    fn key(&self) -> String {
        match self {
            Source::Material(r) => format!("material {:?} {:?}", r.kind, r.bake_key()),
            Source::Environment(name) => format!("environment {name}"),
            Source::Alpha(name) => format!("alpha {name}"),
        }
    }
}

/// `source`'s picture: `None` for one that cannot be made, or one this pass had no room left to
/// make (it asks for another pass, which makes it).
pub fn picture(ctx: &egui::Context, source: &Source) -> Option<egui::TextureId> {
    let key = source.key();
    let id = egui::Id::new(("picture", key.as_str()));
    if let Some(made) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return made.map(|m| m.id());
    }
    let count = egui::Id::new("pictures made");
    let pass = ctx.cumulative_pass_nr();
    let made = ctx.data(|d| d.get_temp::<(u64, usize)>(count));
    let made = made.filter(|(at, _)| *at == pass).map_or(0, |(_, n)| n);
    if made == PER_PASS {
        ctx.request_repaint();
        return None;
    }
    let options = egui::TextureOptions::LINEAR;
    let texture = image(source).map(|i| ctx.load_texture(&key, i, options));
    let kept = egui::Id::new("pictures kept");
    ctx.data_mut(|d| {
        d.insert_temp(count, (pass, made + 1));
        d.insert_temp(id, texture.clone());
        let order = d.get_temp_mut_or_default::<Vec<String>>(kept);
        order.push(key);
        let oldest = (order.len() > KEPT).then(|| order.remove(0));
        if let Some(oldest) = oldest {
            // Its texture is freed once nothing holds its handle.
            d.remove::<Option<egui::TextureHandle>>(egui::Id::new(("picture", oldest.as_str())));
        }
    });
    texture.map(|t| t.id())
}

fn image(source: &Source) -> Option<egui::ColorImage> {
    let shown = |light| encode(Display::default().show(light));
    match source {
        Source::Material(recipe) if recipe.kind == MaterialKind::Flat => {
            // As the view draws it: the color itself, with no light and no view transform.
            let color = encode(recipe.material.base_color.0);
            Some(ball(|_, _, _| color))
        }
        Source::Material(recipe) => {
            let mut recipe = Recipe::clone(recipe);
            recipe.bake.size = SIDE;
            let env = Environment::shipped(&recipe.environment).ok()?;
            let baked = bake_under(&recipe, &env, Options::default());
            Some(ball(|x, y, _| shown(baked.radiance(x, y))))
        }
        Source::Environment(name) => {
            let env = Environment::shipped(name).ok()?;
            // Seen from the front, looking down -z: the ball shows the direction the line of
            // sight reflects to about its normal.
            Some(ball(|_, _, n| {
                let k = 2.0 * n[2];
                shown(env.radiance([k * n[0], k * n[1], k * n[2] - 1.0], 0.0))
            }))
        }
        Source::Alpha(name) => {
            let grey = gs_engine::library::alpha_thumbnail(name, SIDE)?;
            Some(egui::ColorImage::from_gray([SIDE as usize; 2], &grey))
        }
    }
}

/// A ball `SIDE` pixels across, each pixel the color `color` gives for its column, row and the
/// ball's unit normal there (x right, y up, z toward the viewer); clear outside the ball, its
/// edge smoothed over a pixel.
fn ball(color: impl Fn(u32, u32, [f32; 3]) -> [u8; 3]) -> egui::ColorImage {
    let half = SIDE as f32 / 2.0;
    let mut pixels = Vec::with_capacity((SIDE * SIDE) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let u = (x as f32 + 0.5) / half - 1.0;
            let v = (y as f32 + 0.5) / half - 1.0;
            let r = libm::hypotf(u, v);
            let cover = ((1.0 - r) * half + 0.5).clamp(0.0, 1.0);
            let s = 1.0 / r.max(1.0);
            let (nu, nv) = (u * s, v * s);
            let n = [nu, -nv, (1.0 - nu * nu - nv * nv).max(0.0).sqrt()];
            let [red, green, blue] = color(x, y, n);
            let alpha = (cover * 255.0).round() as u8;
            pixels.push(egui::Color32::from_rgba_unmultiplied(
                red, green, blue, alpha,
            ));
        }
    }
    egui::ColorImage::new([SIDE as usize; 2], pixels)
}
