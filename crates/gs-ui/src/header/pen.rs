//! The bar's pen end: the layout's `[header] show` in its order, the view buttons once the view
//! strip's among them (decision 0060 as amended for H5). A view button shows its icon alone
//! where it has one, else its short label, with its name and hint on hover.

use egui::RichText;
use gs_protocol::Step;

use super::{Menus, spec};
use crate::direction::{Control, HeaderOptions, HeaderSpec, title};
use crate::pieces::{
    Dragging, Face, Now, control, controls_of, faced, held_past_click, label, options_mark,
    press_of, pressed_as, section,
};
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;
use crate::{descriptions, icons};

/// Draws the pen end's pieces in the layout's order: an options button by its id, a set with a
/// switch with options a control at a time as view buttons ([`buttons`]), and any other piece
/// as a tray section draws it. A set that is also an options button's id is that button while
/// `fold` (Symmetry in a narrow view, decision 0060 as amended for H6). Returns what they asked
/// to run.
pub(super) fn end(
    ui: &mut egui::Ui,
    (spec, now, fold): (&HeaderSpec, &Now, bool),
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let mut asked = Vec::new();
    for name in &spec.show {
        let set = controls_of(name, now);
        let button = spec.options.iter().find(|o| o.id == *name);
        if let Some(o) = button.filter(|_| fold || set.is_empty()) {
            options_button(ui, o, now, menus, rec);
        } else if set.iter().any(|c| c.options.is_some()) {
            asked.extend(buttons(ui, name, now, Some(&mut *menus), rec));
        } else {
            asked.extend(section(ui, name, now, dragging, rec));
        }
    }
    asked
}

/// Draws the set `set`'s shown controls as view buttons, a control at a time, a line's label
/// before its first: the header's, and the Reference View's bar (`menus` `None`, which opens
/// no options). Returns what they asked to run.
pub(crate) fn buttons(
    ui: &mut egui::Ui,
    set: &str,
    now: &Now,
    mut menus: Option<&mut Menus>,
    rec: &mut Recorder,
) -> Vec<Step> {
    let (mut line, mut asked) = (None, Vec::new());
    for c in controls_of(set, now).iter().filter(|c| c.shown(&now.state)) {
        if c.line != line {
            line.clone_from(&c.line);
            if let Some(words) = &line {
                label(ui, RichText::new(words), rec);
            }
        }
        asked.extend(button(ui, c, now, menus.as_deref_mut(), rec));
    }
    asked
}

/// One view button. A switch with options (rule 8.5) runs on a tap and opens them under it on
/// a press held past a click or a right click, lit while they are open. A control with options
/// and no command (Symmetry's ▼) opens them on a tap, as Look does; any other control that runs
/// no command shows as it does anywhere. Returns its step when tapped.
fn button(
    ui: &mut egui::Ui,
    c: &Control,
    now: &Now,
    menus: Option<&mut Menus>,
    rec: &mut Recorder,
) -> Option<Step> {
    let Some(mut press) = press_of(c, true, now) else {
        if let (Some(set), Some(menus)) = (c.options.as_deref(), menus) {
            let (face, heading) = (
                c.short.as_deref().unwrap_or(&c.label),
                title(&now.directions.controls, set),
            );
            tap_opens(
                ui,
                (set, heading),
                ((face, None), c.hint.as_deref()),
                (false, false),
                menus,
                rec,
            );
            return None;
        }
        return control(ui, c, true, now, rec);
    };
    let options = c.options.as_deref().zip(menus);
    press.lit |= (options.as_ref()).is_some_and(|(set, m)| m.options() == Some(*set));
    let (step, label, lit) = (press.step.clone(), press.label, press.lit);
    let icon = icons::of(c);
    let shown = pressed_as(ui, press, icon.map_or(Face::Label, Face::Alone), now, rec);
    tip(ui, &shown, (label, icon.is_some()), c.hint.as_deref(), rec);
    if let Some((set, menus)) = options {
        with_options(ui, (&shown, lit), set, (now, menus), rec);
    }
    shown.clicked().then_some(step)
}

/// A view button's options, the set `set`: a press on `shown` held past a click (egui's longest,
/// so letting go is never a tap) or a right click opens them under it, and a press outside
/// shuts them (`early`). A section of the header, folded while they are shut.
fn with_options(
    ui: &egui::Ui,
    (shown, lit): (&egui::Response, bool),
    set: &str,
    (now, menus): (&Now, &mut Menus),
    rec: &mut Recorder,
) {
    options_mark(ui, shown.rect, lit);
    if held_past_click(ui, shown) || shown.secondary_clicked() {
        menus.show_options(set);
    }
    let open = menus.options() == Some(set);
    if open {
        menus.at = Some(shown.rect);
    }
    let heading = title(&now.directions.controls, set);
    rec.section(set, heading, !open, lit, shown.rect);
}

/// An options button (Look): a tap opens its options under it, or shuts them; lit while they
/// are open or the state value it `shows` is on. A section of the header, folded while they
/// are shut.
fn options_button(
    ui: &mut egui::Ui,
    o: &HeaderOptions,
    now: &Now,
    menus: &mut Menus,
    rec: &mut Recorder,
) {
    let on = (o.shows.as_ref().and_then(|v| now.state.get(v)))
        .is_some_and(|v| v.as_bool() == Some(true));
    let face = (o.label.as_str(), icons::of_options(&o.id));
    let id = (o.id.as_str(), o.label.as_str());
    tap_opens(ui, id, (face, o.hint.as_deref()), (on, true), menus, rec);
}

/// A button whose tap opens the options `id` under it, or shuts them: an options button, or a
/// control with options and no command. Lit while they are open or `on`, with the options mark
/// where `marked`; a section of the header titled `heading`, folded while they are shut.
fn tap_opens(
    ui: &mut egui::Ui,
    (id, heading): (&str, &str),
    (face, hint): ((&str, Option<&'static str>), Option<&str>),
    (on, marked): (bool, bool),
    menus: &mut Menus,
    rec: &mut Recorder,
) {
    let was_open = menus.options() == Some(id);
    let lit = was_open || on;
    let shown = opener(ui, face, hint, lit, rec);
    if marked {
        options_mark(ui, shown.rect, lit);
    }
    if shown.clicked() && was_open {
        menus.shut();
    } else if shown.clicked() {
        menus.show_options(id);
    }
    let open = menus.options() == Some(id);
    if open {
        menus.at = Some(shown.rect);
    }
    rec.section(id, heading, !open, on, shown.rect);
}

/// A button that opens something rather than running a command (Look, and the menus' fold):
/// its icon alone where it has one, else its label, lit while `lit`, and recorded by its label
/// with what hovering it says (see [`tip`]).
pub(super) fn opener(
    ui: &mut egui::Ui,
    (label, icon): (&str, Option<&'static str>),
    hint: Option<&str>,
    lit: bool,
    rec: &mut Recorder,
) -> egui::Response {
    let colors = &theme(ui.ctx()).colors;
    let ink = if lit {
        (Some(colors.on_lit), Some(colors.lit))
    } else {
        (None, None)
    };
    let face = icon.map_or(Face::Label, Face::Alone);
    let shown = faced(ui, (face, label), ink, (true, lit));
    rec.widget(Widget::plain(label, Role::Button).selected(lit), shown.rect);
    tip(ui, &shown, (label, icon.is_some()), hint, rec);
    shown
}

/// What a view button is for, on hover: its name over its hint when it shows its icon alone
/// (`alone`), else its hint; recorded as the hint of the widget recorded last.
pub(crate) fn tip(
    ui: &egui::Ui,
    shown: &egui::Response,
    (label, alone): (&str, bool),
    hint: Option<&str>,
    rec: &mut Recorder,
) {
    if let Some(hint) = hint {
        rec.hint(hint);
    }
    let says = match (alone, hint) {
        (true, Some(hint)) => format!("{label}\n{hint}"),
        (true, None) => label.to_string(),
        (false, Some(hint)) => hint.to_string(),
        (false, None) => return,
    };
    descriptions::hint(ui, shown, &says);
}

/// Whether the pen end of `spec` has the options `id`: an options button's, or a view button's
/// set.
pub(super) fn opens(spec: &HeaderSpec, now: &Now, id: &str) -> bool {
    spec.show.iter().any(|p| {
        let button = p == id && spec.options.iter().any(|o| o.id == id);
        button
            || controls_of(p, now)
                .iter()
                .any(|c| c.options.as_deref() == Some(id))
    })
}

/// What the options `id` are called and hold: an options button's label and pieces, else a view
/// button's set under its title.
pub(super) fn options_of(id: &str, now: &Now) -> (String, Vec<String>) {
    let button = spec(now).and_then(|s| s.options.iter().find(|o| o.id == id));
    match button {
        Some(o) => (o.label.clone(), o.show.clone()),
        None => (title(&now.directions.controls, id).into(), vec![id.into()]),
    }
}
