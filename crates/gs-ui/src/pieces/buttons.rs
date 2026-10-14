//! Buttons: a control that runs its command, lit while the state value it names has its `on`
//! value, or dimmed, with why on hover, while its command cannot run or has no effect. With
//! Ctrl held over one, it is described (`descriptions.rs`). A button with options (a tray's
//! Save) runs on a tap and lists them under it on a press held past a click or a right click
//! (rule 8.5).

use std::time::Duration;

use egui::{Color32, Popup, PopupCloseBehavior, RichText, SetOpenCommand, Shape, Stroke, vec2};
use gs_engine::Shows;
use gs_protocol::Step;

use super::sliders::setting;
use super::{Now, controls_of, detail, head};
use crate::direction::Control;
use crate::keymap::picked;
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;
use crate::{descriptions, icons, recent};

/// One control: a button running its command, a toggle when it shows a state value, and a
/// disabled button saying on hover what it waits for when it is not built yet.
pub fn control(
    ui: &mut egui::Ui,
    c: &Control,
    short: bool,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    if let (Some(arg), Some(range)) = (c.dragged(), c.range) {
        return setting(ui, c, (&arg, range), now, rec);
    }
    let Some(press) = press_of(c, short, now) else {
        if let Some(set) = c.options.as_deref().filter(|_| c.later.is_none()) {
            return lists(ui, c, set, now, rec);
        }
        // Waiting: no fill and dim text, no word beside it; its hint then what it waits for on
        // hover (decision 0025, section 7, amended 2026-10-13).
        let label = label_of(c, short);
        let colors = &theme(ui.ctx()).colors;
        let hint = c
            .hint
            .as_deref()
            .map(|h| format!("{h} "))
            .unwrap_or_default();
        let waits = format!(
            "{hint}Waits for: {}",
            c.later.as_deref().unwrap_or_default()
        );
        let button = egui::Button::new(RichText::new(label).color(colors.text_off))
            .fill(Color32::TRANSPARENT);
        let shown = (ui.add_enabled(false, button)).on_disabled_hover_text(&waits);
        rec.widget(
            Widget::plain(label, Role::Button).disabled(&waits),
            shown.rect,
        );
        rec.hint(&waits);
        return None;
    };
    // A control naming a state value reads it after its label ("Undo Draw stroke"), on one row
    // (decision 0062, row 794).
    let named = c.named(press.label, &now.state);
    let face = named.as_ref().map_or(Face::of(icons::of(c)), |_| Face::Cut);
    let press = Press {
        label: named.as_deref().unwrap_or(press.label),
        ..press
    };
    let step = press.step.clone();
    let shown = pressed_as(ui, press, face, now, rec);
    if let Some(hint) = &c.hint {
        rec.hint(hint);
        descriptions::hint(ui, &shown, hint);
    }
    let picked = (c.options.as_deref()).and_then(|set| options(ui, &shown, set, now, rec));
    picked.or_else(|| shown.clicked().then_some(step))
}

/// A button with options and no command of its own (Open a recent scene, a level's `···`): a tap
/// opens or shuts its options, as a hold or a right click opens them. It stands for them, so
/// it shows its short label wherever it is drawn. Returns what was picked.
fn lists(ui: &mut egui::Ui, c: &Control, set: &str, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let label = label_of(c, true);
    let shown = ui.button(label);
    rec.widget(Widget::plain(label, Role::Button), shown.rect);
    let shown = hover(shown, c.hint.as_deref(), rec);
    if shown.clicked() {
        Popup::toggle_id(ui.ctx(), Popup::menu(&shown).get_id());
    }
    options(ui, &shown, set, now, rec)
}

/// The options of a button with them (`shown`), the set `set`, in a menu under it: opened by a
/// press held past a click or a right click, and shut by a pick or a press anywhere else.
/// `ui.describe` lists them with the button's panel while they show, whole: the menu stands
/// over the panel, so a list scrolling under it does not cut them. Each row shows its short
/// label; the `recent` set's are the recent scenes, a row each. Returns what was picked.
fn options(
    ui: &mut egui::Ui,
    shown: &egui::Response,
    set: &str,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    options_mark(ui, shown.rect, false);
    let opens = held_past_click(ui, shown) || shown.secondary_clicked();
    let menu = Popup::menu(shown)
        .open_memory(opens.then_some(SetOpenCommand::Bool(true)))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let id = menu.get_id();
    let listed = controls_of(set, now);
    let listing = rec.listing();
    rec.list(None);
    let picked = menu.show(|ui| {
        if set == recent::SET {
            return recent::rows(ui, now, rec);
        }
        // Drawn in code: the Gizmo tool's ⋯ starts with its operators (Alt changes them), the
        // level's with its levels, and the brush's ends with the brushes to copy to.
        let first = match set {
            crate::gizmo::MENU => crate::gizmo::menu(ui, now, rec),
            detail::LEVELS => choices(ui, "Go to level", &detail::levels(now), now, rec),
            _ => None,
        };
        let shown = listed.iter().filter(|c| c.shown(&now.state));
        let asked = shown.map(|c| control(ui, c, true, now, rec));
        let asked = asked.fold(first, Option::or);
        if set != head::FILE {
            return asked;
        }
        asked.or(choices(
            ui,
            "Copy settings to",
            &head::copies(now),
            now,
            rec,
        ))
    });
    rec.list(listing);
    let picked = picked.and_then(|p| p.inner);
    if picked.is_some() {
        Popup::close_id(ui.ctx(), id);
    }
    picked
}

/// `title` over `picks` as buttons on rows no wider than the menu, picked rather than typed
/// (review 0009, R6); nothing while there are none. Returns what was picked.
fn choices(
    ui: &mut egui::Ui,
    title: &str,
    picks: &[Control],
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    if picks.is_empty() {
        return None;
    }
    super::label(ui, RichText::new(title).small().weak(), rec);
    let wide = ui.min_rect().width().max(ui.spacing().menu_width);
    let rows = ui.horizontal_wrapped(|ui| {
        ui.set_max_width(wide);
        let asked = picks.iter().map(|c| control(ui, c, true, now, rec));
        asked.fold(None, Option::or)
    });
    rows.inner
}

/// True while a press on `button` has been held past the longest click (egui's, so letting go
/// is never a tap, and the length egui takes for a touch screen's long press) and the pen is
/// still on it: a button with options opens them (rule 8.5). egui stops counting a press as on
/// the button once it is too long for a click, so the button keeps when its press began until
/// the pen lets go or leaves it.
pub fn held_past_click(ui: &egui::Ui, button: &egui::Response) -> bool {
    let longest = ui.ctx().options(|o| o.input_options.max_click_duration);
    let key = button.id.with("pressed");
    let (time, start, down, at) = ui.input(|i| {
        let p = &i.pointer;
        let (start, down) = (p.press_start_time(), p.primary_down());
        (i.time, start, down, p.latest_pos())
    });
    if let Some(start) = start.filter(|_| down && button.is_pointer_button_down_on()) {
        ui.data_mut(|d| d.insert_temp(key, start));
    }
    let on = down && at.is_some_and(|p| button.rect.contains(p));
    let pressed = ui.data(|d| d.get_temp::<f64>(key)).filter(|_| on);
    match pressed.map(|start| time - start) {
        Some(held) if held > longest => return true,
        Some(held) => ui
            .ctx()
            .request_repaint_after(Duration::from_secs_f64(longest - held)),
        None => ui.data_mut(|d| d.remove::<f64>(key)),
    }
    false
}

/// A small triangle in a button's lower corner: the button has options. On a lit button it
/// takes the lit button's text color.
pub fn options_mark(ui: &egui::Ui, rect: egui::Rect, lit: bool) {
    let corner = rect.right_bottom() - vec2(3.0, 3.0);
    let size = 7.0;
    let points = vec![corner, corner - vec2(size, 0.0), corner - vec2(0.0, size)];
    let colors = &theme(ui.ctx()).colors;
    let color = if lit { colors.on_lit } else { colors.text_dim };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// `shown` saying `hint` on hover, as `ui.describe` says for the widget recorded last (its own):
/// in its description when it has one (a brush's, a Library tile's), else as egui's hover text.
pub fn hover(shown: egui::Response, hint: Option<&str>, rec: &mut Recorder) -> egui::Response {
    let Some(hint) = hint else {
        return shown;
    };
    rec.hint(hint);
    let ctx = shown.ctx.clone();
    descriptions::said(&ctx, shown, hint)
}

/// The button a control that runs a command draws, with its short label when `short`; `None`
/// while it waits for a feature.
pub fn press_of<'a>(c: &'a Control, short: bool, now: &'a Now) -> Option<Press<'a>> {
    let step = c.step_in(&now.state)?;
    Some(Press {
        label: label_of(c, short),
        key: now.keymap.key_for_args(&step.command, &c.args),
        step,
        lit: c.lit(&now.state).unwrap_or(false),
        shows: c.shows(),
    })
}

/// A button that runs a command, as drawn: its label, what it runs and the key bound to that,
/// whether it is lit, and the state value it shows when it is a toggle.
pub struct Press<'a> {
    pub label: &'a str,
    pub step: Step,
    pub key: Option<&'a str>,
    pub lit: bool,
    pub shows: Option<Shows>,
}

/// Draws `p` with its label (see [`pressed_as`]). Returns its command when pressed.
pub fn run_button(ui: &mut egui::Ui, p: Press, now: &Now, rec: &mut Recorder) -> Option<Step> {
    run_as(ui, p, Face::Label, now, rec)
}

/// Draws `p` showing `face` (see [`pressed_as`]). Returns its command when pressed.
pub fn run_as(
    ui: &mut egui::Ui,
    p: Press,
    face: Face,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let step = p.step.clone();
    pressed_as(ui, p, face, now, rec).clicked().then_some(step)
}

/// What a button shows: its label; its label beside an icon (decision 0025, section 13); the
/// icon alone, square (the header's view buttons, decision 0060 as amended for H5); an empty
/// square that many points across, for the caller to paint a face in (a Library tile); or its
/// label on one row, cut with "…" where it is too long, whole on hover (a step's or part's name).
#[derive(Clone, Copy)]
pub enum Face {
    Label,
    Icon(&'static str),
    Alone(&'static str),
    Square(f32),
    Cut,
}

impl Face {
    /// Its label, beside `icon` if it has one.
    pub fn of(icon: Option<&'static str>) -> Face {
        icon.map_or(Face::Label, Face::Icon)
    }
}

/// Draws `p` showing `face`: lit with dark text while on, and dimmed, with why on hover, while
/// its command cannot run now (held: it does not run) or it has no effect now (idle: it still
/// runs); an icon takes its label's color. A dimmed toggle still shows whether it is on. It
/// records the same whatever it shows. Returns how the pointer met it.
pub fn pressed_as(
    ui: &mut egui::Ui,
    p: Press,
    face: Face,
    now: &Now,
    rec: &mut Recorder,
) -> egui::Response {
    let colors = &theme(ui.ctx()).colors;
    let Press {
        label,
        step,
        key,
        lit,
        shows,
    } = p;
    let held = now.applies.and_then(|a| a.why(&step));
    let idle = (now.applies.zip(shows.as_ref())).and_then(|(a, s)| a.why_idle(s));
    let colored = match (held.or(idle), lit) {
        // Dimmed: no fill and dim text, or, while on, a fill in the dim text's color under
        // dark text, so it still reads as on (decision 0025, section 6). It names its command.
        (Some(_), true) => (Some(colors.on_lit), Some(colors.text_off)),
        (Some(_), false) => (Some(colors.text_off), Some(Color32::TRANSPARENT)),
        // On: lit, with dark text. egui would paint it with the text selection's colors.
        (None, true) => (Some(colors.on_lit), Some(colors.lit)),
        // A button that deletes reads in the warning color (review 0009).
        (None, false) => (crate::theme::deletes(&step).then(|| colors.warning()), None),
    };
    let shown = faced(ui, (face, label), colored, (held.is_none(), lit));
    let brush = brush_tip(&step, now, held.or(idle));
    // A brush's box says why itself, so only one box shows.
    let shown = match (held, idle) {
        (Some(why), _) if brush.is_none() => shown.on_disabled_hover_text(why),
        (None, Some(why)) if brush.is_none() => shown.on_hover_text(why),
        _ => shown,
    };
    descriptions::note_brush(ui, &shown, &step, key, brush);
    let widget = Widget::runs(label, Role::Button, step, key);
    let widget = match shows {
        Some(shows) => widget.toggle(shows, lit),
        None => widget.selected(lit),
    };
    let widget = match (held, idle) {
        (Some(why), _) => widget.disabled(why),
        (None, Some(why)) => widget.idle(why),
        (None, None) => widget,
    };
    rec.widget(widget, shown.rect);
    shown
}

/// What the box over a button that runs `step` says when it picks a brush by name (`brush.set`,
/// or a Smooth brush's `brush.slot`): the brush, the keys that pick it in `now`'s keymap and
/// slots, and `why` it cannot run now.
fn brush_tip(step: &Step, now: &Now, why: Option<&str>) -> Option<descriptions::Brush> {
    let args: toml::Table = serde_json::from_value(step.args.clone()).ok()?;
    let name = picked(&step.command, &args)?;
    Some(descriptions::Brush {
        keys: now.keymap.brush_keys(name, &now.brush.slots),
        name: name.into(),
        why: why.map(Into::into),
    })
}

/// Adds a button showing `face` for `label`: its text and icon in `ink` (the theme's text
/// color when `None`) over `fill` (egui's when `None`), enabled while `enabled` and selected
/// while `selected`. An icon alone is square, its padding across as it is down. Records
/// nothing. Returns how the pointer met it.
pub fn faced(
    ui: &mut egui::Ui,
    (face, label): (Face, &str),
    (ink, fill): (Option<Color32>, Option<Color32>),
    (enabled, selected): (bool, bool),
) -> egui::Response {
    let tint = ink.unwrap_or(theme(ui.ctx()).colors.text);
    let text = ink.map_or_else(|| RichText::new(label), |c| RichText::new(label).color(c));
    let icon = |name| icons::image(ui, name, tint);
    let mut cut = false;
    let button = match face {
        Face::Square(side) => egui::Button::new("").min_size(vec2(side, side)),
        Face::Icon(name) => match icon(name) {
            Some(image) => egui::Button::image_and_text(image, text),
            None => egui::Button::new(text),
        },
        Face::Alone(name) => match icon(name) {
            Some(image) => egui::Button::image(image),
            None => egui::Button::new(text),
        },
        Face::Label => egui::Button::new(text),
        Face::Cut => {
            let galley = one_row(ui, text);
            cut = galley.elided;
            egui::Button::new(galley)
        }
    };
    let button = match fill {
        Some(f) => button.fill(f),
        None => button,
    }
    .selected(selected);
    let padding = ui.spacing().button_padding;
    if matches!(face, Face::Alone(_)) {
        let side = ui.spacing().interact_size.y;
        ui.spacing_mut().button_padding.x = padding.y;
        let shown = ui.add_enabled(enabled, button.min_size(vec2(side, side)));
        ui.spacing_mut().button_padding = padding;
        // An icon alone still reads as its label to the screen reader (egui gives it none), but
        // not in a pass that only measures it (`header::one_row`), which would read it twice.
        if ui.is_visible() {
            let kind = egui::WidgetType::Button;
            shown.widget_info(|| egui::WidgetInfo::selected(kind, enabled, selected, label));
        }
        return shown;
    }
    let shown = ui.add_enabled(enabled, button);
    if cut {
        return shown.on_hover_text(label).on_disabled_hover_text(label);
    }
    shown
}

/// `text` laid out on one row, cut with "…" to the widest a button can show it where it is
/// drawn: a whole row in a layout that wraps (which moves the button to the next row when it
/// does not fit the rest of this one), else the room left on the row.
fn one_row(ui: &egui::Ui, text: RichText) -> std::sync::Arc<egui::Galley> {
    let room = if ui.layout().main_wrap {
        ui.max_rect().width()
    } else {
        ui.available_width()
    };
    let room = room - 2.0 * ui.spacing().button_padding.x;
    let cut = Some(egui::TextWrapMode::Truncate);
    egui::WidgetText::from(text).into_galley(ui, cut, room, egui::TextStyle::Button)
}

/// The label a control shows: its short one on a shelf or a narrow header.
pub(super) fn label_of(c: &Control, short: bool) -> &str {
    if short { c.short.as_deref() } else { None }.unwrap_or(&c.label)
}
