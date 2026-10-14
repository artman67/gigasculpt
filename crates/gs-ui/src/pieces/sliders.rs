//! Sliders: the brush's value sliders (size, strength, Softness), a control's slider setting one
//! of its command's arguments, the Alpha palette's index slider, the active material's sliders,
//! a curve's Steps, Step strength and Tile and the brush's Wrap Mode ([`running`]). Every one is
//! drawn by [`rail`], so each fits its row the same way (decision 0025, rule 12) and sets its
//! value the same ways (section 4; `interface.sliders`), and each runs the command a drag runs,
//! once a value:
//! - a drag, set when the pen lets go, which moves the value from where it is: a press never
//!   jumps it to the pen. Shift rounds it to whole numbers in the units shown (tenths and finer on
//!   a rail spanning less than ten); with Settings > Sliders' Falloff snap on, Shift held as a
//!   drag of Softness starts takes it to -100, 0 and 100 only, until Shift is let go. With the
//!   center snap on, a rail through 0 stops there over a fiftieth of its length.
//! - a tap on its value, which opens a field to type one in (Enter or a press elsewhere sets it,
//!   Escape leaves it);
//! - a double-click anywhere on it, which opens that field and a fine handle under the rail that
//!   moves the value a tenth as far, until a press lands off them or Escape; with Ctrl held it
//!   puts the default back instead (the preset's own, for the brush's settings);
//! - the wheel turned over it.
//!
//! With Settings > Sliders' show values off, a slider reads its value only under the pen.

mod rail;
mod reads;

use egui::Sense;
use gs_engine::{Navigation, WindowSettings};
use gs_protocol::Step;
use serde_json::json;

use super::{Dragging, Now};
use crate::descriptions;
use crate::direction::{Control, put};
use crate::gizmo::GizmoPrefs;
use crate::record::{Recorder, Role, Widget, with_args};
use crate::values::Value;
pub(in crate::pieces) use rail::rail;
pub(crate) use rail::{fine_shown, knob};
pub(crate) use reads::{Reads, number};

/// Softness's stops, with Settings > Sliders' Falloff snap on.
const FALLOFF_STOPS: [f32; 3] = [-100.0, 0.0, 100.0];

/// The brush size the window shows before one is set, in pixels (gs-engine's default).
const DEFAULT_SIZE: f32 = 64.0;

/// A slider as it is drawn.
pub(super) struct Slider<'a> {
    /// Where it keeps a value being typed.
    pub(super) id: egui::Id,
    pub(super) title: &'a str,
    /// The value it shows.
    pub(super) value: f32,
    /// What it reads beside its rail: its title and value ("Size  40 px").
    pub(super) label: String,
    /// Its value as a number in the units it shows, which an empty typing field shows faintly.
    pub(super) number: String,
    /// What it shows at its rail's ends, so its row fits the widest label it shows.
    pub(super) ends: [String; 2],
    /// How many of the numbers it shows make one of its value: a percentage's 100.
    pub(super) per: f32,
    /// The ends of its rail, and whether the rail runs on a logarithmic scale.
    pub(super) rail: [f32; 2],
    pub(super) log: bool,
    /// Where a Shift drag goes instead of whole numbers, when it starts with Shift held.
    pub(super) stops: &'static [f32],
    /// How sliders behave (`ui.settings`).
    pub(super) settings: WindowSettings,
}

/// What was done with a slider in a frame.
pub(super) struct Moved {
    /// Where a drag holds it, while the drag goes on.
    pub(super) held: Option<f32>,
    /// A value to set, once: where a drag let go, one typed, or one a wheel notch stepped to.
    pub(super) set: Option<f32>,
    /// A Ctrl+double-click asked for its default.
    reset: bool,
    /// Its rail and label.
    pub(super) rect: egui::Rect,
    /// Its fine handle and its typing field, while they are open.
    fine: Option<egui::Rect>,
    pub(super) field: Option<egui::Rect>,
}

/// A value slider. Dragging it with the pen shows the value as it goes; letting go runs
/// `brush.set` with it once, as a typed value or a wheel step does.
pub(super) fn slider(
    ui: &mut egui::Ui,
    v: Value,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    titled(ui, (v, v.title(), Some(said(v))), now, dragging, rec)
}

/// What hovering a brush value's slider says (decision 0060, section 11).
pub(super) fn said(v: Value) -> &'static str {
    match v {
        Value::Size => {
            "How big the brush is, in screen pixels. Keys: S held, [ and ]. (Blender: Radius.)"
        }
        Value::Strength => "How much each dab changes the surface. Key: U held.",
        Value::Falloff => {
            "How quickly the effect fades toward the edge: minus gives a soft wide edge, plus a hard flat center. Key: O held. (Blender: Falloff.)"
        }
    }
}

/// A value slider under another title, where the value means something else (the brush's Size
/// is a soft gizmo drag's Reach), saying `hint` on hover.
pub(super) fn titled(
    ui: &mut egui::Ui,
    (v, title, hint): (Value, &str, Option<&str>),
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    let value = v.of(now.brush);
    let [low, high] = v.range();
    // The rail stops short of the largest size; a typed size goes on to it.
    let top = high.min(2000.0);
    let text = v.text(value);
    let settings = now.layout.settings;
    let s = Slider {
        id: ui.make_persistent_id(("slider", title)),
        title,
        value,
        label: format!("{title}  {text}"),
        number: text.trim_end_matches([' ', 'p', 'x', '%']).to_string(),
        ends: [v.text(low), v.text(top)],
        // Strength reads as a percentage (`Value::text`).
        per: if v == Value::Strength { 100.0 } else { 1.0 },
        rail: [low, top],
        log: v == Value::Size,
        stops: if v == Value::Falloff && settings.falloff_snap {
            &FALLOFF_STOPS
        } else {
            &[]
        },
        settings,
    };
    let moved = rail(ui, &s);
    let step = |value: f32| with_args("brush.set", json!({ v.arg(): value }));
    let key = now.keymap.key_for_args("brush.set", &held_arg(v.arg()));
    let shown = step(moved.held.unwrap_or(value));
    described(ui, rec, &moved, (title, hint), (shown, key));
    if let Some(held) = moved.held {
        *dragging = Some((v, held));
    }
    if moved.reset {
        *dragging = None;
        return Some(with_args("brush.set", brush_default(v)));
    }
    moved.set.map(|value| {
        *dragging = None;
        step(value)
    })
}

/// `brush.set`'s arguments that put `v` back: the session's change dropped, so the preset's own
/// strength or Softness applies again, or the size the window starts with (the size is shared,
/// not the preset's).
fn brush_default(v: Value) -> serde_json::Value {
    match v {
        Value::Size => json!({ "size_px": DEFAULT_SIZE }),
        Value::Strength => json!({ "settings": { "strength": null } }),
        Value::Falloff => json!({ "settings": { "tip": { "focal": null } } }),
    }
}

/// A control's slider (`range`): it shows the state value the control names and runs the
/// control's command once with `arg` (a dotted path for one inside a table) set to a value: where
/// the pen lets go of a drag, one typed, or a wheel step. While it is dragged egui's memory holds
/// the value, so it follows the pen; a value in words ("auto") reads as them until then. It
/// draws the control's short label when it has one (its group's label above already says what it
/// belongs to), and `ui.describe` reports the full one. A `percent` control reads 0.5 as 50%.
pub(super) fn setting(
    ui: &mut egui::Ui,
    c: &Control,
    (arg, [low, high]): (&str, [f32; 2]),
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let id = egui::Id::new(("setting", &c.label));
    let shows = c.shows.as_deref().unwrap_or_default();
    let state = now.state.get(shows);
    let held = ui.data(|d| d.get_temp::<f32>(id));
    let value =
        (held.or(state.and_then(serde_json::Value::as_f64).map(|v| v as f32))).unwrap_or_default();
    // A value in words (the occlusion radius's "auto") reads as them, the slider at its low
    // end, until the pen drags it to a number.
    let words = state
        .and_then(serde_json::Value::as_str)
        .filter(|_| held.is_none());
    // A whole number reads as one: Rows 8, not 8.00 (decision 0060 as amended for S1); a value
    // the control names (`reads`) as its word; a `percent` control as a percentage.
    let whole = state.is_some_and(|v| v.is_u64() || v.is_i64());
    let word = (c.reads.as_ref()).map(|(at, word)| (*at, word.as_str()));
    let percent = c.percent;
    let reads = Reads {
        whole,
        word,
        percent,
    };
    let print = |v: f32| reads.read(v, [low, high]);
    let number = words.map_or_else(|| print(value), str::to_string);
    let title = c.short.as_deref().unwrap_or(&c.label);
    let s = Slider {
        id: ui.make_persistent_id(("slider", &c.label)),
        title,
        value,
        label: format!("{title}  {number}"),
        number: number.trim_end_matches('%').to_string(),
        ends: [low, high].map(print),
        per: if c.percent { 100.0 } else { 1.0 },
        rail: [low, high],
        log: false,
        stops: &[],
        settings: now.layout.settings,
    };
    let moved = rail(ui, &s);
    let command = c.command.as_deref().unwrap_or_default();
    let step = |value: serde_json::Value| {
        let mut args = serde_json::to_value(&c.args).unwrap_or_default();
        put(&mut args, arg, value);
        with_args(command, args)
    };
    let key = now.keymap.key_for_args(command, &c.args);
    let shown = step(json!(moved.held.unwrap_or(value)));
    described(ui, rec, &moved, (&c.label, c.hint.as_deref()), (shown, key));
    if let Some(held) = moved.held {
        ui.data_mut(|d| d.insert_temp(id, held));
    }
    let reset = (moved.reset)
        .then(|| control_default(command, arg))
        .flatten();
    let set = reset.or(moved.set.map(|v| json!(v)));
    set.map(|value| {
        ui.data_mut(|d| d.remove::<f32>(id));
        step(value)
    })
}

/// The value the pen holds on the slider of the control labelled `label` while it drags it, as
/// [`setting`] keeps it; `None` while no drag holds it.
pub(super) fn held(ui: &egui::Ui, label: &str) -> Option<f32> {
    ui.data(|d| d.get_temp::<f32>(egui::Id::new(("setting", label))))
}

/// An index slider, recorded as `label` and titled `title`: picks one of `count` things by its
/// number from 0, `at` being the one picked, running `pick` with it once a value as every
/// slider does; a value between two numbers picks the nearer. The Alpha palette's picks an
/// alpha (`alpha.pick` with `index`), the Material section's a material by place. A
/// Ctrl+double-click has no default to put back.
pub(super) fn index(
    ui: &mut egui::Ui,
    (label, title): (&str, &str),
    (count, at): (usize, usize),
    pick: impl Fn(usize) -> Step,
    (now, rec): (&Now, &mut Recorder),
) -> Option<Step> {
    let last = count.checked_sub(1).filter(|&l| l > 0)?;
    let id = egui::Id::new(("index", label));
    let held = ui.data(|d| d.get_temp::<f32>(id));
    let nearest = |v: f32| (v.round().max(0.0) as usize).min(last);
    let value = held.unwrap_or(at as f32);
    let number = nearest(value).to_string();
    let s = Slider {
        id: ui.make_persistent_id(("slider", label)),
        title,
        value,
        label: format!("{title}  {number}"),
        number,
        ends: ["0".into(), last.to_string()],
        per: 1.0,
        rail: [0.0, last as f32],
        log: false,
        stops: &[],
        settings: now.layout.settings,
    };
    let moved = rail(ui, &s);
    let step = |v: f32| pick(nearest(v));
    let shown = step(moved.held.unwrap_or(value));
    describe(rec, &moved, label, shown, None);
    if let Some(held) = moved.held {
        ui.data_mut(|d| d.insert_temp(id, held));
    }
    moved.set.map(|v| {
        ui.data_mut(|d| d.remove::<f32>(id));
        step(v)
    })
}

/// A material editor's slider: shows `value`, the material's `key`, and runs `material.set` on
/// `material` with it once a value, as every slider does. A Ctrl+double-click has no default to
/// put back; the editor's Reset puts a built-in material's shipped settings back.
pub(super) fn material(
    ui: &mut egui::Ui,
    (title, key, value, ends): (&str, &str, f32, [f32; 2]),
    material: &str,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let step = |v: f32| with_args("material.set", json!({ "material": material, key: v }));
    running(ui, ("material", title, value, ends, false), &step, now, rec)
}

/// A slider of the window's own, not a control's: shows `value` between `ends` as `reads` says
/// (a count's `true` in whole numbers with 1 reading "off"), keeps a drag's value under `scope`
/// and `title` between frames, and runs `step` with the value once a value, as every slider
/// does. A Ctrl+double-click has no default to put back.
pub(crate) fn running<'a>(
    ui: &mut egui::Ui,
    (scope, title, value, ends, reads): (&str, &str, f32, [f32; 2], impl Into<Reads<'a>>),
    step: &dyn Fn(f32) -> Step,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let id = egui::Id::new((scope, title));
    let value = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(value);
    let reads: Reads = reads.into();
    let s = Slider {
        id: ui.make_persistent_id(("slider", title)),
        title,
        value,
        label: format!("{title}  {}", reads.read(value, ends)),
        number: reads.digits(value, ends),
        ends: ends.map(|v| reads.read(v, ends)),
        per: 1.0,
        rail: ends,
        log: false,
        stops: &[],
        settings: now.layout.settings,
    };
    let moved = rail(ui, &s);
    describe(rec, &moved, title, step(moved.held.unwrap_or(value)), None);
    ui.data_mut(|d| match moved.held {
        Some(held) => {
            d.insert_temp(id, held);
        }
        None => d.remove::<f32>(id),
    });
    moved.set.map(step)
}

/// What a control's slider puts back on a Ctrl+double-click: `null` for a brush setting, which
/// drops the session's change so the preset's own applies, and the window's own default for a
/// navigation, gizmo or wheel setting. The others (display, light, Live Detail) have no
/// default the window knows, and keep their value.
fn control_default(command: &str, arg: &str) -> Option<serde_json::Value> {
    let defaults = match command {
        "brush.set" => return Some(serde_json::Value::Null),
        "ui.navigation" => serde_json::to_value(Navigation::default()).ok()?,
        "ui.gizmo" => serde_json::to_value(GizmoPrefs::default()).ok()?,
        "ui.settings" => serde_json::to_value(WindowSettings::default()).ok()?,
        _ => return None,
    };
    defaults.get(arg).cloned()
}

/// Puts a slider in `ui.describe` under `label`, running `shown`, with its fine handle ("Fine
/// Size", running the same) and its typing field ("Type Size") while they are open.
pub(super) fn describe(
    rec: &mut Recorder,
    moved: &Moved,
    label: &str,
    shown: Step,
    key: Option<&str>,
) {
    rec.widget(
        Widget::runs(label, Role::Slider, shown.clone(), key),
        moved.rect,
    );
    extras(rec, moved, label, shown);
}

/// [`describe`], with what hovering the slider says, as a button says it (decision 0060,
/// section 11): `hint` as egui's hover text and on the slider in `ui.describe`, and with Ctrl
/// held over it the description box, with what it runs and its key.
pub(super) fn described(
    ui: &egui::Ui,
    rec: &mut Recorder,
    moved: &Moved,
    (label, hint): (&str, Option<&str>),
    (shown, key): (Step, Option<&str>),
) {
    let id = ui.make_persistent_id(("hint", label));
    let over = ui.interact(moved.rect, id, Sense::hover());
    descriptions::note(ui, &over, &shown, key);
    rec.widget(
        Widget::runs(label, Role::Slider, shown.clone(), key),
        moved.rect,
    );
    if let Some(hint) = hint {
        rec.hint(hint);
        descriptions::hint(ui, &over, hint);
    }
    extras(rec, moved, label, shown);
}

/// The slider's fine handle ("Fine Size", running `shown`) and its typing field ("Type Size")
/// in `ui.describe`, while they are open.
fn extras(rec: &mut Recorder, moved: &Moved, label: &str, shown: Step) {
    if let Some(rect) = moved.fine {
        let fine = Widget::runs(format!("Fine {label}"), Role::Slider, shown, None);
        rec.widget(fine, rect);
    }
    if let Some(rect) = moved.field {
        rec.widget(
            Widget::plain(format!("Type {label}"), Role::TextField),
            rect,
        );
    }
}

/// `{ arg = "drag" }`: the arguments of the key that sets `arg` while held.
fn held_arg(arg: &str) -> toml::Table {
    let mut args = toml::Table::new();
    args.insert(arg.into(), "drag".into());
    args
}
