//! The gizmo's pose buttons in the Brush tray while the tool is on: Pose's (`pose`) and the
//! operators in the tool's ⋯ (decision 0060, section 6).

use gs_protocol::Step;
use serde_json::json;

use crate::pieces::{Face, Now, Press as Button, hover, pressed_as};
use crate::record::{Recorder, with_args};

/// The set the Gizmo tool's ⋯ lists after [`menu`]'s operators.
pub const MENU: &str = "gizmo-menu";

/// Pose: the gizmo's Lock, Sticky and Follow mask, for the part whose gizmo shows, while the tool
/// is on: each a `gizmo.place` that turns it over, lit while it is on in the pose the window
/// draws. Then the deformers (`super::deformer::section`).
pub fn pose_buttons(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let (part, pose) = now.gizmo?;
    let toggles = [
        ("Lock", "locked", pose.locked, LOCK),
        ("Sticky", "sticky", pose.sticky, STICKY),
        ("Follow mask", "follow", pose.follow, FOLLOW),
    ];
    let toggles = toggles.map(|(label, key, on, hint)| {
        let step = with_args("gizmo.place", json!({ "part": part, key: !on }));
        (label, step, on, hint)
    });
    let asked = buttons(ui, toggles, now, rec);
    super::deformer::section(ui, part, now, rec).or(asked)
}

/// The tool's ⋯'s operators: Mesh to axis and Reset orientation, which move or turn the part
/// with its gizmo, or with Alt held only the gizmo (`gizmo.mesh-to-axis`,
/// `gizmo.reset-orientation`).
pub fn menu(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let (part, _) = now.gizmo?;
    // With Alt the gizmo-only forms: the part stays.
    let only = json!({ "part": part });
    let (to_axis, reset) = if ui.input(|i| i.modifiers.alt) {
        let origin = json!({ "part": part, "rule": "world_origin" });
        (
            with_args("gizmo.place", origin),
            with_args("gizmo.reset", only),
        )
    } else {
        let reset = with_args("gizmo.reset_orientation", only.clone());
        (with_args("gizmo.mesh_to_axis", only), reset)
    };
    let operators = [
        ("Mesh to axis", to_axis, false, TO_AXIS),
        ("Reset orientation", reset, false, RESET),
    ];
    buttons(ui, operators, now, rec)
}

/// Each of `all`, a label, what it runs, whether it is lit and its hint, as a button. Returns
/// what was pressed.
fn buttons<const N: usize>(
    ui: &mut egui::Ui,
    all: [(&str, Step, bool, &str); N],
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut asked = None;
    for (label, step, lit, hint) in all {
        let press = Button {
            label,
            step: step.clone(),
            key: None,
            lit,
            shows: None,
        };
        let shown = hover(
            pressed_as(ui, press, Face::Label, now, rec),
            Some(hint),
            rec,
        );
        asked = shown.clicked().then_some(step).or(asked);
    }
    asked
}

const LOCK: &str = "On, a drag moves the part with the gizmo; off, it moves only the gizmo.";
const STICKY: &str = "The gizmo stays where it was while a drag moves the part.";
const FOLLOW: &str =
    "The gizmo goes to the middle of the unmasked points, now and whenever the mask changes.";
const TO_AXIS: &str =
    "Moves the part so the gizmo sits on the scene's origin. With Alt, only the gizmo moves.";
const RESET: &str =
    "Turns the part so the gizmo's axes are the scene's. With Alt, only the gizmo turns.";
