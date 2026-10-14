//! The Gizmo's deformers (decision 0041, amended 2026-10-12, step `DF7b`), at the end of its set
//! in Brush tray > Brush: a picker that opens one on the part whose gizmo shows, at no change;
//! the open deformer's settings, a row for each argument of its first call, each change one
//! `update`; and Accept, Cancel and Reset, which the quick menu shows too ([`quick_buttons`]).
//! Enter and Escape accept and cancel it, and the window lets go of it when the pen lifts
//! ([`keys`]); a save the engine refuses while the pen is down waits for the lift ([`held_back`],
//! [`lifted`]) and the status line says so ([`waits`]), all in `lift`. Every call's frame has the
//! gizmo's axes and the engine measures its box (amended for `DF7c`, section 1); its session
//! calls are the spring slider's (`pieces/spring.rs`), and its handles in the view are
//! `deforming`'s.

mod lift;

use egui::{Id, RichText};
use gs_core::gizmo::{DeformerHandle, Drive, MOVE_SNAP, Unit as Snaps, seen};
use gs_core::transform::Frame;
use gs_core::{Quat, Vec3};
use gs_protocol::Step;
use serde_json::{Value, json};

pub use lift::{held_back, keys, lifted, waits};

use super::drag::symmetry;
use crate::pieces::deform::{AXES, Shape, num, pressed, shape};
use crate::pieces::spring::{phase, session};
use crate::pieces::{Now, hover, label, running};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

const DIAGONAL: &str = "gizmo deformer diagonal";

/// A button of the picker: its label, its hint, and the operation it opens on a part at no
/// change.
type Pick = (&'static str, &'static str, fn(&Shape) -> Value);

/// The picker, in its order (section 2): Header > Deform's eleven, then the Gizmo's own.
const PICKS: [Pick; 15] = [
    (
        "Offset",
        "Opens a deformer that moves the part",
        |_| json!({ "move": [0.0, 0.0, 0.0] }),
    ),
    (
        "Rotate",
        "Opens a deformer that turns the part about its middle",
        |_| json!({ "rotate": { "axis": "x", "degrees": 0.0 } }),
    ),
    (
        "Stretch",
        "Opens a deformer that scales the part along its own axes",
        |_| json!({ "scale": [1.0, 1.0, 1.0] }),
    ),
    (
        "Fold bend",
        "Opens a deformer that folds the part at its middle",
        |_| json!({ "fold": { "axis": "x", "toward": "y", "degrees": 0.0 } }),
    ),
    (
        "Smooth bend",
        "Opens a deformer that bends the part into an arc",
        |_| json!({ "bend": { "axis": "x", "toward": "y", "degrees": 0.0 } }),
    ),
    (
        "Twist",
        "Opens a deformer that twists the part about an axis",
        |_| json!({ "twist": { "axis": "x", "degrees": 0.0 } }),
    ),
    (
        "Taper",
        "Opens a deformer that narrows the part toward its high end",
        |_| json!({ "taper": { "axis": "x", "amount": 0.0, "exponent": 1.0 } }),
    ),
    (
        "Flatten",
        "Opens a deformer that flattens one side up to a plane",
        |_| json!({ "clip": { "axis": "x", "depth": 0.0 } }),
    ),
    (
        "Spherize",
        "Opens a deformer that rounds the part toward a sphere",
        |s| {
            let radius = s.half().iter().sum::<f32>() / 3.0;
            json!({ "spherize": { "amount": 0.0, "radius": num(radius) } })
        },
    ),
    (
        "Inflate",
        "Opens a deformer that swells the part along its normals",
        |_| json!({ "inflate": 0.0 }),
    ),
    (
        "Noise",
        "Opens a deformer that roughens the part with smooth noise",
        |s| {
            let scale = num(s.diagonal() / 20.0);
            json!({ "noise": { "amount": 0.0, "scale": scale, "seed": 0 } })
        },
    ),
    (
        "Skew",
        "Opens a deformer that slides the part's layers sideways",
        |_| json!({ "shear": { "axis": "x", "toward": "y", "amount": 0.0, "profile": "linear" } }),
    ),
    (
        "Soft cage",
        "Opens a cage of points around the part that bends it smoothly",
        |_| json!({ "lattice": { "kind": "soft", "size": [3, 3, 3] } }),
    ),
    (
        "Hard cage",
        "Opens a cage of points around the part, each cell bent straight",
        |_| json!({ "lattice": { "kind": "hard", "size": [3, 3, 3] } }),
    ),
    (
        "Curve",
        "Opens a deformer that lays the part along a curve",
        // Along the part's longest side, which `opened` sets.
        |_| json!({ "curve": { "axis": "x" } }),
    ),
];

/// How a settings row's slider reads and writes its number.
#[derive(Clone, Copy, PartialEq)]
enum Unit {
    Plain,
    /// A length: its ends are times the box's diagonal.
    Long,
    /// A whole number (the noise's seed).
    Whole,
    /// A whole count of points, shown as one.
    Count,
    /// The last of a list of one a point, from this at the first point evenly to it (a curve's
    /// twist and scale).
    Ramp(f32),
}

/// A row of the open deformer's settings, each at a JSON pointer into its first call's `op`.
enum Row {
    /// A slider: its title, where, its ends, its unit and its hint.
    Number(&'static str, &'static str, [f32; 2], Unit, &'static str),
    /// A button per value, each `(value, face)`: its title, where, the values and its hint.
    /// Toward leaves out the axis.
    Choice(&'static str, &'static str, Choices, &'static str),
    /// How the dots in the view move the points (step `DF7c`), and how many moved; its hint.
    Points(&'static str),
}

type Choices = &'static [(&'static str, &'static str)];

const XYZ: Choices = &[("x", "X"), ("y", "Y"), ("z", "Z")];
const AXIS: &str = "The part's own axis it works along";
const TOWARD: &str = "The axis it bends toward";
const ANGLE: &str = "The angle, in degrees";
const MOVE: &str = "How far the part moves along its own axis, in scene units";
const SIZE: &str = "The scale along the part's own axis; 1 keeps its size";
const CAGE: &str = "The cage's points along the part's own axis";
const POINTS: &str = "The dots in the view move them";
const DOTS: &str =
    "Drag a dot; tap to select, Shift adds, Ctrl removes; drag on the model to select several";
const AMOUNT: &str = "How far it goes; below 0 the other way";

/// Each operation's rows, in order (section 3).
const ROWS: [(&str, &[Row]); 14] = {
    use Row::{Choice, Number, Points};
    use Unit::{Count, Long, Plain, Ramp, Whole};
    [
        (
            "move",
            &[
                Number("X", "/move/0", [-1.0, 1.0], Long, MOVE),
                Number("Y", "/move/1", [-1.0, 1.0], Long, MOVE),
                Number("Z", "/move/2", [-1.0, 1.0], Long, MOVE),
            ],
        ),
        (
            "rotate",
            &[
                Choice("Axis", "/rotate/axis", XYZ, AXIS),
                Number("Angle", "/rotate/degrees", [-180.0, 180.0], Plain, ANGLE),
            ],
        ),
        (
            "scale",
            &[
                Number("X", "/scale/0", [0.0, 3.0], Plain, SIZE),
                Number("Y", "/scale/1", [0.0, 3.0], Plain, SIZE),
                Number("Z", "/scale/2", [0.0, 3.0], Plain, SIZE),
            ],
        ),
        (
            "fold",
            &[
                Choice("Axis", "/fold/axis", XYZ, AXIS),
                Choice("Toward", "/fold/toward", XYZ, TOWARD),
                Number("Angle", "/fold/degrees", [-180.0, 180.0], Plain, ANGLE),
            ],
        ),
        (
            "bend",
            &[
                Choice("Axis", "/bend/axis", XYZ, AXIS),
                Choice("Toward", "/bend/toward", XYZ, TOWARD),
                Number("Angle", "/bend/degrees", [-360.0, 360.0], Plain, ANGLE),
            ],
        ),
        (
            "twist",
            &[
                Choice("Axis", "/twist/axis", XYZ, AXIS),
                Number("Angle", "/twist/degrees", [-360.0, 360.0], Plain, ANGLE),
            ],
        ),
        (
            "taper",
            &[
                Choice(
                    "Axis",
                    "/taper/axis",
                    XYZ,
                    "The axis it narrows along; a new taper",
                ),
                Number("Amount", "/taper/amount", [-1.0, 1.0], Plain, AMOUNT),
                Number(
                    "Exponent",
                    "/taper/exponent",
                    [0.5, 4.0],
                    Plain,
                    "1 narrows evenly",
                ),
            ],
        ),
        (
            "clip",
            &[
                Choice("Axis", "/clip/axis", XYZ, AXIS),
                Number(
                    "Depth",
                    "/clip/depth",
                    [-0.5, 0.5],
                    Long,
                    "Where the plane is, in scene units",
                ),
            ],
        ),
        (
            "spherize",
            &[
                Number("Amount", "/spherize/amount", [-1.0, 1.0], Plain, AMOUNT),
                Number(
                    "Radius",
                    "/spherize/radius",
                    [0.0, 0.5],
                    Long,
                    "The sphere's radius",
                ),
            ],
        ),
        (
            "inflate",
            &[Number("Amount", "/inflate", [-0.25, 0.25], Long, AMOUNT)],
        ),
        (
            "noise",
            &[
                Number(
                    "Amount",
                    "/noise/amount",
                    [0.0, 0.05],
                    Long,
                    "How far the noise moves it",
                ),
                Number(
                    "Scale",
                    "/noise/scale",
                    [0.01, 0.25],
                    Long,
                    "The size of its bumps",
                ),
                Number(
                    "Seed",
                    "/noise/seed",
                    [0.0, 100.0],
                    Whole,
                    "Another seed, other noise",
                ),
            ],
        ),
        (
            "shear",
            &[
                Choice("Axis", "/shear/axis", XYZ, AXIS),
                Choice("Toward", "/shear/toward", XYZ, "The axis it slides along"),
                Number("Amount", "/shear/amount", [-1.0, 1.0], Long, AMOUNT),
                Choice(
                    "Profile",
                    "/shear/profile",
                    &[("linear", "Linear"), ("smooth", "Smooth")],
                    "Linear slides evenly; Smooth eases in and out",
                ),
            ],
        ),
        (
            "lattice",
            &[
                Choice(
                    "Kind",
                    "/lattice/kind",
                    &[("soft", "Soft"), ("hard", "Hard")],
                    "Soft bends the cells smoothly; Hard keeps each straight",
                ),
                Number("X", "/lattice/size/0", [2.0, 8.0], Count, CAGE),
                Number("Y", "/lattice/size/1", [2.0, 8.0], Count, CAGE),
                Number("Z", "/lattice/size/2", [2.0, 8.0], Count, CAGE),
                Points(POINTS),
            ],
        ),
        (
            "curve",
            &[
                Choice(
                    "Axis",
                    "/curve/axis",
                    XYZ,
                    "The part's own axis that follows the curve",
                ),
                Number(
                    "End twist",
                    "/curve/twist",
                    [-360.0, 360.0],
                    Ramp(0.0),
                    "The turn at the curve's end, in degrees, from none at its start",
                ),
                Number(
                    "End scale",
                    "/curve/scale",
                    [0.0, 3.0],
                    Ramp(1.0),
                    "The size at the curve's end, from 1 at its start",
                ),
                Points(POINTS),
            ],
        ),
    ]
};

/// Accept, Cancel and Reset: each label, phase, key and hint (section 4).
const THREE: [(&str, &str, Option<&str>, &str); 3] = [
    (
        "Accept",
        "accept",
        Some("Enter"),
        "Keeps the deformer's change as one undo step",
    ),
    (
        "Cancel",
        "cancel",
        Some("Escape"),
        "Puts the part back as it was before the deformer opened",
    ),
    (
        "Reset",
        "reset",
        None,
        "Puts every setting back to no change; the deformer stays open",
    ),
];

/// Draws the deformers for `part`, on rows of their own; inside the quick menu without Accept,
/// Cancel and Reset, which it shows at its head. Returns what they asked to run.
pub(super) fn section(
    ui: &mut egui::Ui,
    part: &str,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    ui.end_row();
    label(ui, RichText::new("Deformers").small().weak(), rec);
    ui.end_row();
    let calls = (now.state.get("deformer.transforms"))
        .and_then(Value::as_array)
        .filter(|calls| !calls.is_empty());
    let mut asked = if let Some(s) = shape(ui.ctx(), part) {
        picker(ui, &s, calls.map(|c| &c[0]["op"]), now, rec)
    } else {
        label(ui, RichText::new("No part to deform").weak(), rec);
        None
    };
    ui.end_row();
    if let Some(calls) = calls {
        asked = settings(ui, calls, now, rec).or(asked);
    }
    if crate::quick::inside(ui) {
        return asked;
    }
    asked.or(quick_buttons(ui, now, rec))
}

/// Accept, Cancel and Reset while a deformer is open, on a row of their own; the quick menu
/// shows them at its head. Returns what they asked to run.
pub fn quick_buttons(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    now.state.get("deformer").filter(|d| !d.is_null())?;
    let shown = ui.horizontal_wrapped(|ui| {
        let mut asked = None;
        for (label, name, key, hint) in THREE {
            asked = pressed(ui, (label, phase(name), key, false, hint), now, rec).or(asked);
        }
        asked
    });
    shown.inner
}

/// A button for each of [`PICKS`], lit on the open deformer's operation (`open`).
fn picker(
    ui: &mut egui::Ui,
    s: &Shape,
    open: Option<&Value>,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut asked = None;
    for (label, hint, op) in PICKS {
        let op = op(s);
        let lit = open
            .is_some_and(|o| kind(o) == kind(&op) && o["lattice"]["kind"] == op["lattice"]["kind"]);
        asked = pressed(ui, (label, opened(s, op, now), None, lit, hint), now, rec).or(asked);
    }
    asked
}

/// The open of `op` on the part at no change (section 1 as amended for `DF7c`): with the gizmo's
/// axes, about the middle of the part's box seen in them, or for a taper the middle of that
/// box's low face on its axis, so the low end keeps its size; a curve along that box's longest
/// side. The engine measures the box in that frame, and the brush's Mirror and Parallel planes
/// go with it.
fn opened(s: &Shape, mut op: Value, now: &Now) -> Step {
    let axes = now.gizmo.map_or(Quat::IDENTITY, |(_, pose)| pose.axes);
    let (part, bounds, placed) = s.own();
    let frame = Frame {
        origin: Vec3::ZERO,
        axes,
    };
    let b = seen(bounds, &placed, &frame);
    let (lo, hi) = (b.lo.to_array(), b.hi.to_array());
    let mut at: [f32; 3] = std::array::from_fn(|a| f32::midpoint(lo[a], hi[a]));
    let taper = op["taper"]["axis"].as_str();
    if let Some(a) = taper.and_then(|a| AXES.iter().position(|x| *x == a)) {
        at[a] = lo[a];
    }
    if op.get("curve").is_some() {
        let longest = (0..3).fold(0, |a, k| if hi[k] - lo[k] > hi[a] - lo[a] { k } else { a });
        op["curve"]["axis"] = json!(AXES[longest]);
    }
    let origin = axes.rotate(Vec3::from_array(at)).to_array().map(num);
    let frame = json!({ "origin": origin, "axes": axes.to_array().map(num) });
    let mut call = json!({ "part": part, "frame": frame, "op": op });
    if let Some(symmetry) = symmetry(now.brush) {
        call["symmetry"] = symmetry;
    }
    session("open", vec![with_args("transform.apply", call)])
}

/// The operation's name: its one key.
fn kind(op: &Value) -> &str {
    op.as_object()
        .and_then(|o| o.keys().next())
        .map_or("", String::as_str)
}

/// The open deformer's rows, read from its first call (`calls[0]`). Returns what they asked
/// to run.
fn settings(ui: &mut egui::Ui, calls: &[Value], now: &Now, rec: &mut Recorder) -> Option<Step> {
    let kind = kind(&calls[0]["op"]);
    let rows = ROWS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(&[][..], |(_, rows)| *rows);
    let part = now.state.get("deformer.part").and_then(Value::as_str);
    let s = part.and_then(|p| shape(ui.ctx(), p));
    let d = s
        .as_ref()
        .map_or(1.0, |s| diagonal(ui, &calls[0]["frame"], s));
    let mut asked = None;
    for row in rows {
        let step = match *row {
            Row::Number(title, at, ends, unit, hint) => {
                number(ui, (title, at, ends, unit, hint), (calls, d), now, rec)
            }
            Row::Choice(title, at, values, hint) => {
                choice(ui, (title, at, values, hint), (calls, s.as_ref(), now), rec)
            }
            Row::Points(hint) => {
                dots(ui, &calls[0]["op"], hint, rec);
                None
            }
        };
        asked = step.or(asked);
        ui.end_row();
    }
    asked
}

/// The open deformer's box diagonal as it was at the open, kept with its first call's frame,
/// which no update moves: the rows' ranges stay put while the part changes under them.
fn diagonal(ui: &egui::Ui, frame: &Value, s: &Shape) -> f32 {
    let frame = frame.to_string();
    ui.data_mut(|d| {
        let kept =
            d.get_temp_mut_or_insert_with(Id::new(DIAGONAL), || (frame.clone(), s.diagonal()));
        if kept.0 != frame {
            *kept = (frame, s.diagonal());
        }
        kept.1
    })
}

/// A number row's slider, which sends an update at let go, at a typed value and at a wheel
/// notch, as every slider does.
fn number(
    ui: &mut egui::Ui,
    (title, at, ends, unit, hint): (&str, &str, [f32; 2], Unit, &str),
    (calls, d): (&[Value], f32),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let op = &calls[0]["op"];
    let ends = scaled(ends, unit, d);
    let held = op.pointer(at);
    let value = match unit {
        Unit::Ramp(from) => held
            .and_then(|l| l.as_array()?.last()?.as_f64())
            .map_or(from, |v| v as f32),
        _ => held.and_then(Value::as_f64).unwrap_or_default() as f32,
    };
    let step = |v: f32| {
        let written = match unit {
            Unit::Plain | Unit::Long => json!(num(v)),
            Unit::Whole | Unit::Count => json!(v.round().max(0.0) as u64),
            Unit::Ramp(from) => {
                let n = op["curve"]["points"].as_array().map_or(2, Vec::len).max(2);
                let at = |i: usize| num(from + (v - from) * i as f32 / (n - 1) as f32);
                json!((0..n).map(at).collect::<Vec<_>>())
            }
        };
        update(calls, at, &written)
    };
    let slider = ("gizmo deformer", title, value, ends, unit == Unit::Count);
    let shown = ui.scope(|ui| running(ui, slider, &step, now, rec));
    rec.hint(hint);
    shown.response.on_hover_text(hint);
    shown.inner
}

/// A choice row: its title, then a button for each value, lit on the one the call holds. A
/// taper's Axis opens a new taper on that axis, about its low face (section 3).
fn choice(
    ui: &mut egui::Ui,
    (title, at, values, hint): (&str, &str, Choices, &str),
    (calls, s, now): (&[Value], Option<&Shape>, &Now),
    rec: &mut Recorder,
) -> Option<Step> {
    let op = &calls[0]["op"];
    let axis = at
        .strip_suffix("toward")
        .and_then(|o| op.pointer(&format!("{o}axis")));
    let colors = &theme(ui.ctx()).colors;
    label(ui, RichText::new(title).small(), rec);
    let mut asked = None;
    for &(value, face) in values {
        if axis.is_some_and(|a| a == value) {
            continue;
        }
        let step = match (at, s) {
            ("/taper/axis", Some(s)) => opened(
                s,
                json!({ "taper": { "axis": value, "amount": 0.0, "exponent": 1.0 } }),
                now,
            ),
            _ => update(calls, at, &json!(value)),
        };
        let lit = op.pointer(at).is_some_and(|v| v == value);
        let text = RichText::new(face);
        let button = if lit {
            egui::Button::new(text.color(colors.on_lit)).fill(colors.lit)
        } else {
            egui::Button::new(text)
        };
        let shown = ui.add(button.selected(lit));
        let widget = Widget::runs(format!("{title} {face}"), Role::Button, step.clone(), None);
        rec.widget(widget.selected(lit), shown.rect);
        if hover(shown, Some(hint), rec).clicked() {
            asked = Some(step);
        }
    }
    asked
}

/// A number row's ends: a length's are times the box's diagonal `d`.
fn scaled(ends: [f32; 2], unit: Unit, d: f32) -> [f32; 2] {
    if unit == Unit::Long {
        ends.map(|e| e * d)
    } else {
        ends
    }
}

/// How the dots in the view work (section 8 as amended for `DF7c`), and how many points have
/// moved ([`moved`]).
fn dots(ui: &mut egui::Ui, op: &Value, hint: &str, rec: &mut Recorder) {
    let text = format!("{DOTS}. {}", moved(op));
    let line = egui::Label::new(RichText::new(&text).weak()).wrap();
    let shown = ui.add(line).on_hover_text(hint);
    rec.widget(Widget::plain(text, Role::Label), shown.rect);
    rec.hint(hint);
}

/// How many points have moved: a cage's offset ones, or a curve's (two along the part while it
/// has none of its own).
fn moved(op: &Value) -> String {
    match op.get("lattice") {
        Some(l) => {
            let n: u64 = (0..3).map(|a| l["size"][a].as_u64().unwrap_or(1)).product();
            match l["offsets"].as_array().map_or(0, Vec::len) {
                0 => format!("{n} points, none moved"),
                moved => format!("{n} points, {moved} moved"),
            }
        }
        None => match op["curve"]["points"].as_array() {
            Some(p) => format!("{} points, moved", p.len()),
            None => "2 points, along the part".into(),
        },
    }
}

/// What the open deformer's handle `h` drags, for the cursor over it (decision 0041, amended
/// 2026-10-13 for `DF7c-notes`, section 2): its row's title and value as the row shows them,
/// then the row's hint and what Shift snaps it to, a turn to the Gizmo's `steps` of one; for a
/// dot, how many points moved and how the dots work. A length's ends are times the box's
/// diagonal as the rows keep it ([`diagonal`]), or else `fallback`.
pub(super) fn words(
    ctx: &egui::Context,
    calls: &[Value],
    h: &DeformerHandle,
    (fallback, steps): (f32, u32),
) -> Vec<String> {
    let op = &calls[0]["op"];
    let at = match (h.drive, h.axis) {
        (Drive::Cage { .. } | Drive::Curve { .. }, _) => {
            return vec![moved(op), format!("{DOTS}.")];
        }
        (Drive::Move { .. }, Some(a)) => format!("/move/{a}"),
        (drive, _) => drive.at().to_string(),
    };
    let rows = (ROWS.iter().find(|(k, _)| *k == kind(op))).map_or(&[][..], |(_, rows)| *rows);
    let row = rows.iter().find_map(|row| match *row {
        Row::Number(title, a, ends, unit, hint) if a == at => Some((title, ends, unit, hint)),
        _ => None,
    });
    let Some((title, ends, unit, hint)) = row else {
        return Vec::new();
    };
    let frame = calls[0]["frame"].to_string();
    let kept = ctx.data(|d| d.get_temp::<(String, f32)>(Id::new(DIAGONAL)));
    let d = kept
        .filter(|(f, _)| *f == frame)
        .map_or(fallback, |(_, d)| d);
    let value = op.pointer(&at).and_then(Value::as_f64).unwrap_or_default() as f32;
    let key = match h.drive {
        Drive::Number {
            unit: Snaps::Angle, ..
        } => {
            format!(" Shift snaps to {}°.", super::turn_step(steps))
        }
        Drive::Number {
            unit: Snaps::Factor,
            ..
        } => String::new(),
        _ => format!(" Shift snaps to {MOVE_SNAP}."),
    };
    let value = crate::pieces::number(value, scaled(ends, unit, d));
    vec![format!("{title}  {value}"), format!("{hint}.{key}")]
}

/// One `update` of every call with the value at `at` set to `value`, kept one the engine takes:
/// Toward moves off the axis to the next one, and a smaller cage drops its moved points past
/// its edge.
pub(super) fn update(calls: &[Value], at: &str, value: &Value) -> Step {
    let changed = calls.iter().map(|call| {
        let mut call = call.clone();
        let op = &mut call["op"];
        if let Some(slot) = op.pointer_mut(at) {
            *slot = value.clone();
        } else if let Some((parent, key)) = at.rsplit_once('/')
            && let Some(Value::Object(o)) = op.pointer_mut(parent)
        {
            o.insert(key.into(), value.clone());
        }
        if let Some(Value::Object(o)) = op.as_object_mut().and_then(|o| o.values_mut().next()) {
            let axis = o.get("axis").and_then(Value::as_str);
            if let Some(a) = axis.and_then(|a| AXES.iter().position(|x| *x == a))
                && o.get("toward") == Some(&json!(AXES[a]))
            {
                o.insert("toward".into(), json!(AXES[(a + 1) % 3]));
            }
            let size = o.get("size").cloned().unwrap_or_default();
            if let Some(Value::Array(moved)) = o.get_mut("offsets") {
                moved.retain(|m| (0..3).all(|a| m["at"][a].as_u64() < size[a].as_u64()));
            }
        }
        with_args("transform.apply", call)
    });
    session("update", changed.collect())
}
