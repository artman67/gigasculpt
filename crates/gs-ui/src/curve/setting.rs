//! A curve setting drawn whole, as the Depth Curve, Curve By Pen's Zero and Pen Curves and the
//! pressure curves are: its graph ([`graph`]), a line under it saying which way it runs, then
//! Invert, Copy and Paste ([`tools`]).

use egui::RichText;
use gs_core::curve::Curve;
use gs_protocol::Step;

use super::graph::{Edit, Points, across, graph};
use super::tools::tools;
use crate::pieces::{Now, label};
use crate::record::Recorder;

/// Makes the dim line under a curve from the curve as the pen holds it.
type Under<'a> = &'a dyn Fn(&Curve) -> Vec<[f32; 2]>;

/// A curve setting.
pub struct Setting<'a> {
    /// What `ui.describe` calls the graph and each of its points.
    pub names: [&'a str; 2],
    /// The curve as the brush holds it.
    pub curve: &'a Curve,
    /// The call that sets the curve to some points.
    pub step: &'a dyn Fn(&Points) -> Step,
    /// Which way the graph runs.
    pub legend: &'a str,
    /// A pressure graph: a point dragged near the top lands on it.
    pub snap: bool,
    /// A line drawn dim under the curve, from the curve as the pen holds it; `None` for none.
    pub under: Option<Under<'a>>,
}

/// Draws `s`, each part ending its row. Returns what it asked to run.
pub fn setting(ui: &mut egui::Ui, s: &Setting, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let lines = |p: &Points| {
        let shown = p.curve().unwrap_or_else(|| s.curve.clone());
        let under = s.under.map(|under| under(&shown));
        under
            .into_iter()
            .chain([across(|x| shown.eval(x))])
            .collect()
    };
    let [name, point] = s.names;
    let edit = Edit {
        name,
        point,
        points: Points::of(s.curve),
        step: s.step,
        lines: &lines,
        snap: s.snap,
    };
    let asked = graph(ui, &edit, rec);
    label(ui, RichText::new(s.legend).small().weak(), rec);
    ui.end_row();
    tools(ui, &edit, now, rec).or(asked)
}
