//! The brush shelf's family buttons (decision 0025, amended 2026-10-12, Shelf-family): the shared
//! widget `families` draws each brush family's small label and one button, its face. The face is
//! the family's brush lit now (Smooth's is the brush on the Smooth slot), else the one lit last
//! in this window, else its first in `brush.list`'s order, on a button as wide as the family's
//! longest name, so a new face moves nothing. A tap on a face not lit picks it; a tap on a lit
//! face, a press held past a click or a right click opens the family below the face at the
//! pointer's x, and a tap on the face while it is open shuts it. Opening runs nothing.
//!
//! The open family ([`window`]) lists every brush of it, each the same control as on the shelf,
//! in a small window whose top edge is just below the face's row, so the face stays bare. A pick
//! runs its step and shuts it; Escape, or a press anywhere outside it other than on its face,
//! shuts it and runs nothing. Which family is open, and each family's face, live in egui's
//! memory for the window's life, as a button's options do.

use egui::{Align2, Id, Rect, RichText};
use gs_protocol::Step;

use crate::direction::{Control, title};
use crate::floats::scrolled;
use crate::pieces::{
    Face, Now, control, held_past_click, keep_together, label, options_mark, press_of, pressed_as,
};
use crate::record::Recorder;
use crate::theme::theme;

/// The open family's egui area, which Escape and Enter order by (`shell/escape.rs`).
pub(crate) const AREA: &str = "brush family window";

/// The open family: its name, the pointer's x where it opened, in points, and its face on the
/// shelf, which it opens below and a press on which is the face's to answer.
#[derive(Clone)]
pub(crate) struct Open {
    family: String,
    x: f32,
    face: Rect,
}

/// Where egui's memory keeps the open family, and with a family's name, its face.
fn kept() -> Id {
    Id::new("brush family")
}

pub(crate) fn opened(ctx: &egui::Context) -> Option<Open> {
    ctx.data(|d| d.get_temp(kept()))
}

/// Shuts the open family; Escape too, given by the window (`shell/escape.rs`).
pub(crate) fn shut(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Open>(kept()));
}

/// `brushes`' families in their order, each with its brushes.
fn families(brushes: &[Control]) -> Vec<(&str, Vec<&Control>)> {
    let mut out: Vec<(&str, Vec<&Control>)> = Vec::new();
    for c in brushes {
        let family = c.group.as_deref().unwrap_or_default();
        match out.iter_mut().find(|(f, _)| *f == family) {
            Some((_, members)) => members.push(c),
            None => out.push((family, vec![c])),
        }
    }
    out
}

/// The widest of `members`' labels, with a button's padding.
fn widest(ui: &egui::Ui, members: &[&Control]) -> (f32, String) {
    let width = |label: &str| crate::text::width(ui, label, egui::TextStyle::Button);
    let widths = members.iter().map(|c| (width(&c.label), c.label.clone()));
    let (wide, name) = widths.fold((0.0, String::new()), |a, b| if b.0 > a.0 { b } else { a });
    (wide + 2.0 * ui.spacing().button_padding.x, name)
}

/// Draws a button per brush family along a shelf, the first after the piece's title when
/// `titled`. Returns what a tap picked.
pub fn buttons(ui: &mut egui::Ui, titled: bool, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let brushes = now.directions.brush_controls();
    let heading = RichText::new(title(&now.directions.controls, "families")).strong();
    let mut heading = titled.then_some(heading);
    let mut asked = None;
    for (family, members) in families(&brushes) {
        let name = RichText::new(family).small().weak();
        let (wide, longest) = widest(ui, &members);
        let longest = RichText::new(longest);
        let texts: Vec<&RichText> = heading.iter().chain([&name, &longest]).collect();
        keep_together(ui, &texts);
        for text in heading.take().into_iter().chain([name]) {
            label(ui, text, rec);
        }
        asked = button(ui, family, &members, wide, now, rec).or(asked);
    }
    asked
}

/// The face of `family`, whose brushes are `members` (never none): its brush lit now, else the
/// one lit last in this window, else its first.
fn face<'c>(ui: &egui::Ui, family: &str, members: &[&'c Control], now: &Now) -> &'c Control {
    let id = kept().with(family);
    if let Some(&lit) = members.iter().find(|c| c.lit(&now.state).unwrap_or(false)) {
        ui.data_mut(|d| d.insert_temp(id, lit.label.clone()));
        return lit;
    }
    let last = ui.data(|d| d.get_temp::<String>(id));
    let named = last.and_then(|l| members.iter().find(|c| c.label == l));
    named.copied().unwrap_or(members[0])
}

/// `family`'s button, `wide` points across, showing its face: a tap picks the face unless it is
/// lit, which opens the family, as a held press or a right click does; a tap while it is open
/// shuts it. A family of one brush is that brush's button. Returns what a tap picked.
fn button(
    ui: &mut egui::Ui,
    family: &str,
    members: &[&Control],
    wide: f32,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let press = press_of(face(ui, family, members, now), true, now)?;
    let (lit, step) = (press.lit, press.step.clone());
    let size = egui::vec2(wide, ui.spacing().interact_size.y);
    let fill = egui::Layout::centered_and_justified(egui::Direction::LeftToRight);
    let drawn = ui.allocate_ui_with_layout(size, fill, |ui| {
        pressed_as(ui, press, Face::Label, now, rec)
    });
    let shown = drawn.inner;
    // Which family it is and what opening it does, from the shelf's controls file.
    if let Some(hint) = now.directions.controls.families.get(family) {
        rec.hint(hint);
        crate::descriptions::hint(ui, &shown, hint);
    }
    if members.len() < 2 {
        return shown.clicked().then_some(step);
    }
    options_mark(ui, shown.rect, lit);
    let held = held_past_click(ui, &shown) || shown.secondary_clicked();
    let open = opened(ui.ctx()).is_some_and(|o| o.family == family);
    let pointer = (shown.interact_pointer_pos()).or_else(|| ui.input(|i| i.pointer.latest_pos()));
    let here = Open {
        family: family.into(),
        x: pointer.unwrap_or(shown.rect.center()).x,
        face: shown.rect,
    };
    let opens = match (shown.clicked(), open) {
        (true, true) => {
            shut(ui.ctx());
            false
        }
        (true, false) if !lit => return Some(step),
        (true, false) => true,
        (false, false) => held,
        (false, true) => false,
    };
    if opens {
        ui.data_mut(|d| d.insert_temp(kept(), here));
    }
    None
}

/// Draws the open family, if one is, its top edge just below its face's row at the pointer's x
/// where it opened, kept inside the window and scrolling in the room below; call after the 3D
/// view and the quick menu, so it lies over them. Shuts it on a pick or a press outside it other
/// than on its face. Returns what was picked.
pub fn window(ui: &egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let open = opened(ui.ctx())?;
    let brushes = now.directions.brush_controls();
    let family = Some(open.family.as_str());
    let members: Vec<&Control> = brushes
        .iter()
        .filter(|c| c.group.as_deref() == family)
        .collect();
    if members.is_empty() {
        shut(ui.ctx());
        return None;
    }
    let t = theme(ui.ctx());
    let room = ui.ctx().content_rect().shrink(t.spacing.margin);
    let (wide, _) = widest(ui, &members);
    let top = open.face.bottom() + ui.spacing().item_spacing.y;
    let area = egui::Area::new(Id::new(AREA))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_TOP)
        .fixed_pos(egui::pos2(open.x, top))
        .constrain_to(room);
    let shown = area.show(ui.ctx(), |ui| {
        let frame = egui::Frame::popup(ui.style()).inner_margin(t.spacing.margin);
        let below = room.bottom() - top - frame.total_margin().sum().y;
        let most = below.max(ui.spacing().interact_size.y);
        let framed = frame.show(ui, |ui| {
            ui.set_width(wide);
            rec.panel("family", &open.family, ui.max_rect());
            scrolled(ui, most, rec, |ui, rec| {
                let column = egui::Layout::top_down_justified(egui::Align::Min);
                let picks = ui.with_layout(column, |ui| {
                    let mut picked = None;
                    for c in &members {
                        picked = picked.or(control(ui, c, false, now, rec));
                    }
                    picked
                });
                picks.inner
            })
        });
        framed.inner
    });
    let rect = shown.response.rect;
    rec.fit(rect);
    let shuts = ui.input(|i| {
        let origin = i.pointer.press_origin();
        let outside = origin.is_some_and(|p| !rect.contains(p) && !open.face.contains(p));
        i.pointer.any_pressed() && outside
    });
    if shown.inner.is_some() || shuts {
        shut(ui.ctx());
    }
    shown.inner
}
