//! What the window keeps of its input between frames: the egui events waiting for the next
//! frame, the modifiers held, and where the pen is in a stroke or a gesture.

use gs_engine::PenStroke;
use gs_protocol::Step;

use super::gesture::Machine;
use crate::glue::PenRouter;

/// The window's input state between frames.
#[derive(Default)]
pub(super) struct InputState {
    /// Input for the next frame.
    pub(super) pending: Vec<egui::Event>,
    /// Turns pen samples into egui pointer events.
    pub(super) router: PenRouter,
    /// The modifiers held now.
    pub(super) held: egui::Modifiers,
    /// The machine that owns the tip's press, from the touch to the lift; `None` while the pen
    /// is up.
    pub(super) tip: Option<Machine>,
    /// The stroke in progress's straight line, while Shift held after the press may lock it.
    pub(super) line: Option<super::pen::Line>,
    /// Alt as the stroke in progress last sent it: held at the press, or pressed or released
    /// since.
    pub(super) alt: bool,
    /// Space as the stroke in progress last sent it, while its stroke has something Space
    /// moves (`StrokeSpec::moves`); `None` otherwise.
    pub(super) space: Option<bool>,
    /// The pen touched down off the 3D view (on a panel or a drag button) and has not lifted,
    /// so sliding onto the view does not start a stroke.
    pub(super) pressed_off: bool,
    /// Where barrel button 1 went down, in pixels, and whether that press may pick a part
    /// (Alt held, over the 3D view); `None` while it is up.
    pub(super) barrel_down: Option<([f64; 2], bool)>,
    /// What the pen did to the 3D view since the engine last asked.
    pub(super) strokes: Vec<PenStroke>,
    /// A Ctrl press on the 3D view, from the press until the engine says it fell on the model
    /// or, on empty canvas, until its gesture has run.
    pub(super) background: Option<Background>,
}

/// A press with Ctrl alone held (`held::background`). On the model it is a Mask Pen stroke; on
/// empty canvas it is a gesture on the mask, run when the pen lifts (decision 0017 as amended,
/// item 10).
#[derive(Default)]
pub(super) struct Background {
    /// Where the press was, in window pixels.
    pub(super) at: [f64; 2],
    /// The pen has moved farther than a click from `at`.
    pub(super) dragged: bool,
    /// The engine has taken the press, so the report of the call that took it says where it
    /// fell.
    pub(super) asked: bool,
    /// Whether it fell on empty canvas, once the engine has said.
    pub(super) canvas: Option<bool>,
    /// What the gesture does, once the pen has lifted: the command it runs, or why it runs
    /// none.
    pub(super) ending: Option<Result<Step, String>>,
}
