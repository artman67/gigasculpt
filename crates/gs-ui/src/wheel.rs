//! The brush wheel: one ring of a set's controls around the point where it opened, the first
//! at the top and the rest clockwise, with the previous brush in the middle. It opens a little
//! away from the hand that holds the pen (`ui.settings`' `wheel_offset`, mirrored with the
//! layout's hand), so the hand hides less of it. Moving the pointer out of the middle points
//! at a slot, which lights up; letting go (`ui.wheel` with `open: false`, which the wheel key
//! sends when it comes up) runs the slot pointed at, so a flick picks a brush without waiting.
//! A letter jumps to the next slot whose label starts with it, which letting go then picks
//! wherever the pointer is. Clicking a slot picks it too; a right click puts the brush in use
//! in that slot (`ui.wheel` with `slot` and `brush`); Escape closes it. One level, nothing
//! nested.

use std::f32::consts::TAU;

use egui::{Event, Key, Pos2, Stroke, Vec2};
use gs_engine::Hand;
use gs_protocol::Step;
use serde_json::json;

use crate::direction::Control;
use crate::layout::Layout;
use crate::pieces::{Now, control, controls_of};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// The middle's radius, in points: inside it the pointer points at nothing. The wheel's
/// offset from the pen stays inside it, so letting go where it opened picks nothing.
pub const MIDDLE: f32 = 48.0;
/// A slot's size, in points; a longer label wraps to two lines.
const SLOT: Vec2 = egui::vec2(104.0, 56.0);

/// The wheel's state: where it is open, and the slot a letter jumped to.
#[derive(Clone, Debug, Default)]
pub struct Wheel {
    pub at: Option<Pos2>,
    /// The slot the last letter jumped to: letting go picks it wherever the pointer is.
    jumped: Option<usize>,
}

/// The direction of slot `i` of `n`, clockwise from straight up, as a unit vector (screen y
/// points down).
fn toward(i: usize, n: usize) -> Vec2 {
    let angle = TAU * i as f32 / n as f32;
    egui::vec2(libm::sinf(angle), -libm::cosf(angle))
}

/// The slot of `n` the pointer at `offset` from the middle points at, if it is out of the
/// middle: the one whose direction is closest.
fn pointed(offset: Vec2, n: usize) -> Option<usize> {
    if offset.length() < MIDDLE {
        return None;
    }
    let turn = libm::atan2f(offset.x, -offset.y).rem_euclid(TAU);
    Some((turn / TAU * n as f32).round() as usize % n.max(1))
}

/// The wheel's slots and radius in the layout shown: the brushes the layout state's edited
/// ring names (a brush no longer shipped is left out), else the piece the layout rings.
pub fn slots(now: &Now) -> (Vec<Control>, f32) {
    let Some(spec) = now.direction.and_then(|d| d.wheel.as_ref()) else {
        return Default::default();
    };
    let ring = &now.layout.wheel;
    if ring.is_empty() {
        return (controls_of(&spec.show, now), spec.radius);
    }
    let brushes = now.directions.brush_controls();
    let named = |name: &String| brushes.iter().find(|c| &c.label == name).cloned();
    (ring.iter().filter_map(named).collect(), spec.radius)
}

/// The brush each of `slots` picks, by name: a slot that is no brush's by its label.
pub fn ring(slots: &[Control]) -> Vec<String> {
    let brush = |c: &Control| {
        c.args
            .get("brush")
            .and_then(|b| b.as_str().map(str::to_string))
    };
    (slots.iter())
        .map(|c| brush(c).unwrap_or_else(|| c.label.clone()))
        .collect()
}

impl Wheel {
    /// Opens it for the pen at `pen`: its middle `layout`'s wheel offset away from the hand
    /// that holds the pen, to the pen's left for a right hand and to its right for a left.
    pub fn open(&mut self, pen: Pos2, layout: &Layout) {
        let away = match layout.hand {
            Hand::Right => -1.0,
            Hand::Left => 1.0,
        };
        self.at = Some(pen + egui::vec2(away * layout.settings.wheel_offset, 0.0));
        self.jumped = None;
    }

    /// The slot of `n` letting go would pick: the one a letter jumped to, else the one the
    /// pointer at `pointer` points at.
    fn aimed(&self, pointer: Option<Pos2>, n: usize) -> Option<usize> {
        let at = self.at?;
        (self.jumped).or_else(|| pointed(pointer? - at, n))
    }

    /// Closes it. Returns the control a letter jumped to or the pointer at `pointer` points
    /// at, if any.
    pub fn let_go<'c>(
        &mut self,
        pointer: Option<Pos2>,
        slots: &'c [Control],
    ) -> Option<&'c Control> {
        let aimed = self.aimed(pointer, slots.len());
        *self = Self::default();
        slots.get(aimed?)
    }

    /// Takes `e` if it is a letter pressed without Ctrl or Alt while it is open and some
    /// slot's label starts with it: it jumps to the next such slot after the one it jumped to
    /// last, so the same letter again steps through them. Returns whether it took it; a letter
    /// no slot starts with goes on to the keymap.
    pub fn letter(&mut self, e: &Event, slots: &[Control]) -> bool {
        let Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = e
        else {
            return false;
        };
        let name = key.name();
        let letter = name.len() == 1 && name.bytes().all(|b| b.is_ascii_alphabetic());
        if self.at.is_none() || !letter || modifiers.ctrl || modifiers.alt || modifiers.command {
            return false;
        }
        let starts = |c: &Control| (c.label.get(..1)).is_some_and(|f| f.eq_ignore_ascii_case(name));
        let n = slots.len();
        let after = self.jumped.map_or(0, |j| j + 1);
        let next = (0..n).map(|k| (after + k) % n).find(|&i| starts(&slots[i]));
        self.jumped = next.or(self.jumped);
        next.is_some()
    }

    /// Draws it if it is open, ringing `slots` at `radius` points. Returns what a click on a
    /// slot or the middle asked to run, which closes it, or what a right click on a slot asked
    /// (the brush in use put there), which leaves it open.
    pub fn show(
        &mut self,
        ui: &egui::Ui,
        slots: &[Control],
        radius: f32,
        now: &Now,
        rec: &mut Recorder,
    ) -> Option<Step> {
        let at = self.at?;
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            *self = Self::default();
            return None;
        }
        let aimed = self.aimed(ui.ctx().pointer_latest_pos(), slots.len());
        let whole = egui::Rect::from_center_size(at, Vec2::splat(2.0 * radius) + SLOT * 2.0);
        let area = egui::Area::new(egui::Id::new("brush wheel"))
            .order(egui::Order::Foreground)
            .fixed_pos(whole.min);
        let picked = area.show(ui.ctx(), |ui| {
            rec.panel("wheel", "Brush wheel", whole);
            // A dark band behind the slots, so labels read over any surface.
            let colors = &theme(ui.ctx()).colors;
            let band = Stroke::new(SLOT.y + 16.0, colors.backdrop);
            ui.painter().circle_stroke(at, radius, band);
            ui.painter().circle_filled(at, MIDDLE, colors.backdrop);
            let ring = Stroke::new(1.5, colors.text_dim);
            ui.painter().circle_stroke(at, MIDDLE, ring);
            let mut asked = middle(ui, at, now, rec);
            let mut edit = None;
            for (i, slot) in slots.iter().enumerate() {
                let place =
                    egui::Rect::from_center_size(at + toward(i, slots.len()) * radius, SLOT);
                if aimed == Some(i) {
                    let glow = place.expand(5.0);
                    ui.painter().rect_filled(glow, 8.0, colors.lit);
                    let note = format!("Let go for {}", slot.label);
                    rec.widget(Widget::plain(note, Role::Label), glow);
                }
                asked = put(ui, place, slot, now, rec).or(asked);
                edit = edit.or_else(|| right_click(ui, place, i, now));
            }
            (asked, edit)
        });
        let (asked, edit) = picked.inner;
        if asked.is_some() {
            *self = Self::default();
        }
        asked.or(edit)
    }
}

/// What a right click on slot `i` at `place` asks: the brush in use put in it.
fn right_click(ui: &egui::Ui, place: egui::Rect, i: usize, now: &Now) -> Option<Step> {
    let over = ui
        .input(|p| p.pointer.interact_pos())
        .is_some_and(|p| place.contains(p));
    let clicked = over && ui.input(|p| p.pointer.secondary_clicked());
    let put = json!({ "slot": i, "brush": now.brush.brush });
    clicked.then(|| with_args("ui.wheel", put))
}

/// The middle: the previous brush, a click away.
fn middle(ui: &mut egui::Ui, at: Pos2, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let previous = now.brush.previous.as_deref()?;
    let label = format!("Back to {previous}");
    let back = Control::runs(
        &label,
        "brush.set",
        &serde_json::json!({ "previous": true }),
    );
    put(ui, egui::Rect::from_center_size(at, SLOT), &back, now, rec)
}

/// A control filling `place`, its label wrapped to fit.
fn put(
    ui: &mut egui::Ui,
    place: egui::Rect,
    c: &Control,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
    let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(place).layout(layout));
    inside.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    control(&mut inside, c, false, now, rec)
}
