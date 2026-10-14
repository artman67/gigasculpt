//! Curve By Pen in Brush tray > Tip's Edit shape popover (decision 0059, sections 2 and 4, as amended 2026-10-12):
//! its switch, and while it is on the Zero Curve, the falloff at no pressure, and the Pen Curve,
//! how much of the profile each pressure takes, each under a heading as a curve setting
//! ([`setting`]) whose edits are one `brush.set` of `tip.zero_curve` or `tip.pen_curve`. Under
//! the Zero Curve a dim line draws what a stroke reads at no pressure, after the brush's hardness
//! and Falloff, as gs-core's `Tip` reads it; a point dragged near the Pen Curve's top lands on
//! it, as on every pressure graph.

use egui::RichText;
use gs_core::brush::TipSpec;
use gs_core::curve::Curve;
use gs_protocol::Step;
use serde_json::json;

use super::graph::{Points, across};
use super::setting::{Setting, setting};
use super::{LEGEND, RIM, switch};
use crate::pieces::{Now, label};
use crate::pressure;
use crate::record::{Recorder, with_args};

/// The state value the switch shows.
const BY_PEN: &str = "brush.tip.curve_by_pen";

/// The switch, then while it is on the two curves. Returns what it asked to run.
pub(super) fn by_pen(
    ui: &mut egui::Ui,
    tip: &TipSpec,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let switched = switch(ui, BY_PEN, now, rec);
    ui.end_row();
    if now.state.get(BY_PEN) != Some(&json!(true)) {
        return switched;
    }
    let zero = |shown: &Curve| {
        let tip = TipSpec {
            curve_by_pen: true,
            zero_curve: shown.clone(),
            ..tip.clone()
        };
        let tip = tip.tip();
        across(|d| tip.falloff_at(d.min(RIM), 0.0))
    };
    let curves = [
        (
            "Zero Curve: the profile at no pressure",
            Setting {
                names: ["Zero Curve", "Zero Curve point"],
                curve: &tip.zero_curve,
                step: &|p: &Points| set("zero_curve", p),
                legend: LEGEND,
                snap: false,
                under: Some(&zero),
            },
        ),
        (
            "Pen Curve: how much of the profile each pressure takes",
            Setting {
                names: ["Pen Curve", "Pen Curve point"],
                curve: &tip.pen_curve,
                step: &|p: &Points| set("pen_curve", p),
                legend: pressure::LEGEND,
                snap: true,
                under: None,
            },
        ),
    ];
    let mut asked = switched;
    for (heading, curve) in curves {
        label(ui, RichText::new(heading).small().weak(), rec);
        ui.end_row();
        asked = setting(ui, &curve, now, rec).or(asked);
    }
    asked
}

/// `brush.set` with `points` as the tip's curve `key`.
fn set(key: &str, points: &Points) -> Step {
    let tip = json!({ key: points.value() });
    with_args("brush.set", json!({ "settings": { "tip": tip } }))
}
