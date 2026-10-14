//! The 3D view's gesture map (decision 0017): which press and drag runs which view command.
//! Every row calls the engine's View commands, so the window navigates exactly as an agent or
//! a replay would; a drag is one gesture (one camera history step and one log line).
//!
//! | Press | Drag | Alt | Ctrl | Shift during | Shift tapped first |
//! |---|---|---|---|---|---|
//! | barrel button or right mouse, anywhere on the view | orbit | pan | dolly | snap | roll |
//! | pen tip or left mouse on empty canvas (classic), unless the brush draws a gesture | orbit | pan; tapped first: dolly | | snap | roll |
//! | barrel button 2 or middle mouse, anywhere | pan | | | | |
//! | any press on the corner figure | orbit | | | snap | |
//! | any press on the Zoom button under it | dolly, up comes closer | | | | |
//! | any press on the Pan button | pan | | | | |
//!
//! The mouse wheel dollies toward the surface under the pointer, one step per notch. Whether a
//! tip press is on empty canvas is the engine's answer (the stroke it starts hits no part, and
//! a gesture, which needs no surface, never is: decision 0017 as amended 2026-10-12); a locked
//! view navigates with nothing. Speeds come from the navigation settings, in points.

use egui::{Modifiers, PointerButton, Pos2, Response};
use gs_engine::{MainView, Navigation};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::record::with_args;

/// One drag's move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Orbit,
    Pan,
    Dolly,
    Roll,
}

/// A navigation drag in progress.
#[derive(Clone, Copy, Debug)]
struct Drag {
    id: u64,
    row: Row,
    button: PointerButton,
    /// It started on one of the drag buttons under the corner figure.
    pad: bool,
}

/// The gesture map's state between frames.
#[derive(Clone, Debug, Default)]
pub struct Navigator {
    /// The id the next gesture gets.
    next: u64,
    drag: Option<Drag>,
    /// Alt or Shift pressed and let go with no press in between: the next drag dollies (Alt)
    /// or rolls (Shift), the classic timing.
    latched: Modifiers,
    /// The taps the last press took with it, for the drag it starts.
    armed: Modifiers,
    /// The modifiers held last frame.
    held: Modifiers,
    /// A pointer button went down while modifiers were held: letting go latches nothing.
    spoiled: bool,
    /// A button that navigates is down on the view and has not dragged yet.
    pressed: bool,
}

/// What the gesture map reads in a frame besides the pointer.
pub struct Context<'a> {
    /// The 3D view's rectangle, in points.
    pub rect: egui::Rect,
    /// Pixels per point.
    pub ppp: f32,
    pub main: &'a MainView,
    pub settings: &'a Navigation,
    /// The pen's tip pressed on empty canvas and has not lifted.
    pub canvas: bool,
    /// The corner figure's area, if it shows: a drag that starts there orbits.
    pub figure: Option<egui::Rect>,
    /// The drag buttons under it and the row a drag that starts on each runs.
    pub pads: Vec<(Row, egui::Rect)>,
}

impl Navigator {
    /// A gesture map whose gestures are numbered from `first` up, so two maps (the 3D view's
    /// and the Reference View's) never hand the engine the same gesture id.
    pub fn numbered_from(first: u64) -> Self {
        Self {
            next: first,
            ..Self::default()
        }
    }

    /// The row a drag is in, if it navigates: what was pressed, the modifiers held and those
    /// tapped before.
    fn row(&self, button: PointerButton, held: Modifiers, canvas: bool) -> Option<Row> {
        let tapped = self.armed;
        match button {
            PointerButton::Middle => Some(Row::Pan),
            PointerButton::Secondary if held.alt => Some(Row::Pan),
            PointerButton::Secondary if held.ctrl => Some(Row::Dolly),
            PointerButton::Primary if !canvas => None,
            PointerButton::Primary if held.alt => Some(Row::Pan),
            PointerButton::Primary if tapped.alt => Some(Row::Dolly),
            PointerButton::Secondary | PointerButton::Primary if tapped.shift => Some(Row::Roll),
            PointerButton::Secondary | PointerButton::Primary => Some(Row::Orbit),
            _ => None,
        }
    }

    /// Keeps track of modifiers tapped on their own. A modifier pressed and let go while a
    /// pointer button is down (the pen's tip in a stroke) is part of that drag, not a tap
    /// (decision 0055, section 6).
    fn latch(&mut self, ui: &egui::Ui) {
        let (now, pressed, down) = ui.input(|i| {
            let key =
                (i.events.iter()).any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
            (
                i.modifiers,
                i.pointer.any_pressed() || key,
                i.pointer.any_down(),
            )
        });
        self.spoiled |= pressed || down;
        if pressed {
            self.armed = std::mem::replace(&mut self.latched, Modifiers::NONE);
        }
        let was = self.held;
        if !self.spoiled {
            self.latched.alt |= was.alt && !now.alt;
            self.latched.shift |= was.shift && !now.shift;
        }
        if !now.alt && !now.shift && !now.ctrl {
            self.spoiled = false;
        }
        self.held = now;
    }

    /// One frame of pointer input over the 3D view: the view commands it asks for.
    pub fn frame(&mut self, ui: &egui::Ui, response: &Response, cx: &Context) -> Vec<Step> {
        self.latch(ui);
        self.pressed = false;
        let mut steps = Vec::new();
        if cx.main.locked {
            self.drag = None;
            return steps;
        }
        let held = ui.input(|i| i.modifiers);
        let pixel = |p: Pos2| ((p - cx.rect.min) * cx.ppp).to_pos2();
        if self.drag.is_none() {
            let buttons = [
                PointerButton::Secondary,
                PointerButton::Middle,
                PointerButton::Primary,
            ];
            let started = buttons.into_iter().find(|b| response.drag_started_by(*b));
            let origin = ui.input(|i| i.pointer.press_origin());
            let on_figure = origin.zip(cx.figure).is_some_and(|(o, f)| f.contains(o));
            let on_pad = (cx.pads.iter())
                .find(|(_, r)| origin.is_some_and(|o| r.contains(o)))
                .map(|(row, _)| *row);
            let row = |b| match b {
                _ if on_pad.is_some() => on_pad,
                PointerButton::Primary if on_figure => Some(Row::Orbit),
                _ => self.row(b, held, cx.canvas),
            };
            let down = |b| ui.input(|i| i.pointer.button_down(b));
            let pressed = origin.is_some_and(|o| cx.rect.contains(o))
                && buttons.into_iter().any(|b| down(b) && row(b).is_some());
            if let Some((button, row)) = started.and_then(|b| Some((b, row(b)?))) {
                self.next += 1;
                self.armed = Modifiers::NONE;
                let at = origin.map(pixel);
                self.drag = Some(Drag {
                    id: self.next,
                    row,
                    button,
                    pad: on_pad.is_some(),
                });
                // The press carries the pixel the pivot may come from.
                let mut first = self.step(row, egui::Vec2::ZERO, held, cx);
                if let (Some(at), Some(args)) = (at, first.args.as_object_mut()) {
                    args.insert("at".into(), json!([at.x, at.y]));
                }
                steps.push(first);
            }
            self.pressed = pressed;
        }
        if let Some(drag) = self.drag {
            if response.dragged_by(drag.button) {
                let delta = response.drag_delta();
                if delta != egui::Vec2::ZERO {
                    steps.push(self.step(drag.row, delta, held, cx));
                }
            }
            if response.drag_stopped() || !ui.input(|i| i.pointer.button_down(drag.button)) {
                let mut last = self.step(drag.row, egui::Vec2::ZERO, held, cx);
                if let Some(args) = last.args.as_object_mut() {
                    args.insert("end".into(), true.into());
                }
                steps.push(last);
                self.drag = None;
            }
        }
        steps.extend(wheel(ui, response, cx, pixel));
        steps
    }

    /// The command for one frame of a drag in `row` by `delta` points.
    fn step(&self, row: Row, delta: egui::Vec2, held: Modifiers, cx: &Context) -> Step {
        let speed = cx.settings.orbit_speed;
        let id = self.drag.map_or(self.next, |d| d.id);
        let (command, mut args) = match row {
            Row::Orbit => (
                "view.orbit",
                json!({ "right_deg": -delta.x * speed, "up_deg": delta.y * speed, "snap": held.shift }),
            ),
            Row::Pan => (
                "view.pan",
                json!({ "dx": delta.x * cx.ppp, "dy": delta.y * cx.ppp }),
            ),
            Row::Dolly => {
                let factor = libm::powf(1.0 + cx.settings.dolly_speed, delta.y);
                ("view.dolly", json!({ "factor": factor }))
            }
            Row::Roll => ("view.roll", json!({ "deg": -delta.x * speed })),
        };
        if let Value::Object(map) = &mut args {
            map.insert("gesture".into(), id.into());
        }
        with_args(command, args)
    }

    /// Whether the view is being dragged, from the press of a button that navigates to its
    /// release.
    pub fn navigating(&self) -> bool {
        self.drag.is_some() || self.pressed
    }

    /// The row of the drag in progress, if one is navigating.
    pub fn dragging(&self) -> Option<Row> {
        self.drag.map(|d| d.row)
    }

    /// The row of the drag in progress, if it started on a drag button under the figure.
    pub fn dragging_pad(&self) -> Option<Row> {
        self.drag.filter(|d| d.pad).map(|d| d.row)
    }
}

/// The dolly the wheel turned over the view this frame, toward the pointer (`pixel` turns a
/// point into the view's pixels), if it turned.
fn wheel(
    ui: &egui::Ui,
    response: &Response,
    cx: &Context,
    pixel: impl Fn(Pos2) -> Pos2,
) -> Option<Step> {
    let wheel: f32 = ui.input(|i| {
        (i.raw.events.iter())
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum()
    });
    if !response.hovered() || wheel == 0.0 {
        return None;
    }
    // Each notch toward the screen comes 10% (the wheel step) closer.
    let factor = libm::powf(1.0 - cx.settings.wheel_step, wheel);
    let mut args = json!({ "factor": factor });
    if let Some(p) = response.hover_pos().map(pixel) {
        args["toward"] = json!([p.x, p.y]);
    }
    Some(with_args("view.dolly", args))
}
