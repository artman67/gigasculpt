//! Header > Deform, a dropdown panel (decision 0041, amended 2026-10-12, steps `DF7a` and
//! `DF-smooth`; moved from the scene tray by `D1`, decision 0060 section 18): the X,
//! Y and Z row, the Reshape and Smoothing groups' spring sliders (`spring.rs`), and the Whole
//! part group's Fit to unit size, Flip mirror, Repeat on other Parts and its Mask switch. Every
//! call's frame is the part's box middle with the part's own axes, read from the parts each call
//! reports ([`heard`]) and kept through a pull; the row and Mask are the window's own settings
//! (`ui.deform`, [`settings`]). A smoothing slider's call is one `mesh.smooth`, along X, Y and
//! Z until the row is touched (decision 0062).

use egui::{Color32, Id, RichText};
use gs_core::transform::Similarity;
use gs_core::{Aabb, NodeId, Vec3};
use gs_engine::{DeformSettings, PartReport, UiDeform};
use gs_protocol::Step;
use serde_json::{Value, json};

use super::spring::{Spring, plain, spring};
use super::{Face, Now, Press, hover, label, pressed_as};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

const SHAPES: &str = "deform shapes";
const SETTINGS: &str = "deform settings";
const TOUCHED: &str = "deform axes touched";
const PIN: &str = "deform pin";
const LAST: &str = "deform last";
const UNLIT: &str = "Light X, Y or Z";
const REPEAT: &str = "Repeat on other Parts";
pub(crate) const AXES: [&str; 3] = ["x", "y", "z"];

/// A part as the section reads it: its live points' box in its own space and where it is. The
/// Gizmo's deformers make their calls with it too (`gizmo/deformer.rs`, [`shape`]).
#[derive(Clone)]
pub(crate) struct Shape {
    name: String,
    lo: Vec3,
    hi: Vec3,
    /// Part to scene.
    place: Similarity,
    shown: bool,
    active: bool,
}

/// The last Deform control that ran, for Repeat on other Parts to run again.
#[derive(Clone, Copy)]
enum Last {
    /// A slider of [`GROUPS`] (its group, its row), at a value, with the axes then lit.
    Row(usize, usize, f32, [bool; 3]),
    Fit,
    Flip([bool; 3]),
}

/// A spring slider of a group, and its calls at a value on a part with the axes lit.
struct Row {
    spring: Spring,
    calls: fn(f32, &Shape, [bool; 3]) -> Vec<Step>,
}

const fn row(
    (title, ends, unit): (&'static str, [f32; 2], &'static str),
    hint: &'static str,
    calls: fn(f32, &Shape, [bool; 3]) -> Vec<Step>,
) -> Row {
    let spring = Spring {
        title,
        ends,
        unit,
        hint,
    };
    Row { spring, calls }
}

const PERCENT: [f32; 2] = [-100.0, 100.0];
const UP: [f32; 2] = [0.0, 100.0];

/// The slider groups, each under its heading, in their order (decision 0041 as amended for
/// `DF-smooth`, section 2).
const GROUPS: [(&str, &[Row]); 2] = [("Reshape", &RESHAPE), ("Smoothing", &SMOOTHING)];

/// The Reshape group, in its order (decision 0041 as amended for `DF7a`, section 2).
const RESHAPE: [Row; 11] = [
    row(
        ("Offset", PERCENT, "%"),
        "Moves the part along the lit axes by a percent of its half size",
        offset,
    ),
    row(
        ("Rotate", [-180.0, 180.0], "°"),
        "Turns the part about its middle, X then Y then Z",
        rotate,
    ),
    row(
        ("Stretch", PERCENT, "%"),
        "Stretches the part along the lit axes about its middle; -100 flattens it",
        stretch,
    ),
    row(
        ("Fold bend", PERCENT, ""),
        "Folds the part at its middle, each half turned flat",
        fold,
    ),
    row(
        ("Smooth bend", PERCENT, ""),
        "Bends the part into an arc along the lit axes",
        bend,
    ),
    row(
        ("Twist", PERCENT, ""),
        "Twists the part about the lit axes, the ends turning opposite ways",
        twist,
    ),
    row(
        ("Taper", PERCENT, ""),
        "Narrows the part toward its high end; below 0 widens it",
        taper,
    ),
    row(
        ("Flatten", PERCENT, "%"),
        "Flattens the low side up to a plane; below 0 the high side",
        flatten,
    ),
    row(
        ("Spherize", PERCENT, ""),
        "Rounds the part toward a sphere about its middle; below 0 away",
        spherize,
    ),
    row(
        ("Inflate", PERCENT, ""),
        "Swells the part along its normals; below 0 shrinks it",
        inflate,
    ),
    row(
        ("Noise", UP, ""),
        "Roughens the part with smooth noise",
        noise,
    ),
];

/// The Smoothing group, in its order (decision 0041 as amended for `DF-smooth`, section 1).
const SMOOTHING: [Row; 4] = [
    row(
        ("Smooth", PERCENT, ""),
        "Smooths the part toward its neighbours; below 0 sharpens it once",
        |v, s, lit| smoothing(("smooth", None), v, s, lit),
    ),
    row(
        ("Polish", UP, ""),
        "Smooths the part and keeps its volume",
        |v, s, lit| smoothing(("polish", Some(true)), v, s, lit),
    ),
    row(
        ("Polish free", UP, ""),
        "Smooths the part by sliding points along its surface",
        |v, s, lit| smoothing(("polish", Some(false)), v, s, lit),
    ),
    row(
        ("Relax", UP, ""),
        "Evens out the part's edges along its surface",
        |v, s, lit| smoothing(("relax", None), v, s, lit),
    ),
];

/// Keeps each part's box and place as a call left them, for the next frame.
pub(crate) fn heard(ctx: &egui::Context, parts: &[PartReport]) {
    let shape = |p: &PartReport| {
        let nodes = 0..p.store.node_count() as NodeId;
        let b = nodes
            .map(|n| p.store.bounds(n))
            .fold(Aabb::EMPTY, Aabb::union);
        (b != Aabb::EMPTY).then(|| Shape {
            name: p.name.to_string(),
            lo: b.lo,
            hi: b.hi,
            place: p.transform,
            shown: p.shown,
            active: p.active,
        })
    };
    let shapes: Vec<Shape> = parts.iter().filter_map(shape).collect();
    ctx.data_mut(|d| d.insert_temp(Id::new(SHAPES), shapes));
}

/// Each part as the last call reported it ([`heard`]).
fn shapes(ctx: &egui::Context) -> Vec<Shape> {
    ctx.data(|d| d.get_temp(Id::new(SHAPES)))
        .unwrap_or_default()
}

/// The part called `name` as the last call reported it.
pub(crate) fn shape(ctx: &egui::Context, name: &str) -> Option<Shape> {
    shapes(ctx).into_iter().find(|s| s.name == name)
}

/// The row and Mask after `change` (`ui.deform`): X lit and Mask on at first. A change of an
/// axis marks the row touched, after which the Smoothing group works along the lit axes too.
pub(crate) fn settings(ctx: &egui::Context, change: &UiDeform) -> DeformSettings {
    ctx.data_mut(|d| {
        if change.x.or(change.y).or(change.z).is_some() {
            d.insert_temp(Id::new(TOUCHED), true);
        }
        let s = d.get_temp_mut_or_insert_with(Id::new(SETTINGS), first);
        s.x = change.x.unwrap_or(s.x);
        s.y = change.y.unwrap_or(s.y);
        s.z = change.z.unwrap_or(s.z);
        s.mask = change.mask.unwrap_or(s.mask);
        *s
    })
}

fn first() -> DeformSettings {
    DeformSettings {
        x: true,
        y: false,
        z: false,
        mask: true,
    }
}

/// Draws the section. Returns what it asked to run.
pub(super) fn section(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let set = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(Id::new(SETTINGS), first));
    let lit = [set.x, set.y, set.z];
    let asked = axes(ui, lit, now, rec);
    ui.end_row();
    let shapes = shapes(ui.ctx());
    // A pull keeps the box and place it started with, while the part moves under the pen.
    let pinned = ui.data(|d| d.get_temp::<Shape>(Id::new(PIN)));
    let Some(shape) = pinned.or_else(|| shapes.iter().find(|s| s.active).cloned()) else {
        label(ui, RichText::new("No part to deform").weak(), rec);
        ui.end_row();
        return asked;
    };
    let touched = ui.data(|d| d.get_temp::<bool>(Id::new(TOUCHED)).unwrap_or_default());
    let pulled = sliders(ui, &shape, (lit, touched), now, rec);
    let pressed = whole(ui, (&shape, &shapes), set, now, rec);
    pulled.or(pressed).or(asked)
}

/// X, Y and Z, each lit while its axis is, each sending `ui.deform` with its own turned.
fn axes(ui: &mut egui::Ui, lit: [bool; 3], now: &Now, rec: &mut Recorder) -> Option<Step> {
    let mut asked = None;
    for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        let step = with_args("ui.deform", json!({ AXES[i]: !lit[i] }));
        let press = Press {
            label: axis,
            step: step.clone(),
            key: None,
            lit: lit[i],
            shows: None,
        };
        let hint = format!(
            "The Deform controls work along the part's own {axis} axis; Smoothing works along \
             all three until X, Y or Z is pressed"
        );
        let shown = pressed_as(ui, press, Face::Label, now, rec);
        if hover(shown, Some(&hint), rec).clicked() {
            asked = Some(step);
        }
    }
    asked
}

/// The slider groups, each under its heading: a spring slider a row, each off with no axis lit.
/// Until the row is `touched` the Smoothing group works along X, Y and Z, whatever is lit
/// (decision 0062): a smoothing along X alone, the row's start, surprises.
fn sliders(
    ui: &mut egui::Ui,
    shape: &Shape,
    (lit, touched): ([bool; 3], bool),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let (mut asked, mut open) = (None, false);
    for (g, (heading, rows)) in GROUPS.iter().enumerate() {
        // GROUPS[1] is Smoothing.
        let lit = if g == 1 && !touched { [true; 3] } else { lit };
        let off = (!lit.contains(&true)).then_some(UNLIT);
        label(ui, RichText::new(*heading).small().weak(), rec);
        ui.end_row();
        for (i, r) in rows.iter().enumerate() {
            let calls = |v: f32| (r.calls)(v, shape, lit);
            let sprung = spring(ui, &r.spring, &calls, off, now, rec);
            ui.end_row();
            if let Some(value) = sprung.landed {
                remember(ui, Last::Row(g, i, value, lit));
            }
            open |= sprung.open;
            asked = sprung.step.or(asked);
        }
    }
    ui.data_mut(|d| {
        if open {
            d.insert_temp(Id::new(PIN), shape.clone());
        } else {
            d.remove::<Shape>(Id::new(PIN));
        }
    });
    asked
}

/// The Whole part group: Fit to unit size, Flip mirror, and Repeat on other Parts with Mask.
fn whole(
    ui: &mut egui::Ui,
    (shape, shapes): (&Shape, &[Shape]),
    set: DeformSettings,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    label(ui, RichText::new("Whole part").small().weak(), rec);
    ui.end_row();
    let lit = [set.x, set.y, set.z];
    let mut asked = None;
    let fitted = fit(shape);
    let hint = "Scales the part so its largest side is 2, about its middle";
    if button(ui, ("Fit to unit size", &fitted, hint), now, rec) {
        remember(ui, Last::Fit);
        asked = Some(fitted);
    }
    let flipped = plain(flips(shape, lit));
    let hint = "Mirrors the part across its middle on the lit axes";
    if !lit.contains(&true) {
        off(ui, "Flip mirror", UNLIT, rec);
    } else if button(ui, ("Flip mirror", &flipped, hint), now, rec) {
        remember(ui, Last::Flip(lit));
        asked = Some(flipped);
    }
    ui.end_row();
    asked = repeat(ui, shapes, set.mask, now, rec).or(asked);
    let mask = with_args("ui.deform", json!({ "mask": !set.mask }));
    let press = Press {
        label: "Mask",
        step: mask.clone(),
        key: None,
        lit: set.mask,
        shows: None,
    };
    let hint = "Repeat on other Parts keeps each part's own mask";
    if hover(
        pressed_as(ui, press, Face::Label, now, rec),
        Some(hint),
        rec,
    )
    .clicked()
    {
        asked = Some(mask);
    }
    ui.end_row();
    asked
}

/// Repeat on other Parts: the last control's calls rebuilt for each other part that shows, as
/// one step, each `transform.apply` and `mesh.smooth` keeping each part's mask or not (`mask`).
fn repeat(
    ui: &mut egui::Ui,
    shapes: &[Shape],
    mask: bool,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let last = ui.data(|d| d.get_temp::<Last>(Id::new(LAST)));
    let others: Vec<&Shape> = shapes.iter().filter(|s| s.shown && !s.active).collect();
    let Some(last) = last else {
        off(
            ui,
            REPEAT,
            "Pull, type or press a Deform control first",
            rec,
        );
        return None;
    };
    if others.is_empty() {
        off(ui, REPEAT, "No other part shows", rec);
        return None;
    }
    let again = |s: &Shape| match last {
        Last::Row(group, row, value, lit) => (GROUPS[group].1[row].calls)(value, s, lit),
        Last::Fit => vec![fit(s)],
        Last::Flip(lit) => flips(s, lit),
    };
    let masked = |mut step: Step| {
        if matches!(step.command.as_str(), "transform.apply" | "mesh.smooth") {
            step.args["mask"] = json!(mask);
        }
        step
    };
    let steps = others.into_iter().flat_map(again).map(masked).collect();
    let step = plain(steps);
    let hint = "Runs the last Deform control again on every other part that shows";
    button(ui, (REPEAT, &step, hint), now, rec).then_some(step)
}

/// A button running `step`, saying `hint` on hover. Returns whether it was pressed.
fn button(
    ui: &mut egui::Ui,
    (label, step, hint): (&str, &Step, &str),
    now: &Now,
    rec: &mut Recorder,
) -> bool {
    pressed(ui, (label, step.clone(), None, false, hint), now, rec).is_some()
}

/// A button running `step`, bound to `key`, lit while `lit` and saying `hint` on hover. Returns
/// `step` when pressed. The Gizmo's deformers draw theirs with it too (`gizmo/deformer.rs`).
pub(crate) fn pressed(
    ui: &mut egui::Ui,
    (label, step, key, lit, hint): (&str, Step, Option<&str>, bool, &str),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let press = Press {
        label,
        step: step.clone(),
        key,
        lit,
        shows: None,
    };
    hover(
        pressed_as(ui, press, Face::Label, now, rec),
        Some(hint),
        rec,
    )
    .clicked()
    .then_some(step)
}

/// A button that cannot run now, dimmed, saying `why` on hover.
fn off(ui: &mut egui::Ui, label: &str, why: &str, rec: &mut Recorder) {
    let dim = theme(ui.ctx()).colors.text_off;
    let button = egui::Button::new(RichText::new(label).color(dim)).fill(Color32::TRANSPARENT);
    let shown = ui.add_enabled(false, button).on_disabled_hover_text(why);
    rec.widget(Widget::plain(label, Role::Button).disabled(why), shown.rect);
}

fn remember(ui: &egui::Ui, last: Last) {
    ui.data_mut(|d| d.insert_temp(Id::new(LAST), last));
}

impl Shape {
    /// The box's middle, in the part's own space.
    pub(crate) fn middle(&self) -> Vec3 {
        (self.lo + self.hi) * 0.5
    }

    /// The middle of the box's low face on axis `a`, in the part's own space: a taper about it
    /// keeps the low end's size.
    pub(crate) fn low(&self, a: usize) -> Vec3 {
        let mut at = self.middle().to_array();
        at[a] = self.lo.to_array()[a];
        Vec3::from_array(at)
    }

    /// Half the box's side along each axis, in scene units.
    pub(crate) fn half(&self) -> [f32; 3] {
        ((self.hi - self.lo) * 0.5)
            .to_array()
            .map(|h| self.place.length(h))
    }

    /// Its name, its box in its own space and where it is: the Gizmo's deformers frame the box
    /// in the gizmo's axes instead (decision 0041, amended 2026-10-13 for `DF7c`, section 1).
    pub(crate) fn own(&self) -> (&str, Aabb, Similarity) {
        let b = Aabb {
            lo: self.lo,
            hi: self.hi,
        };
        (&self.name, b, self.place)
    }

    /// The box's diagonal, in scene units.
    pub(crate) fn diagonal(&self) -> f32 {
        self.place.length((self.hi - self.lo).len2().sqrt())
    }

    /// The part's own axis `a` in the scene.
    fn axis(&self, a: usize) -> Vec3 {
        let mut unit = [0.0; 3];
        unit[a] = 1.0;
        self.place.direction(Vec3::from_array(unit))
    }

    /// A call's `part` and `frame`: this part, about its own axes through `at` (in its own
    /// space).
    fn placed(&self, at: Vec3) -> Value {
        json!({
            "part": self.name,
            "frame": {
                "origin": nums(self.place.point(at).to_array()),
                "axes": nums(self.place.rotation.to_array()),
            },
        })
    }

    /// `transform.apply` of `op` on this part, about its own axes through `at` (in its own
    /// space), with the box in that frame when `boxed`.
    pub(crate) fn call(&self, at: Vec3, op: Value, boxed: bool) -> Step {
        let mut args = self.placed(at);
        args["op"] = op;
        if boxed {
            let side = |v: Vec3| nums((v - at).to_array().map(|c| self.place.length(c)));
            args["box"] = json!({ "lo": side(self.lo), "hi": side(self.hi) });
        }
        with_args("transform.apply", args)
    }
}

/// `make` for each lit axis, X then Y then Z.
fn each(lit: [bool; 3], make: impl Fn(usize) -> Step) -> Vec<Step> {
    (0..3).filter(|&a| lit[a]).map(make).collect()
}

/// `step` keeping only the lit axes' part of its change.
fn kept(mut step: Step, lit: [bool; 3]) -> Vec<Step> {
    step.args["axes"] = json!({ "x": lit[0], "y": lit[1], "z": lit[2] });
    vec![step]
}

/// `x` as the number its shortest decimal reads, so a line the engine logs (an f32 written
/// back) reads the value the window sent.
pub(crate) fn num(x: f32) -> f64 {
    x.to_string().parse().unwrap_or(f64::from(x))
}

fn nums<const N: usize>(xs: [f32; N]) -> [f64; N] {
    xs.map(num)
}

fn offset(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    let half = s.half();
    let by = (0..3).filter(|&a| lit[a]).fold(Vec3::default(), |d, a| {
        d + s.axis(a) * (v / 100.0 * half[a])
    });
    vec![s.call(s.middle(), json!({ "move": nums(by.to_array()) }), false)]
}

/// `op` by `degrees` about each lit axis through the middle. A fold or bend turns toward the
/// next axis (X toward Y, Y toward Z, Z toward X); a bend or twist has the box written in.
fn about(op: &str, degrees: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    each(lit, |a| {
        let mut args = json!({ "axis": AXES[a], "degrees": num(degrees) });
        if op == "fold" || op == "bend" {
            args["toward"] = json!(AXES[(a + 1) % 3]);
        }
        s.call(
            s.middle(),
            json!({ op: args }),
            op == "bend" || op == "twist",
        )
    })
}

fn rotate(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    about("rotate", v, s, lit)
}

fn stretch(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    let by = lit.map(|on| if on { 1.0 + v / 100.0 } else { 1.0 });
    vec![s.call(s.middle(), json!({ "scale": nums(by) }), false)]
}

fn fold(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    about("fold", 0.9 * v, s, lit)
}

fn bend(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    about("bend", 1.8 * v, s, lit)
}

fn twist(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    about("twist", 3.6 * v, s, lit)
}

/// About the box's low face on the axis, so the low end stays.
fn taper(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    each(lit, |a| {
        let op = json!({ "taper": { "axis": AXES[a], "amount": num(v / 100.0) } });
        s.call(s.low(a), op, true)
    })
}

fn flatten(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    let half = s.half();
    each(lit, |a| {
        let op = json!({ "clip": { "axis": AXES[a], "depth": num(-v / 100.0 * half[a]) } });
        s.call(s.middle(), op, true)
    })
}

fn spherize(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    let half = s.half();
    let count = lit.iter().filter(|&&on| on).count().max(1);
    let radius = (0..3).filter(|&a| lit[a]).map(|a| half[a]).sum::<f32>() / count as f32;
    let op = json!({ "spherize": { "amount": num(v / 100.0), "radius": num(radius) } });
    kept(s.call(s.middle(), op, false), lit)
}

fn inflate(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    let op = json!({ "inflate": num(v / 100.0 * s.diagonal()) });
    kept(s.call(s.middle(), op, false), lit)
}

fn noise(v: f32, s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    let d = s.diagonal();
    let op =
        json!({ "noise": { "amount": num(v / 2000.0 * d), "scale": num(d / 20.0), "seed": 0 } });
    kept(s.call(s.middle(), op, false), lit)
}

/// `mesh.smooth` in `mode` (with Polish's `keep`) by `amount`, in the part's own frame through
/// its middle, keeping the lit axes' part of its change. No box: only a gradient reads one.
fn smoothing(
    (mode, keep): (&str, Option<bool>),
    amount: f32,
    s: &Shape,
    lit: [bool; 3],
) -> Vec<Step> {
    let mut args = s.placed(s.middle());
    args["mode"] = json!(mode);
    if let Some(keep) = keep {
        args["keep"] = json!(keep);
    }
    args["amount"] = json!(num(amount));
    kept(with_args("mesh.smooth", args), lit)
}

/// Fit to unit size: scaled about its middle so its box's largest side is 2.
fn fit(s: &Shape) -> Step {
    let largest = s.half().into_iter().fold(0.0, f32::max) * 2.0;
    let k = num(2.0 / largest);
    s.call(s.middle(), json!({ "scale": [k, k, k] }), false)
}

/// Flip mirror: `mesh.flip` across the middle on each lit axis.
fn flips(s: &Shape, lit: [bool; 3]) -> Vec<Step> {
    each(lit, |a| {
        let args = json!({ "part": s.name, "axis": AXES[a], "about": "middle" });
        with_args("mesh.flip", args)
    })
}
