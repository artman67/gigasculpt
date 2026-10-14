//! Pen pressure curves (catalog `brush-settings.tablet-pressure` and `interface.pen-settings`,
//! decision 0036 sections 3 and 6): curve graphs (`curve`) of how much of a setting a dab takes,
//! from none at the foot to all of it at the top, against the pen's pressure, from none at the
//! left to full at the right. The settings window's Pen set holds the session's size curve,
//! which every brush takes unless it names its own (`brush.set`'s `global_pressure.size`);
//! Basics' Pressure popover holds the brush's own size, strength, spacing, Brush Modifier and
//! depth (imbed) curves (`settings.pressure`, as `brush.describe` reports them; decision 0060,
//! section 6). Each has a switch: on, the curve starts as a straight line, the setting
//! following the pressure. Off, a size curve runs `brush.set` with the curve as `null`, so the
//! session's is off again and the brush's own is its preset's; any other is set flat at full,
//! so pressure leaves it alone. Size's and Strength's switches are also the pen toggles at
//! their sliders' right. On a pressure graph a point dragged near the top lands on it, so full
//! is easy to reach.

use egui::{RichText, Shape, Stroke};
use gs_core::brush::Pressure;
use gs_core::curve::Curve;
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::curve::{Points, Setting, setting};
use crate::descriptions;
use crate::pieces::{Face, Now, Press, hover, label, popover, pressed_as, run_button};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// Which way a pressure graph runs.
pub const LEGEND: &str = "Light at the left, full pressure at the right";

/// Basics' button for the brush's own curves, and its hint (decision 0060, section 11).
const POPOVER: [&str; 2] = [
    "Pressure…",
    "This brush's pressure curves, for size, strength, spacing, modifier and depth. The pen's \
     own curve is in Settings > Pen.",
];

/// The pen toggles' curves, by key and title, each with its hint (decision 0060, section 11).
const PENS: [(&str, &str, &str); 2] = [
    (
        "size",
        "Size",
        "Lit: a lighter press makes the brush smaller. Its curve is under Pressure.",
    ),
    (
        "strength",
        "Strength",
        "Lit: a lighter press does less. Its curve is under Pressure.",
    ),
];

/// Which curve: the session's size curve, for every brush, or one of the pen's brush's own, by
/// its key in `settings.pressure` and its title.
#[derive(Clone, Copy)]
enum Whose {
    Every,
    Own(&'static str, &'static str),
}

impl Whose {
    /// `brush.set` with `curve` as this curve (`null` drops a size curve).
    fn step(self, curve: &Value) -> Step {
        match self {
            Whose::Every => with_args("brush.set", json!({ "global_pressure": { "size": curve } })),
            Whose::Own(key, _) => with_args(
                "brush.set",
                json!({ "settings": { "pressure": { key: curve } } }),
            ),
        }
    }

    /// Its heading, switch, graph and points, as `ui.describe` names them.
    fn labels(self) -> [String; 4] {
        let names = |s: [&str; 4]| s.map(String::from);
        match self {
            Whose::Every => names(["size", "Every brush", "Pressure curve", "Pressure point"]),
            Whose::Own("size", _) => names([
                "size",
                "This brush",
                "Brush pressure curve",
                "Brush pressure point",
            ]),
            Whose::Own(_, title) => [
                title.to_lowercase(),
                format!("{title} by pressure"),
                format!("{title} pressure curve"),
                format!("{title} pressure point"),
            ],
        }
    }

    /// What the switch writes to turn the curve off.
    fn off(self) -> Value {
        match self {
            Whose::Every | Whose::Own("size", _) => Value::Null,
            Whose::Own(..) => json!([[0, 1], [1, 1]]),
        }
    }

    /// Its switch, labelled `label` and lit while the curve is `on`: a press turns it off, or
    /// on as a straight line.
    fn switch(self, label: &str, on: bool) -> Press<'_> {
        let curve = if on {
            self.off()
        } else {
            json!([[0, 0], [1, 1]])
        };
        Press {
            label,
            step: self.step(&curve),
            key: None,
            lit: on,
            shows: None,
        }
    }
}

/// The session's curve, in the settings window's Pen set. Returns what it asked to run.
pub fn global(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let set = now.brush.global_pressure.get("size").cloned();
    let curve = set.and_then(|v| serde_json::from_value::<Curve>(v).ok());
    editor(ui, Whose::Every, curve.as_ref(), now, rec)
}

/// The brush's own curves by key and title, each `None` while it gives the whole setting at
/// every pressure.
fn curves(p: &Pressure) -> [(&'static str, &'static str, Option<&Curve>); 5] {
    fn on(c: &Curve) -> Option<&Curve> {
        (*c != Curve::constant(1.0)).then_some(c)
    }
    [
        ("size", "Size", on(&p.size)),
        ("strength", "Strength", on(&p.strength)),
        ("spacing", "Spacing", on(&p.spacing)),
        ("modifier", "Modifier", on(&p.modifier)),
        ("imbed", "Depth", on(&p.imbed)),
    ]
}

/// Basics' Pressure… (decision 0060, section 6): a button whose popover holds the pen's brush's
/// own curves. Nothing in it until the engine has said which brush it is.
pub fn button(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let [text, hint] = POPOVER;
    let shown = ui.button(text);
    rec.widget(Widget::plain(text, Role::Button), shown.rect);
    let shown = hover(shown, Some(hint), rec);
    let (_, asked) = popover(ui, &shown, rec, |ui, rec| {
        ui.horizontal_wrapped(|ui| {
            let p = &now.preset?.pressure;
            let each =
                curves(p).map(|(key, title, on)| editor(ui, Whose::Own(key, title), on, now, rec));
            each.into_iter().fold(None, Option::or)
        })
        .inner
    });
    asked.flatten()
}

/// Size's or Strength's pen toggle (`key`), at its slider's right (decision 0060, section 6):
/// its curve's switch, a pen painted on it. Nothing until the engine has said which brush it is.
pub fn pen(ui: &mut egui::Ui, key: &str, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let (key, title, hint) = PENS.into_iter().find(|p| p.0 == key)?;
    let on = curves(&now.preset?.pressure)
        .into_iter()
        .any(|c| c.0 == key && c.2.is_some());
    let label = format!("{title} by pen");
    let press = Whose::Own(key, title).switch(&label, on);
    let step = press.step.clone();
    let side = ui.spacing().interact_size.y;
    let shown = pressed_as(ui, press, Face::Square(side), now, rec);
    rec.hint(hint);
    descriptions::hint(ui, &shown, hint);
    paint_pen(ui, shown.rect, on);
    shown.clicked().then_some(step)
}

/// Paints a pen on the toggle `rect`, its tip at the lower left, in the lit button's text color
/// while `lit`: the interface font has no pen sign.
fn paint_pen(ui: &egui::Ui, rect: egui::Rect, lit: bool) {
    let colors = &theme(ui.ctx()).colors;
    let ink = if lit { colors.on_lit } else { colors.text };
    let r = rect.shrink(rect.width() * 0.25);
    let (tip, end) = (r.left_bottom(), r.right_top());
    let along = (end - tip) / 3.0;
    let across = along.rot90() * 0.3;
    let neck = tip + along;
    let painter = ui.painter();
    let point = vec![tip, neck + across, neck - across];
    painter.add(Shape::convex_polygon(point, ink, Stroke::NONE));
    painter.line_segment([neck, end], Stroke::new(across.length() * 2.0, ink));
}

/// On a row of its own, a heading, the switch, and while it is on the graph, which way it runs
/// and the graph's tools.
fn editor(
    ui: &mut egui::Ui,
    whose: Whose,
    curve: Option<&Curve>,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let [what, switch, name, point] = whose.labels();
    ui.end_row();
    let heading = format!("Pressure to {what}");
    label(ui, RichText::new(heading).small().weak(), rec);
    ui.end_row();
    let switched = run_button(ui, whose.switch(&switch, curve.is_some()), now, rec);
    ui.end_row();
    let Some(curve) = curve else {
        return switched;
    };
    let step = |p: &Points| whose.step(&p.value());
    let pressure = Setting {
        names: [&name, &point],
        curve,
        step: &step,
        legend: LEGEND,
        snap: true,
        under: None,
    };
    switched.or(setting(ui, &pressure, now, rec))
}
