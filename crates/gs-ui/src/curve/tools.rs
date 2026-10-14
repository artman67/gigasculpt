//! Under a curve graph: Invert, Copy and Paste (decision 0048, sections 2 and 4), and under the
//! falloff curve its Steps, Step strength and Tile (section 1). Invert, Paste and the sliders run
//! the curve's own command, so each is one logged change like any edit of its points; Copy holds
//! the curve (its points, their kinds and its settings) in the window (`ui.curve`) and changes
//! nothing, as selecting text does not.

use gs_core::curve::Shape;
use gs_protocol::Step;
use serde_json::json;

use super::graph::{Edit, Points};
use crate::pieces::{Now, Press, run_button, running};
use crate::record::{Recorder, with_args};

/// A button that runs `step`, labelled `label`.
fn press(
    ui: &mut egui::Ui,
    label: &str,
    step: Step,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let press = Press {
        label,
        step,
        key: None,
        lit: false,
        shows: None,
    };
    run_button(ui, press, now, rec)
}

/// Invert, Copy and, once a curve is copied, Paste for `e`'s curve, ending their row. Each is
/// labelled with the graph's name, so a panel with several graphs names each apart. Returns
/// what it asked to run.
pub fn tools(ui: &mut egui::Ui, e: &Edit, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let inverted = (e.step)(&e.points.inverted());
    let invert = press(ui, &format!("Invert {}", e.name), inverted, now, rec);
    let hold = with_args("ui.curve", json!({ "copy": e.points.value() }));
    let copy = press(ui, &format!("Copy {}", e.name), hold, now, rec);
    let paste = now.copied.and_then(|c| {
        let step = (e.step)(&Points::of(c));
        press(ui, &format!("Paste {}", e.name), step, now, rec)
    });
    ui.end_row();
    invert.or(copy).or(paste)
}

/// A curve setting's slider: its title, value, ends, whether it is a count (in whole numbers,
/// 1 reading "off"), and how it sets the shape.
type Row = (&'static str, f32, [f32; 2], bool, fn(&mut Shape, f32));

/// Steps, Step strength and Tile of `e`'s curve, a slider each on its own row, each running
/// `e`'s command with the points as they are and that setting changed (decision 0048, section
/// 1). Steps and Tile are counts, 1 (off) to 64. Returns what they asked to run.
pub fn shape(ui: &mut egui::Ui, e: &Edit, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let s = e.points.shape;
    let rows: [Row; 3] = [
        ("Steps", s.steps as f32, [1.0, 64.0], true, |s, v| {
            s.steps = v.round().clamp(1.0, 64.0) as u32;
        }),
        (
            "Step strength",
            s.step_strength,
            [0.0, 1.0],
            false,
            |s, v| {
                s.step_strength = v.clamp(0.0, 1.0);
            },
        ),
        ("Tile", s.tile as f32, [1.0, 64.0], true, |s, v| {
            s.tile = v.round().clamp(1.0, 64.0) as u32;
        }),
    ];
    let mut asked = None;
    for (title, value, ends, whole, set) in rows {
        let step = |v: f32| {
            let mut p = e.points.clone();
            set(&mut p.shape, v);
            (e.step)(&p)
        };
        let row = ("curve", title, value, ends, whole);
        asked = asked.or(running(ui, row, &step, now, rec));
        ui.end_row();
    }
    asked
}
