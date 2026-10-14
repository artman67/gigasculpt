//! The wheel while the quick menu shows (decision 0025 section 2, amended 2026-10-12; catalog
//! `interface.quick-menu`): a notch the keymap does not bind steps the brush's Size
//! (`brush.set` `size_step`), with or without modifiers, wherever the pointer is, so the wheel
//! right after the menu opens sizes the brush; with a slider shown alone it steps that slider,
//! a hundredth of its range a notch. Over a slider (one of the menu's, a tray's) a notch is
//! left to the slider, which steps a hundredth of its rail; not over a slider alone's own,
//! whose knob opens under the pointer and which keeps its own step. The notches it takes
//! neither dolly the view nor scroll a tray; part of a notch waits for the rest.

use egui::{Event, Pos2};
use gs_engine::{PanelInfo, SessionState};
use gs_protocol::Step;
use serde_json::json;

use super::{Open, brush_value, slider_control};
use crate::descriptions::{inside, pressed_panel};
use crate::direction::{Controls, put};
use crate::record::with_args;
use crate::values::Value;

/// What the quick menu's wheel reads: the panels recorded last frame (in pixels), the pointer
/// as of then (in points), pixels per point, the session's state and the controls files.
pub struct Turns<'a> {
    pub panels: &'a [PanelInfo],
    pub pointer: Option<Pos2>,
    pub ppp: f32,
    pub state: &'a SessionState,
    pub controls: &'a Controls,
}

impl Open {
    /// Takes the notches it steps out of `events` (in points), and returns what they run.
    pub fn wheel(&mut self, events: &mut Vec<Event>, t: &Turns) -> Vec<Step> {
        let mut pointer = t.pointer;
        let mut asked = Vec::new();
        let alone = self.slider.is_some();
        events.retain(|e| match e {
            Event::PointerMoved(p) => {
                pointer = Some(*p);
                true
            }
            Event::MouseWheel { delta, .. } if delta.y != 0.0 => {
                if pointer.is_some_and(|p| over_slider(p * t.ppp, t.panels, alone)) {
                    return true;
                }
                asked.extend(self.turn(delta.y, t));
                false
            }
            _ => true,
        });
        asked
    }

    /// `dy` notches more: what the whole notches turned so far step.
    fn turn(&mut self, dy: f32, t: &Turns) -> Option<Step> {
        if self.turned * dy < 0.0 {
            self.turned = 0.0;
        }
        self.turned += dy;
        let whole = self.turned.trunc();
        self.turned -= whole;
        if whole == 0.0 {
            return None;
        }
        match self.slider.as_deref() {
            None => Some(size_step(whole)),
            Some(label) => stepped(label, whole, t),
        }
    }
}

/// `brush.set` growing the brush `by` steps, or shrinking it.
fn size_step(by: f32) -> Step {
    with_args("brush.set", json!({ "size_step": by as i32 }))
}

/// The slider labelled `label` moved `by` hundredths of its range: Size by its steps.
fn stepped(label: &str, by: f32, t: &Turns) -> Option<Step> {
    if let Some(v) = brush_value(label) {
        if v == Value::Size {
            return Some(size_step(by));
        }
        let [low, high] = v.range();
        let to = v.of(&t.state.brush) + by * (high - low) / 100.0;
        return Some(with_args(
            "brush.set",
            json!({ v.arg(): to.clamp(low, high) }),
        ));
    }
    let c = slider_control(t.controls, label)?;
    let (arg, [low, high]) = (c.dragged()?, c.range?);
    let state = t.state.values();
    let shows = c.shows.as_deref().and_then(|s| state.get(s));
    let now = shows
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default();
    let to = now as f32 + by * (high - low) / 100.0;
    let mut step = c.step()?;
    put(&mut step.args, &arg, json!(to.clamp(low, high)));
    Some(step)
}

/// Whether a slider is at `at` (in pixels) in the panel on top there; with a slider `alone`,
/// not that one, in the quick menu's window.
fn over_slider(at: Pos2, panels: &[PanelInfo], alone: bool) -> bool {
    let top = pressed_panel(panels, at).filter(|p| !(alone && p.id == "quick"));
    top.is_some_and(|p| (p.widgets.iter()).any(|w| w.role == "slider" && inside(w.rect, at)))
}
