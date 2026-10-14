//! Binding a key from a control (decision 0025 section 5, amended 2026-10-12; catalog
//! `interface.gesture-map-widgets`). A Ctrl+Alt press on a button or switch with a command aims
//! at it without running it, and the status line asks for a key ([`Catch::prompt`]). The next
//! key, with the modifiers held then, binds to the control's command through `ui.keymap`, with
//! its arguments as the controls file writes them, so a switch's `"flip"` key switches it; a
//! wheel notch with a modifier held binds that notch, and a plain notch goes on as usual.
//! Escape cancels; any other press cancels and goes on as it would have, and another Ctrl+Alt
//! press on a control aims at that one. A key another binding holds is refused by `ui.keymap`
//! naming the holder, and the palette's form asks to replace it (`Shell::refused`).
//!
//! It finds the control in the last frame's record, as an agent reads `ui.describe`, and takes
//! its events before the keys and egui see them, so no button draws anything for it. Sliders
//! and list rows are not aimed at: their commands are set by a drag or picked from a list.

use egui::{Event, Key, Modifiers, Pos2};
use gs_engine::PanelInfo;
use gs_protocol::Step;
use serde_json::Value;

use crate::descriptions::{inside, pressed_panel};
use crate::direction::Controls;
use crate::keymap::{Chord, Press};

/// The control a Ctrl+Alt press aimed at, if any, and whether that press is still down.
#[derive(Debug, Default)]
pub struct Catch {
    aim: Option<Aim>,
    /// The aiming press has not been let go: its release is taken too.
    pressing: bool,
}

/// A control aimed at: its label, and its command with its arguments as written.
#[derive(Debug)]
struct Aim {
    label: String,
    command: String,
    args: Value,
}

/// What became of one event.
enum Taken {
    Passed,
    Eaten,
    Bound(Step),
}

impl Catch {
    /// What the status line asks while a control is aimed at.
    pub fn prompt(&self) -> Option<String> {
        let label = &self.aim.as_ref()?.label;
        Some(format!("Press a key for {label} (Escape cancels)"))
    }

    /// Whether a control is aimed at, waiting for its key.
    pub fn aiming(&self) -> bool {
        self.aim.is_some()
    }

    /// Escape, given by the window (`shell/escape.rs`): cancels the aim, binding nothing.
    pub fn escape(&mut self) {
        self.aim = None;
    }

    /// Takes out of `events` (in points, `ppp` pixels each) what aims or binds, by the panels
    /// recorded last frame (in pixels) and the controls file. Returns the `ui.keymap` steps a
    /// key or a notch asked for.
    pub fn take(
        &mut self,
        events: &mut Vec<Event>,
        panels: &[PanelInfo],
        controls: &Controls,
        ppp: f32,
    ) -> Vec<Step> {
        let mut asked = Vec::new();
        events.retain(|e| match self.event(e, panels, controls, ppp) {
            Taken::Passed => true,
            Taken::Eaten => false,
            Taken::Bound(step) => {
                asked.push(step);
                false
            }
        });
        asked
    }

    fn event(&mut self, e: &Event, panels: &[PanelInfo], controls: &Controls, ppp: f32) -> Taken {
        match e {
            Event::PointerButton {
                pos,
                pressed: true,
                modifiers,
                ..
            } => {
                let both = modifiers.ctrl && modifiers.alt;
                self.aim = both.then(|| aimed(*pos * ppp, panels, controls)).flatten();
                self.pressing = self.aim.is_some();
                if self.pressing {
                    Taken::Eaten
                } else {
                    Taken::Passed
                }
            }
            Event::PointerButton { pressed: false, .. } if self.pressing => {
                self.pressing = false;
                Taken::Eaten
            }
            // The window's to give, which cancels the aim (`escape`).
            Event::Key {
                key: Key::Escape, ..
            } => Taken::Passed,
            Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => match self.aim.take() {
                Some(aim) => Taken::Bound(aim.bind(&Chord::pressed(*key, *modifiers))),
                None => Taken::Passed,
            },
            Event::MouseWheel {
                delta, modifiers, ..
            } if delta.y != 0.0 && held(*modifiers) => {
                let press = if delta.y > 0.0 {
                    Press::WheelUp
                } else {
                    Press::WheelDown
                };
                match self.aim.take() {
                    Some(aim) => Taken::Bound(aim.bind(&Chord::pressed(press, *modifiers))),
                    None => Taken::Passed,
                }
            }
            _ => Taken::Passed,
        }
    }
}

impl Aim {
    /// `ui.keymap` binding `chord` to this control's command.
    fn bind(self, chord: &Chord) -> Step {
        let runs = Step {
            command: self.command,
            args: self.args,
        };
        crate::keymap::binding(&chord.to_string(), &runs)
    }
}

/// Whether a modifier key is held, which makes a wheel notch a key to bind.
fn held(m: Modifiers) -> bool {
    m.ctrl || m.alt || m.shift || m.mac_cmd
}

/// The button or switch with a command at `at` (in pixels) in the panel on top there, with the
/// arguments the controls file writes for a control of its label and command, else those its
/// press runs.
fn aimed(at: Pos2, panels: &[PanelInfo], controls: &Controls) -> Option<Aim> {
    let panel = pressed_panel(panels, at)?;
    let pressable = |role: &str| role == "button" || role == "toggle";
    let widget = (panel.widgets.iter()).find(|w| pressable(&w.role) && inside(w.rect, at))?;
    let step = widget.command.as_ref()?;
    let all = controls.sets.iter().flat_map(|s| &s.controls);
    let written = all
        .filter(|c| c.label == widget.label && c.command.as_deref() == Some(&step.command))
        .find_map(|c| serde_json::to_value(&c.args).ok());
    Some(Aim {
        label: widget.label.clone(),
        command: step.command.clone(),
        args: written.unwrap_or_else(|| step.args.clone()),
    })
}
