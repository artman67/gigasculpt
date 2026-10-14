//! Curve graphs the pen edits ([`graph`]) and their tools (`tools`), a curve setting drawn whole
//! (`setting`), and the brush tray's Tip section with its shape editor (catalog
//! `brush-settings.curve`, decision 0060 section 6). The pressure curves (`pressure`) are graphs
//! too.
//!
//! Tip is a row with the tip's picture and Softness (the brush's Falloff), then Edit shape… and
//! Exact (the tip's accurate curve). The picture and Edit shape… each open the shape editor, a
//! popover beside the tray. The editor, top to bottom (decision 0059 section 4, as amended
//! 2026-10-12): the profile picker, a button for each named profile, lit while it is the
//! brush's, and a pick one `brush.set` of the profile with no points; then the falloff curve, a
//! graph of strength, 0 at the foot to 1 at the top, against the distance from the dab's center,
//! at the left, to its rim, at the right. The bright line is the tip's profile, its points drawn
//! as handles; the dim one is what a stroke uses, the profile after the brush's hardness and
//! Softness (`gs_core::falloff`), so the Softness slider bends it. A change runs `brush.set` with
//! the curve as the tip's custom profile; a named profile starts as points on it. Reset curve
//! drops this session's change to the profile, so the preset's applies again. Under it are
//! Invert, Copy and Paste, the curve's Steps, Step strength and Tile (decision 0048), the tip's
//! Wrap Mode (decision 0054), and under the heading Pressure shape, Curve By Pen with its two
//! curves (`by_pen`). The brush's own pressure curves are in Basics' Pressure popover
//! (`pressure`).

mod by_pen;
mod graph;
mod setting;
mod tools;

use egui::{RichText, Sense, Shape as Line, Stroke, StrokeKind, pos2, vec2};
use gs_core::brush::{ProfileName, TipSpec};
use gs_core::curve::Shape;
use gs_core::falloff::Tip;
use gs_engine::Shows;
use gs_protocol::Step;
use serde_json::json;

pub use graph::Points;
pub use setting::{Setting, setting};

use crate::pieces::{
    Dragging, Now, Press, control, controls_of, hover, label, popover, run_button, running,
};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;
use graph::{Edit, across, graph};
use tools::tools;

/// The tip's picture and what hovering it says (decision 0060, section 11).
const PICTURE: [&str; 2] = [
    "Tip picture",
    "How the effect falls from the center (left) to the edge (right). Tap to edit the shape.",
];
/// The button that opens the shape editor, and what hovering it says.
const EDIT: [&str; 2] = [
    "Edit shape…",
    "The shape editor: profiles, the curve, Invert, Copy, Paste, Steps, Tile, Wrap Mode, and a second shape that pressure blends toward.",
];

/// Where a named profile's points start, from the center to the rim.
const STARTS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
/// Where the rim is read: a tip's falloff is 0 from the rim out, whatever its profile says there.
const RIM: f32 = 0.9999;
/// Which way a falloff graph runs.
const LEGEND: &str = "Center at the left, rim at the right";
/// The state value the profile picker shows.
const PROFILE: &str = "brush.tip.profile";
/// The named profiles and their buttons, from the fullest to the sharpest (by the area under
/// each), so neighbours look alike.
const PROFILES: [(ProfileName, &str); 9] = [
    (ProfileName::Constant, "Constant"),
    (ProfileName::Hard, "Hard"),
    (ProfileName::Sphere, "Sphere"),
    (ProfileName::FlatTop, "Flat top"),
    (ProfileName::Smooth, "Smooth"),
    (ProfileName::Soft, "Soft"),
    (ProfileName::Linear, "Linear"),
    (ProfileName::Sharp, "Sharp"),
    (ProfileName::SharpCenter, "Sharp center"),
];

/// The tip's own curve, or smooth points on its named profile.
fn of_tip(tip: &TipSpec) -> Points {
    if let (ProfileName::Custom, Some(c)) = (tip.profile, &tip.points) {
        return Points::of(c);
    }
    let plain = profile(tip);
    Points {
        at: STARTS.iter().map(|&d| [d, inside(&plain, d)]).collect(),
        smooth: vec![true; STARTS.len()],
        shape: Shape::default(),
    }
}

/// `tip` with `points` as its custom profile.
fn with_points(tip: &TipSpec, points: &Points) -> TipSpec {
    TipSpec {
        profile: ProfileName::Custom,
        points: points.curve(),
        ..tip.clone()
    }
}

/// The tip's profile alone, without its hardness and Falloff.
fn profile(tip: &TipSpec) -> Tip {
    let plain = TipSpec {
        hardness: 0.0,
        focal: 0.0,
        alpha: None,
        ..tip.clone()
    };
    plain.tip()
}

/// `tip`'s strength at `d`, the rim read from just inside it.
fn inside(tip: &Tip, d: f32) -> f32 {
    tip.falloff(d.min(RIM))
}

/// Brush tray > Tip: the tip's picture and Softness on a row, then Edit shape… and Exact; the
/// picture and Edit shape… each open the shape editor ([`editor`]) beside the tray. Returns what
/// it asked to run.
pub fn section(
    ui: &mut egui::Ui,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    let picture = picture(ui, now.preset.map(|p| &p.tip), rec);
    // Softness takes the rest of the row, its rail giving up the picture's width so its value
    // stays beside it.
    let width = picture.rect.width() + ui.spacing().item_spacing.x;
    let room = vec2(
        ui.available_size_before_wrap().x,
        ui.spacing().interact_size.y,
    );
    let column = egui::Layout::top_down(egui::Align::Min);
    let soft = ui.allocate_ui_with_layout(room, column, |ui| {
        ui.spacing_mut().slider_width -= width;
        crate::pieces::section(ui, "falloff", now, dragging, rec)
    });
    ui.end_row();
    let edit = ui.button(EDIT[0]);
    rec.widget(Widget::plain(EDIT[0], Role::Button), edit.rect);
    let edit = hover(edit, Some(EDIT[1]), rec);
    let exact = switch(ui, "brush.tip.accurate_curve", now, rec);
    ui.end_row();
    let shaped = |ui: &mut egui::Ui, rec: &mut Recorder| {
        ui.horizontal_wrapped(|ui| editor(ui, now, rec)).inner
    };
    let (_, pictured) = popover(ui, &picture, rec, shaped);
    let (_, edited) = popover(ui, &edit, rec, shaped);
    let opened = pictured.flatten().or(edited.flatten());
    soft.inner.or(exact).or(opened)
}

/// The tip's picture, a button: its profile as a stroke uses it, after the brush's hardness and
/// Softness, from the center at the left to the rim at the right, as the editor's graph draws it.
fn picture(ui: &mut egui::Ui, tip: Option<&TipSpec>, rec: &mut Recorder) -> egui::Response {
    let side = ui.spacing().interact_size.y;
    let (rect, shown) = ui.allocate_exact_size(vec2(2.0 * side, side), Sense::click());
    rec.widget(Widget::plain(PICTURE[0], Role::Button), rect);
    let shown = hover(shown, Some(PICTURE[1]), rec);
    let t = theme(ui.ctx());
    let c = &t.colors;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.spacing.radius, c.control);
    let edge = if shown.hovered() { c.lit } else { c.edge };
    painter.rect_stroke(
        rect,
        t.spacing.radius,
        Stroke::new(1.0, edge),
        StrokeKind::Inside,
    );
    if let Some(tip) = tip {
        let (used, inner) = (tip.tip(), rect.shrink(3.0));
        let at = across(|d| inside(&used, d)).into_iter().map(|[x, y]| {
            pos2(
                inner.left() + x * inner.width(),
                inner.bottom() - y.clamp(0.0, 1.0) * inner.height(),
            )
        });
        painter.add(Line::line(at.collect(), Stroke::new(1.5, c.lit)));
    }
    shown
}

/// Draws the shape editor: the profile picker, the falloff curve of the pen's brush, a line
/// saying which way the graph runs, Reset curve, the curve's tools and settings, Wrap Mode, and
/// under the heading Pressure shape, Curve By Pen. Returns what it asked to run. Nothing until
/// the engine has said which brush it is.
pub fn editor(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let tip = &now.preset?.tip;
    let picked = picker(ui, now, rec);
    let start = of_tip(tip);
    let step = |p: &Points| {
        let tip = json!({ "profile": "custom", "points": p.value() });
        with_args("brush.set", json!({ "settings": { "tip": tip } }))
    };
    // Points the pen has not moved draw the tip as it is, a named profile exactly.
    let lines = |p: &Points| {
        let shown = if *p == start {
            tip.clone()
        } else {
            with_points(tip, p)
        };
        let (used, plain) = (shown.tip(), profile(&shown));
        let used = across(|d| inside(&used, d));
        vec![used, across(|d| inside(&plain, d))]
    };
    let edit = Edit {
        name: "Falloff curve",
        point: "Curve point",
        points: start.clone(),
        step: &step,
        lines: &lines,
        snap: false,
    };
    let asked = graph(ui, &edit, rec);
    label(ui, RichText::new(LEGEND).small().weak(), rec);
    ui.end_row();
    let reset = Press {
        label: "Reset curve",
        step: with_args(
            "brush.set",
            json!({ "settings": { "tip": { "profile": null, "points": null } } }),
        ),
        key: None,
        lit: false,
        shows: None,
    };
    let reset = run_button(ui, reset, now, rec);
    ui.end_row();
    let tools = tools(ui, &edit, now, rec);
    let shape = tools::shape(ui, &edit, now, rec);
    let wrap = wrap(ui, tip, now, rec);
    label(ui, RichText::new("Pressure shape").small().weak(), rec);
    ui.end_row();
    let by_pen = by_pen::by_pen(ui, tip, now, rec);
    let asked = picked.or(reset).or(asked).or(tools).or(shape);
    asked.or(wrap).or(by_pen)
}

/// The profile picker: a heading, "Profile", or "Profile: custom curve" while the tip is one,
/// then a button for each named profile, lit while `brush.tip.profile` is its name, each running
/// one `brush.set` of the profile with no points, which only a custom profile may hold (decision
/// 0059 section 4, as amended 2026-10-12). Returns what it asked to run.
fn picker(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let shown = now.state.get(PROFILE);
    let custom = shown == Some(&json!(ProfileName::Custom));
    let heading = if custom {
        "Profile: custom curve"
    } else {
        "Profile"
    };
    label(ui, RichText::new(heading).small().weak(), rec);
    ui.end_row();
    let mut asked = None;
    for (name, title) in PROFILES {
        let on = json!(name);
        let tip = json!({ "profile": on, "points": null });
        let press = Press {
            label: title,
            step: with_args("brush.set", json!({ "settings": { "tip": tip } })),
            key: None,
            lit: shown == Some(&on),
            shows: Some(Shows {
                value: PROFILE.into(),
                on,
            }),
        };
        asked = run_button(ui, press, now, rec).or(asked);
    }
    ui.end_row();
    asked
}

/// The `curve` set's switch that shows the state value `value`, as the controls file lists it;
/// nothing where it does not. Returns what it asked to run.
fn switch(ui: &mut egui::Ui, value: &str, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let all = controls_of("curve", now);
    let c = all.iter().find(|c| c.shows.as_deref() == Some(value))?;
    control(ui, c, false, now, rec)
}

/// Wrap Mode's slider, ending its row: how many tiles along each side of the part's plane a
/// stroke repeats in, 1 (off) to 16, the tip's `wrap`, set with `brush.set` once a value
/// (decision 0054). Returns what it asked to run.
fn wrap(ui: &mut egui::Ui, tip: &TipSpec, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let step = |v: f32| {
        let wrap = v.round().clamp(1.0, 16.0) as u32;
        with_args(
            "brush.set",
            json!({ "settings": { "tip": { "wrap": wrap } } }),
        )
    };
    let row = ("curve", "Wrap Mode", tip.wrap as f32, [1.0, 16.0], true);
    let asked = running(ui, row, &step, now, rec);
    ui.end_row();
    asked
}
