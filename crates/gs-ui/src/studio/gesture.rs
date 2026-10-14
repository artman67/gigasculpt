//! The pen's presses on the 3D view and its gestures. One classifier (catalog
//! `interface.gesture-map`) decides, when the tip touches or barrel button 1 goes down, which
//! machine owns the press until it ends: a table of rows in the gesture map's precedence (an
//! interface control, then a gizmo handle, then the model or empty canvas). A new gesture is a
//! row and its machine. The gestures' machines live here: Alt with a barrel click picks a
//! part, and Ctrl alone on empty canvas works the mask. The stroke is in `pen.rs`, the gizmo
//! tool in `crate::gizmo`.

use egui::Modifiers;
use gs_engine::PenReads;
use gs_protocol::{PenSample, Step};
use serde_json::json;

use super::Studio;
use crate::held;
use crate::record::with_args;

/// The machine a press goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Machine {
    /// Off the 3D view: the panels and the Zoom and Pan buttons take it through egui, and it
    /// never sculpts, even when it slides onto the view.
    Interface,
    /// Alt with a barrel click: the click picks the part under it ([`Studio::pick`]); a drag
    /// navigates, as every barrel press does.
    Pick,
    /// Barrel button 1 without Alt: it turns the view (`navigate.rs`, through egui's pointer).
    Navigate,
    /// The gizmo tool: on a handle it drags, on the model it places the gizmo, on empty
    /// canvas it navigates (`gizmo::Tool::sample`).
    Gizmo,
    /// Ctrl alone: on the model a Mask Pen stroke, on empty canvas a gesture on the mask, run
    /// when the pen lifts (decision 0017 as amended, item 10).
    Mask,
    /// A stroke: on the model it sculpts, on empty canvas it navigates. The engine tells which.
    Stroke,
}

/// The button whose press is classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    /// The pen's tip touching.
    Tip,
    /// Barrel button 1.
    Barrel,
}

/// The held keys a row asks for.
#[derive(Clone, Copy)]
enum Keys {
    /// Whatever is held.
    Any,
    /// Alt, with or without others.
    Alt,
    /// Ctrl and nothing else ([`held::background`]).
    CtrlAlone,
}

impl Keys {
    fn hold(self, m: Modifiers) -> bool {
        match self {
            Keys::Any => true,
            Keys::Alt => m.alt,
            Keys::CtrlAlone => held::background(m),
        }
    }
}

/// The tool a row asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Whichever is on.
    Any,
    /// The gizmo tool.
    Gizmo,
}

/// One row: a press of a button over the 3D view, with these keys held and this tool on, goes
/// to a machine.
struct Row(Button, Keys, Tool, Machine);

/// The rows, in the gesture map's precedence: the first that matches owns the press.
const ROWS: [Row; 5] = [
    Row(Button::Barrel, Keys::Alt, Tool::Any, Machine::Pick),
    Row(Button::Barrel, Keys::Any, Tool::Any, Machine::Navigate),
    Row(Button::Tip, Keys::Any, Tool::Gizmo, Machine::Gizmo),
    Row(Button::Tip, Keys::CtrlAlone, Tool::Any, Machine::Mask),
    Row(Button::Tip, Keys::Any, Tool::Any, Machine::Stroke),
];

/// What is read when a press starts.
struct Press {
    button: Button,
    held: Modifiers,
    /// The 3D view is what is under the pen.
    over_view: bool,
    /// The gizmo tool is on.
    gizmo: bool,
}

impl Press {
    /// The machine that owns the press.
    fn owner(&self) -> Machine {
        if !self.over_view {
            return Machine::Interface;
        }
        let matches = |Row(button, keys, tool, _): &&Row| {
            *button == self.button && keys.hold(self.held) && (*tool == Tool::Any || self.gizmo)
        };
        ROWS.iter()
            .find(matches)
            .map_or(Machine::Interface, |r| r.3)
    }
}

impl Studio {
    /// A press of `button` at `s`, as the window is now.
    fn press(&self, s: &PenSample, button: Button) -> Press {
        Press {
            button,
            held: self.input.held,
            over_view: self.shell.over_view(s),
            gizmo: self.shell.gizmo.on,
        }
    }

    /// The machine that owns the tip's press at `s`: classified when the tip touches, kept
    /// until it lifts. `None` while the pen is up. Call [`Studio::tip_lifted`] after the
    /// sample has gone to its machine.
    pub(super) fn tip_owner(&mut self, s: &PenSample) -> Option<Machine> {
        if s.in_proximity && s.contact && self.input.tip.is_none() {
            self.input.tip = Some(self.press(s, Button::Tip).owner());
        }
        self.input.tip
    }

    /// Ends the tip's press if `s` is not touching.
    pub(super) fn tip_lifted(&mut self, s: &PenSample) {
        if !(s.in_proximity && s.contact) {
            self.input.tip = None;
        }
    }

    /// Alt with a barrel click on the 3D view makes the part under it the active one (decision
    /// 0017 as amended): a release of barrel button 1 within egui's click distance of its
    /// press names the frontmost part that shows there in a `part.select` call, so the log
    /// names the part. On empty canvas nothing happens.
    pub(super) fn pick(&mut self, s: &PenSample, reads: &PenReads) {
        let barrel = s.in_proximity && s.buttons & 1 != 0;
        match (self.input.barrel_down, barrel) {
            (None, true) => {
                let picks = self.press(s, Button::Barrel).owner() == Machine::Pick;
                self.input.barrel_down = Some(([s.x, s.y], picks));
            }
            (Some((at, picks)), false) => {
                self.input.barrel_down = None;
                let slop = self.click_slop();
                let [dx, dy] = [s.x - at[0], s.y - at[1]];
                let click = dx * dx + dy * dy <= slop * slop;
                if let Some(part) = (picks && click)
                    .then(|| self.part_under(at, reads))
                    .flatten()
                {
                    let select = with_args("part.select", json!({ "part": part }));
                    self.shell.queue(select);
                }
            }
            _ => {}
        }
    }

    /// How far, in window pixels, the pen may move between press and release for a click.
    pub(super) fn click_slop(&self) -> f64 {
        f64::from(self.ctx.options(|o| o.input_options.max_click_dist) * self.ppp())
    }

    /// The frontmost part that shows under the window pixel `at` of the 3D view, by the ray the
    /// gizmo casts.
    fn part_under(&self, at: [f64; 2], reads: &PenReads) -> Option<String> {
        let (rect, ppp) = (self.shell.viewport(), self.ppp());
        let size = [rect.width() * ppp, rect.height() * ppp];
        let [x, y] = [0, 1].map(|i| at[i] as f32 - rect.min[i] * ppp);
        let (o, d) = self.shell.main.camera.ray(x, y, size);
        (reads.under)(o, d).map(|hit| hit.part)
    }

    /// The pen lifted with a Ctrl press waiting: what its gesture does is settled now, from
    /// the session as it is (`reads`), and runs at once if the press is known to be on empty
    /// canvas, else when the engine says so. With no active part (an empty scene) it runs
    /// nothing and says why.
    pub(super) fn lifted(&mut self, reads: &PenReads) {
        let Some(mut b) = self.input.background.take() else {
            return;
        };
        let ending = match self.shell.parts.iter().find(|p| p.active) {
            None => {
                Err("the scene has no parts: Ctrl on empty canvas works on the active part".into())
            }
            Some(_) if !b.dragged => Ok(with_args("mask.invert", json!({ "scope": "all" }))),
            Some(_) if (reads.masked)() => {
                Ok(with_args("mask.set", json!({ "value": 0, "scope": "all" })))
            }
            // The engine refuses it for a part not in Voxel mode (decision 0002).
            Some(_) => Ok(with_args("mesh.voxel_update", json!({}))),
        };
        if b.canvas == Some(true) {
            self.finish(ending);
        } else {
            b.ending = Some(ending);
            self.input.background = Some(b);
        }
    }

    /// Runs a background gesture's command, or shows why it runs none.
    fn finish(&mut self, ending: Result<Step, String>) {
        match ending {
            Ok(step) => self.shell.queue(step),
            Err(problem) => self.shell.status.says(&problem),
        }
    }

    /// The engine's answer to the Ctrl press it took, from the report of the call that took it
    /// (`on_canvas`). On the model the press is a Mask Pen stroke and the gesture is gone; on
    /// empty canvas the gesture goes on, and runs now if the pen has already lifted. True when
    /// the answer was the gesture's, so it does not navigate.
    pub(super) fn background_answer(&mut self, on_canvas: bool) -> bool {
        let Some(b) = (self.input.background.as_mut()).filter(|b| b.asked && b.canvas.is_none())
        else {
            return false;
        };
        b.canvas = Some(on_canvas);
        if !on_canvas {
            self.input.background = None;
        } else if let Some(ending) = b.ending.take() {
            self.input.background = None;
            self.finish(ending);
        }
        true
    }
}
