//! The Alpha palette (catalog `alpha.palette`; Brush tray > Alpha, decision 0060 section 6, item
//! 5): the picker row, the brush's alpha as a large picture, which opens the Library's Alphas
//! tab to pick from them all, then the alphas used lately as small pictures, each picking its
//! alpha (`alpha.pick` with `slot`, so an alpha picked next takes that slot instead of joining
//! the list). Hovering a picture says the alpha's name, size and bit depth, then what a tap does.
//! The list is the engine's (`alpha.recent`), so an agent's picks show in it as a hand's do.
//! Under it the `alpha` set: +, None and Alpha's ⋯ on a row, then Turn and Follow.

use egui::Color32;
use gs_protocol::Step;
use serde_json::json;

use super::{Face, Now, Press, control, controls_of, hover, pressed_as};
use crate::record::{Recorder, with_args};
use crate::theme::theme;

/// The brush's alpha's picture, in points across.
const LARGE: f32 = 72.0;
/// A recent alpha's picture, in points across.
const SMALL: f32 = 36.0;
/// Pixels across a picture as made.
const SIDE: u32 = 96;

/// Decision 0060, section 11's hints for the large picture and a recent one.
const LARGE_HINT: &str = "The image the tip stamps. Tap for every alpha in the Library.";
const RECENT_HINT: &str = "Tap to use it again.";

/// Draws the palette. Returns what it asked to run.
pub(super) fn palette(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let alphas = gs_engine::library::alphas();
    let title = |name: &str| {
        (alphas.iter().find(|a| a.0 == name)).map_or(name.to_string(), |a| a.1.to_string())
    };
    // A brush with no alpha stamps with white, the alpha that leaves its tip as it is.
    let current = now.brush.alpha.as_deref().unwrap_or("white");
    let library = with_args("ui.library", json!({ "tab": "alphas" }));
    let large = (LARGE, false, LARGE_HINT);
    let mut asked = picture(ui, (current, &title(current)), library, large, now, rec);
    let recent = now.state.get("alpha.recent").and_then(|r| r.as_array());
    let names = recent.into_iter().flatten().filter_map(|n| n.as_str());
    for (slot, name) in names.enumerate() {
        let pick = with_args("alpha.pick", json!({ "slot": slot }));
        let small = (SMALL, name == current, RECENT_HINT);
        asked = picture(ui, (name, &title(name)), pick, small, now, rec).or(asked);
    }
    ui.end_row();
    // A line's controls (+, None, the ⋯) share a row; the sliders after them take one each.
    let mut lined = false;
    for c in controls_of("alpha", now) {
        let line = c.line.is_some();
        if lined && !line {
            ui.end_row();
        }
        asked = control(ui, &c, line, now, rec).or(asked);
        if !line {
            ui.end_row();
        }
        lined = line;
    }
    asked
}

/// Alpha `name`'s picture on a square button `side` points across, recorded under `title` and
/// lit when `lit`, saying its name, size and bits, then `hint`, on hover. Returns `step` when
/// tapped.
fn picture(
    ui: &mut egui::Ui,
    (name, title): (&str, &str),
    step: Step,
    (side, lit, hint): (f32, bool, &str),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let press = Press {
        label: title,
        step: step.clone(),
        key: None,
        lit,
        shows: None,
    };
    let shown = pressed_as(ui, press, Face::Square(side), now, rec);
    let about = gs_engine::library::alpha_hover(name).map(|about| format!("{about}. {hint}"));
    let shown = hover(shown, Some(about.as_deref().unwrap_or(hint)), rec);
    let t = theme(ui.ctx());
    let face = shown.rect.shrink(t.spacing.item[1]);
    let seen = ui.is_rect_visible(shown.rect);
    if let Some(texture) = seen.then(|| texture(ui.ctx(), name)).flatten() {
        let tint = if shown.enabled() {
            Color32::WHITE
        } else {
            t.colors.text_off
        };
        let image = egui::Image::from_texture((texture, face.size())).tint(tint);
        image.paint_at(ui, face);
    }
    shown.clicked().then_some(step)
}

/// Alpha `name`'s picture, made the first time it shows and kept for the life of the window: one
/// a frame at most, asking for another frame, so showing the palette never stalls the window.
fn texture(ctx: &egui::Context, name: &str) -> Option<egui::TextureId> {
    let id = egui::Id::new(("alpha picture", name));
    if let Some(made) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(made.id());
    }
    let (pass, last) = (
        ctx.cumulative_pass_nr(),
        egui::Id::new("alpha picture made"),
    );
    if ctx.data(|d| d.get_temp::<u64>(last)) == Some(pass) {
        ctx.request_repaint();
        return None;
    }
    ctx.data_mut(|d| d.insert_temp(last, pass));
    let grey = gs_engine::library::alpha_thumbnail(name, SIDE)?;
    let image = egui::ColorImage::from_gray([SIDE as usize; 2], &grey);
    let options = egui::TextureOptions::LINEAR;
    let made = ctx.load_texture(format!("alpha {name}"), image, options);
    ctx.data_mut(|d| d.insert_temp(id, made.clone()));
    Some(made.id())
}
