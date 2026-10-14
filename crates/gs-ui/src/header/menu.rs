//! The open menu under its title and its submenu beside its row: a set's rows, or a dropdown
//! panel for a set with a slider or a piece drawn in code; and a view button's options, in the
//! same dropdown panel under their button. Folded, the button the menus fold into, their list
//! under it and a menu beside its row there (decision 0060 as amended for H7, item 3).

use egui::{Align, Align2, Layout, Pos2, RichText, Shape, Stroke, vec2};
use gs_engine::Hand;
use gs_protocol::Step;

use super::{DROPDOWN, GAP, MENU, Menus, bar, fold_of, pen, spec};
use crate::direction::{Control, HeaderSpec, title};
use crate::floats::scrolled;
use crate::pieces::{
    Dragging, Face, Now, Press, control, controls_of, hover, label, press_of, pressed_as, section,
    stacked,
};
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;
use crate::{descriptions, icons, recent, text};

/// Draws what is open over the 3D view, its room `room`: the folded menus' list under their
/// button ([`list`]), then what is open under its title or button, or beside its row in the
/// list (see [`spot`]). Call after the 3D view, so it lies on top. Returns what was picked or
/// changed.
pub fn menu(
    ui: &egui::Ui,
    now: &Now,
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
    room: egui::Rect,
) -> Vec<Step> {
    let mut asked = list(ui, now, menus, rec, room);
    asked.extend(open(ui, now, menus, dragging, rec, room));
    asked
}

/// What is open: a menu, then its submenu, as a set's rows or as a dropdown panel for a set
/// with a slider or a piece drawn in code, drawn as a tray section; or a view button's options,
/// their pieces each under its title in a dropdown panel by their own id. A dropdown panel stays
/// open after a change; a pick in a menu shuts it. Returns what was picked or changed.
fn open(
    ui: &egui::Ui,
    now: &Now,
    menus: &mut Menus,
    dragging: &mut Dragging,
    rec: &mut Recorder,
    room: egui::Rect,
) -> Vec<Step> {
    let (Some(name), Some(at)) = (menus.open.clone(), menus.at) else {
        return Vec::new();
    };
    let (spot, room) = spot(ui, now, menus.list, at, room);
    let place = (name.as_str(), spot, room);
    let controls = controls_of(&name, now);
    let heading = title(&now.directions.controls, &name);
    let dropped = if menus.options().is_some() {
        let (heading, show) = pen::options_of(&name, now);
        let panel = (name.as_str(), heading.as_str());
        Some(dropdown(ui, panel, place, rec, |ui, rec| {
            stacked(ui, &show, now, dragging, rec)
        }))
    } else if now.directions.controls.piece(&name).is_none()
        || controls.iter().any(|c| c.range.is_some())
    {
        Some(dropdown(ui, ("menu", heading), place, rec, |ui, rec| {
            let mut asked = Vec::new();
            ui.horizontal_wrapped(|ui| asked.extend(section(ui, &name, now, dragging, rec)));
            asked
        }))
    } else {
        None
    };
    if let Some(asked) = dropped {
        // A step that opens a window of its own shuts the panel, so the next press reaches it.
        if (asked.iter()).any(|s| ["ui.palette", "ui.library"].contains(&s.command.as_str())) {
            menus.shut();
        }
        return asked;
    }
    let mut asked = Vec::new();
    let shown = popup(ui, ("header menu", &name), spot, room, |ui| {
        rec.panel("menu", heading, ui.max_rect());
        asked = items(ui, &controls, now, Some(&mut *menus), rec);
    });
    rec.fit(shown);
    if !asked.is_empty() {
        menus.shut();
    }
    asked.extend(submenu(ui, now, menus, rec, room));
    asked
}

/// Where what is open shows, and the room it slides inside (decision 0060 as amended for H7,
/// items 3 and 4): under its title or button `at`, inside `room`; or, opened through the folded
/// menus' list (`listed`), beside its row `at` toward the view, inside the window beside the
/// list, so that it never covers the list.
fn spot(
    ui: &egui::Ui,
    now: &Now,
    listed: bool,
    at: egui::Rect,
    room: egui::Rect,
) -> ((Pos2, Align2), egui::Rect) {
    if listed {
        let window = ui.ctx().content_rect();
        let beside = if now.layout.hand == Hand::Left {
            window.left()..=at.left()
        } else {
            at.right()..=window.right()
        };
        let room = egui::Rect::from_x_y_ranges(beside, window.y_range());
        (toward(at, now, true), room)
    } else {
        ((at.left_bottom(), Align2::LEFT_TOP), room)
    }
}

/// Where a popup opens from `at`, toward the view: under it from its left edge, or beside it to
/// its right when `beside`; for the left hand, under it from its right edge, or to its left.
fn toward(at: egui::Rect, now: &Now, beside: bool) -> (Pos2, Align2) {
    let left = now.layout.hand == Hand::Left;
    let x = if left == beside {
        at.left()
    } else {
        at.right()
    };
    let y = if beside { at.top() } else { at.bottom() };
    let pivot = if left {
        Align2::RIGHT_TOP
    } else {
        Align2::LEFT_TOP
    };
    (egui::pos2(x, y), pivot)
}

/// The button the menus fold into (decision 0060 as amended for H7, item 3): drawn as a title
/// is, lit while their list, or a menu through it, is open. It runs no command: a tap lists the
/// menus under it, or shuts what is open.
pub(super) fn fold(ui: &mut egui::Ui, spec: &HeaderSpec, menus: &mut Menus, rec: &mut Recorder) {
    let (label, hint) = fold_of(spec);
    let shown = pen::opener(ui, (label, None), hint, menus.list, rec);
    if shown.clicked() {
        let list = !menus.list;
        menus.shut();
        menus.list = list;
    }
    menus.fold = Some(shown.rect);
}

/// The folded menus' list while it is open: their titles a row each with ▸ (`bar::titles`),
/// under their button ([`toward`]), between `room`'s sides and inside the window's height,
/// recorded as the panel `menus` titled with the button's label. Returns what a tap asked for.
fn list(
    ui: &egui::Ui,
    now: &Now,
    menus: &mut Menus,
    rec: &mut Recorder,
    room: egui::Rect,
) -> Vec<Step> {
    let (Some(spec), Some(fold)) = (spec(now), menus.fold.filter(|_| menus.list)) else {
        return Vec::new();
    };
    let mut asked = Vec::new();
    // Between the view's sides and in the window's height: it hangs over the header's rows.
    let room = egui::Rect::from_x_y_ranges(room.x_range(), ui.ctx().content_rect().y_range());
    let shown = popup(ui, "header menus", toward(fold, now, false), room, |ui| {
        rec.panel("menus", fold_of(spec).0, ui.max_rect());
        // Rows read as a menu's: no fill at rest, the hover's fill under the pointer. They
        // touch, so in a 640 by 480 window the ten slide up over the button only a few points.
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        ui.spacing_mut().item_spacing.y = 0.0;
        let text = |m: &String| {
            text::width(
                ui,
                title(&now.directions.controls, m),
                egui::TextStyle::Button,
            )
        };
        let widest = spec.menus.iter().map(text).fold(0.0, f32::max);
        ui.set_width(widest + 2.0 * ui.spacing().button_padding.x + GAP);
        ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
            asked = bar::titles(ui, (spec, now), menus, true, rec);
        });
    });
    rec.fit(shown);
    asked
}

/// A dropdown panel, recorded as `panel` titled `heading`, [`DROPDOWN`] wide or as wide as
/// `room` leaves (Deform beside its row in a 640-point window): what `add` draws, at `spot`
/// (under the open menu `name`'s title or button, or beside its row) and inside `room`,
/// scrolling when taller than the room under `spot` (`floats::scrolled`). Returns what it
/// asked to run.
fn dropdown(
    ui: &egui::Ui,
    (panel, heading): (&str, &str),
    (name, (at, pivot), room): (&str, (Pos2, Align2), egui::Rect),
    rec: &mut Recorder,
    add: impl FnOnce(&mut egui::Ui, &mut Recorder) -> Vec<Step>,
) -> Vec<Step> {
    // It hangs from the room's top when `at` is above it, as `popup` moves a panel that fits.
    let top = at.y.max(room.top());
    let margins = frame(ui).total_margin().sum();
    let most = room.bottom() - top - margins.y;
    let wide = DROPDOWN.min(room.width() - margins.x);
    let mut asked = Vec::new();
    let spot = (egui::pos2(at.x, top), pivot);
    let shown = popup(ui, ("header menu", name), spot, room, |ui| {
        rec.panel(panel, heading, ui.max_rect());
        ui.set_width(wide);
        asked = scrolled(ui, most, rec, add);
    });
    rec.fit(shown);
    asked
}

/// The open submenu, beside its row toward the view (to its left for the left hand): Open
/// recent's scenes (`recent::rows`), or a set's rows. A pick shuts it and its menu. Returns what
/// was picked.
fn submenu(
    ui: &egui::Ui,
    now: &Now,
    menus: &mut Menus,
    rec: &mut Recorder,
    room: egui::Rect,
) -> Vec<Step> {
    let (Some(set), Some(at)) = (menus.sub.clone(), menus.sub_at) else {
        return Vec::new();
    };
    let heading = title(&now.directions.controls, &set);
    let mut asked = Vec::new();
    let shown = popup(
        ui,
        ("header submenu", &set),
        toward(at, now, true),
        room,
        |ui| {
            rec.panel("submenu", heading, ui.max_rect());
            if set == recent::SET {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
                    asked.extend(recent::rows(ui, now, rec));
                });
            } else {
                asked = items(ui, &controls_of(&set, now), now, None, rec);
            }
        },
    );
    rec.fit(shown);
    if !asked.is_empty() {
        menus.shut();
    }
    asked
}

/// Draws `add` in a popup's frame with its corner `pivot` at `at`, over everything and inside
/// `room`. Returns where it showed.
fn popup(
    ui: &egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    (at, pivot): (Pos2, Align2),
    room: egui::Rect,
    add: impl FnOnce(&mut egui::Ui),
) -> egui::Rect {
    let frame = frame(ui);
    let area = egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .pivot(pivot)
        .constrain_to(room);
    area.show(ui.ctx(), |ui| frame.show(ui, add)).response.rect
}

/// A popup's frame, with a tray's margin.
fn frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::popup(ui.style()).inner_margin(theme(ui.ctx()).spacing.margin)
}

/// A menu's rows, top to bottom, each control shown by the state: a group's name as a small
/// dim heading over its rows, a thin line between groups and above a control with `separator`,
/// and a row opening a submenu for a control with `options` and no command (`menus` is `None`
/// inside a submenu, which opens none of its own). Returns what was picked.
fn items(
    ui: &mut egui::Ui,
    controls: &[Control],
    now: &Now,
    mut menus: Option<&mut Menus>,
    rec: &mut Recorder,
) -> Vec<Step> {
    let shown: Vec<&Control> = (controls.iter()).filter(|c| c.shown(&now.state)).collect();
    ui.set_width(width(ui, &shown, now));
    // Rows read as a menu's: no fill at rest, the hover's fill under the pointer.
    ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    let mut asked = Vec::new();
    ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
        let mut group = None;
        for (i, line) in lines(&shown).enumerate() {
            let c = line[0];
            if i > 0 && (c.separator || c.group != group) {
                ui.separator();
            }
            if c.group.is_some() && c.group != group {
                let heading = RichText::new(c.group.as_deref().unwrap_or_default());
                label(ui, heading.small().weak(), rec);
            }
            group.clone_from(&c.group);
            match (c.options.as_deref(), menus.as_deref_mut()) {
                (Some(set), Some(menus)) if c.command.is_none() => {
                    submenu_row(ui, c, set, menus, rec);
                }
                _ if c.line.is_some() => asked.extend(line_row(ui, line, now, rec)),
                _ => asked.extend(row(ui, c, true, now, rec)),
            }
        }
    });
    asked
}

/// `shown` by rows: the controls sharing a `line` together, each other control alone.
fn lines<'a, 'c>(shown: &'a [&'c Control]) -> impl Iterator<Item = &'a [&'c Control]> {
    shown.chunk_by(|a, b| a.line.is_some() && a.line == b.line)
}

/// The controls sharing a `line` as one row (decision 0060 as amended for H3): the line's label,
/// then each control by its short label as a button of its own, each a [`row`], and their keys
/// once at the row's right (see [`line_keys`]). Returns what was picked.
fn line_row(ui: &mut egui::Ui, line: &[&Control], now: &Now, rec: &mut Recorder) -> Option<Step> {
    let text = line[0].line.as_deref().unwrap_or_default();
    let shown = ui.horizontal(|ui| {
        // Buttons with their fill at rest, as a tray's line shows them.
        ui.reset_style();
        ui.add_space(ui.spacing().button_padding.x);
        label(ui, RichText::new(text), rec);
        let asked = line.iter().map(|c| row(ui, c, false, now, rec));
        asked.fold(None, Option::or)
    });
    if let Some((keys, held)) = line_keys(line, now) {
        let rect = shown.response.rect.with_max_x(ui.max_rect().right());
        shortcut(ui, (rect, held), &keys, rec);
    }
    shown.inner
}

/// A line's keys as its row shows them (decision 0060 as amended for H4, item 5): the start
/// they share, then the rest of each, `V X, Y, Z` for V X, V Y and V Z; and whether none of its
/// controls can run now. `None` when none has a key.
fn line_keys(line: &[&Control], now: &Now) -> Option<(String, bool)> {
    let presses: Vec<Press> = line.iter().filter_map(|c| press_of(c, true, now)).collect();
    let keys: Vec<&str> = presses.iter().filter_map(|p| p.key).collect();
    let first = *keys.first()?;
    let same = |k: &&str| {
        first
            .bytes()
            .zip(k.bytes())
            .take_while(|(a, b)| a == b)
            .count()
    };
    let shared = keys.iter().map(same).min().unwrap_or(0);
    // Cut after the last `+` or space they share, so no key is split inside a word.
    let cut = first
        .get(..shared)
        .and_then(|s| s.rfind(['+', ' ']))
        .map_or(0, |i| i + 1);
    let rest: Vec<&str> = keys.iter().map(|k| &k[cut..]).collect();
    let held = (presses.iter()).all(|p| now.applies.and_then(|a| a.why(&p.step)).is_some());
    Some((format!("{}{}", &first[..cut], rest.join(", ")), held))
}

/// The key shown at the right of `l`, one of [`lines`]: a line's (see [`line_keys`]) or a
/// row's own.
fn key_of(l: &[&Control], now: &Now) -> Option<String> {
    if l[0].line.is_some() {
        return line_keys(l, now).map(|(keys, _)| keys);
    }
    Some(press_of(l[0], true, now)?.key?.to_string())
}

/// What a row says: its short label, with the state value it names after it as a tray's button
/// reads it, then "…" when it opens the palette's form first.
fn item_label(c: &Control, now: &Now) -> String {
    let short = c.short.as_deref().unwrap_or(&c.label);
    let named = c.named(short, &now.state);
    let named = named.unwrap_or_else(|| short.to_string());
    let form = c.command.as_deref() == Some("ui.palette") && c.args.contains_key("form");
    if form { format!("{named}…") } else { named }
}

/// A menu's width, in points: its longest label beside an icon, a gap and its longest key,
/// with every key starting past its row's middle, or its widest [`line_row`] with its keys; at
/// least [`MENU`].
fn width(ui: &egui::Ui, shown: &[&Control], now: &Now) -> f32 {
    let text = |t: &str| text::width(ui, t, egui::TextStyle::Button);
    let spacing = ui.spacing();
    let icon = ui.text_style_height(&egui::TextStyle::Button) + spacing.icon_spacing;
    let labels = shown.iter().map(|c| text(&item_label(c, now)));
    let label = labels.fold(0.0, f32::max) + icon;
    let keys = lines(shown).filter_map(|l| key_of(l, now));
    let key = keys.map(|k| text(&k)).fold(0.0, f32::max);
    let pad = 2.0 * spacing.button_padding.x;
    let button = |c: &&Control| spacing.item_spacing.x + text(&item_label(c, now)) + pad;
    let line_row = |l: &[&Control]| {
        let keys = key_of(l, now).map_or(0.0, |k| GAP + text(&k));
        Some(text(l[0].line.as_deref()?) + l.iter().map(button).sum::<f32>() + keys)
    };
    let line = lines(shown).filter_map(line_row).fold(0.0, f32::max);
    let row = (label + GAP + key).max(2.0 * key + GAP);
    (row.max(line) + pad).max(MENU)
}

/// A row: its label (see [`item_label`]) and, when `keyed`, its key at its right (a line's
/// buttons leave theirs to [`line_row`]); gray while it cannot run, with why as its hint (rule
/// 7), and otherwise its own hint. A control waiting for a feature shows as it does anywhere.
/// Returns its step when picked.
fn row(ui: &mut egui::Ui, c: &Control, keyed: bool, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let Some(press) = press_of(c, true, now) else {
        return control(ui, c, true, now, rec);
    };
    let shown = item_label(c, now);
    let (step, key) = (press.step.clone(), press.key);
    let held = now.applies.and_then(|a| a.why(&step));
    let press = Press {
        label: &shown,
        ..press
    };
    let button = pressed_as(ui, press, Face::of(icons::of(c)), now, rec);
    if let Some(hint) = held.or(c.hint.as_deref()) {
        rec.hint(hint);
    }
    if let Some(hint) = &c.hint {
        descriptions::hint(ui, &button, hint);
    }
    if let Some(key) = key.filter(|_| keyed) {
        shortcut(ui, (button.rect, held.is_some()), key, rec);
    }
    button.clicked().then_some(step)
}

/// `key` at the right of the row `rect`, dim (and gray on a row that cannot run), recorded as a
/// label.
fn shortcut(ui: &egui::Ui, (rect, held): (egui::Rect, bool), key: &str, rec: &mut Recorder) {
    let colors = &theme(ui.ctx()).colors;
    let color = if held {
        colors.text_off
    } else {
        colors.text_dim
    };
    let at = egui::pos2(
        rect.right() - ui.spacing().button_padding.x,
        rect.center().y,
    );
    let font = egui::TextStyle::Button.resolve(ui.style());
    let shown = (ui.painter()).text(at, Align2::RIGHT_CENTER, key, font, color);
    rec.widget(Widget::plain(key, Role::Label), shown);
}

/// A row opening the submenu `set` beside it: a tap opens it, or shuts it when it is open; ▸ at
/// its right, and lit while it is open.
fn submenu_row(ui: &mut egui::Ui, c: &Control, set: &str, menus: &mut Menus, rec: &mut Recorder) {
    let open = menus.sub.as_deref() == Some(set);
    let shown = c.short.as_deref().unwrap_or(&c.label);
    let colors = &theme(ui.ctx()).colors;
    let button = if open {
        let text = RichText::new(shown).color(colors.on_lit);
        egui::Button::new(text).fill(colors.lit).selected(true)
    } else {
        egui::Button::new(shown)
    };
    let button = ui.add(button);
    rec.widget(
        Widget::plain(shown, Role::Button).selected(open),
        button.rect,
    );
    let button = hover(button, c.hint.as_deref(), rec);
    arrow(ui, button.rect, open);
    if button.clicked() {
        menus.sub = (!open).then(|| set.to_string());
    }
    if menus.sub.as_deref() == Some(set) {
        menus.sub_at = Some(button.rect);
    }
}

/// ▸ at the right of the row `rect`, in the lit text's color while what it opens is `open`.
pub(super) fn arrow(ui: &egui::Ui, rect: egui::Rect, open: bool) {
    let colors = &theme(ui.ctx()).colors;
    let color = if open { colors.on_lit } else { colors.text_dim };
    let tip = egui::pos2(
        rect.right() - ui.spacing().button_padding.x,
        rect.center().y,
    );
    let points = vec![tip, tip + vec2(-6.0, 4.5), tip + vec2(-6.0, -4.5)];
    (ui.painter()).add(Shape::convex_polygon(points, color, Stroke::NONE));
}
