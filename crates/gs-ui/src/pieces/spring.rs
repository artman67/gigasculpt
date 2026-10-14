//! A spring-back slider (decision 0041, amended 2026-10-12, step `DF7a`, section 1): it rests at
//! 0. A press on its rail opens a `gizmo.deformer` session with its calls at 0, each frame the
//! drag moves it sends the session the whole calls at the value it holds (the window sends at
//! most one a frame, `Shell::queue`), and letting go accepts them as one step and it reads 0
//! again; Escape cancels, and the rest of that press sends nothing. A typed value runs its
//! calls at once as one plain step, the line a pull to it leaves. The wheel and a
//! Ctrl+double-click do nothing. It draws as every slider does (`sliders.rs`).

use egui::{Key, Sense};
use gs_protocol::Step;
use serde_json::{Value, json};

use super::Now;
use super::sliders::{Slider, describe, number, rail};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;

/// A spring slider: its title, its rail's ends, the unit it shows its value in ("" for none)
/// and what it does, on hover.
pub(super) struct Spring {
    pub title: &'static str,
    pub ends: [f32; 2],
    pub unit: &'static str,
    pub hint: &'static str,
}

/// Where a pull is between frames.
#[derive(Clone, Copy)]
enum Pull {
    /// Its session is open with the calls at this value, the last sent.
    Open(f32),
    /// Escape cancelled it while the pen still holds the slider.
    Dropped,
}

/// What a spring slider did in a frame.
#[derive(Default)]
pub(super) struct Sprung {
    /// What it asks to run.
    pub step: Option<Step>,
    /// A value that landed as a step: a pull let go away from 0, or one typed.
    pub landed: Option<f32>,
    /// A pull's session is open.
    pub open: bool,
}

/// Draws `s`, whose `calls` at a value are the calls a pull or a typed value runs, or off with
/// why (`off`, or why its plain step cannot run now).
pub(super) fn spring(
    ui: &mut egui::Ui,
    s: &Spring,
    calls: &dyn Fn(f32) -> Vec<Step>,
    off: Option<&str>,
    now: &Now,
    rec: &mut Recorder,
) -> Sprung {
    let id = ui.make_persistent_id(("spring", s.title));
    let pull = ui.data(|d| d.get_temp::<Pull>(id));
    let rest = plain(calls(0.0));
    // While a pull is open its session holds the part, so only `off` ends it.
    let refused = || (now.applies?.why(&rest)).filter(|_| pull.is_none());
    if let Some(why) = off.or_else(refused) {
        ui.data_mut(|d| d.remove::<Pull>(id));
        let shown = ui.add_enabled_ui(false, |ui| {
            ui.visuals_mut().override_text_color = Some(theme(ui.ctx()).colors.text_off);
            rail(ui, &slider(ui, s, 0.0, now))
        });
        shown.response.on_disabled_hover_text(why);
        let widget = Widget::runs(s.title, Role::Slider, rest, None).disabled(why);
        rec.widget(widget, shown.inner.rect);
        let cancel = matches!(pull, Some(Pull::Open(_))).then(|| phase("cancel"));
        return Sprung {
            step: cancel,
            ..Sprung::default()
        };
    }
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    let held = match pull {
        Some(Pull::Open(v)) => v,
        _ => 0.0,
    };
    let moved = rail(ui, &slider(ui, s, held, now));
    let (step, pull, landed) = match (pull, moved.held) {
        // egui may end the drag itself at Escape, so the pen can still be down or not.
        (Some(Pull::Open(_)), _) if escape => (Some(phase("cancel")), Some(Pull::Dropped), None),
        (Some(Pull::Open(sent)), Some(v)) => {
            let update = v
                .total_cmp(&sent)
                .is_ne()
                .then(|| session("update", calls(v)));
            (update, Some(Pull::Open(v)), None)
        }
        (Some(Pull::Open(sent)), None) => {
            (Some(phase("accept")), None, (sent != 0.0).then_some(sent))
        }
        (Some(Pull::Dropped), Some(_)) => (None, Some(Pull::Dropped), None),
        (Some(Pull::Dropped), None) => (None, None, None),
        (None, Some(v)) => (Some(session("open", calls(v))), Some(Pull::Open(v)), None),
        // Only a typed value: a wheel notch sets with no field open.
        (None, None) => {
            let typed = moved.set.filter(|v| moved.field.is_some() && *v != 0.0);
            (typed.map(|v| plain(calls(v))), None, typed)
        }
    };
    ui.data_mut(|d| {
        if let Some(p) = pull {
            d.insert_temp(id, p);
        } else {
            d.remove::<Pull>(id);
        }
    });
    let value = match pull {
        Some(Pull::Open(v)) => v,
        _ => 0.0,
    };
    describe(rec, &moved, s.title, plain(calls(value)), None);
    rec.hint(s.hint);
    ui.interact(moved.rect, id.with("hint"), Sense::hover())
        .on_hover_text(s.hint);
    Sprung {
        step,
        landed,
        open: matches!(pull, Some(Pull::Open(_))),
    }
}

/// `s` as a slider draws it, showing `value`.
fn slider<'a>(ui: &egui::Ui, s: &'a Spring, value: f32, now: &Now) -> Slider<'a> {
    let reads = |v: f32| format!("{}{}", number(v, s.ends), s.unit);
    Slider {
        id: ui.make_persistent_id(("slider", s.title)),
        title: s.title,
        value,
        label: format!("{}  {}", s.title, reads(value)),
        number: number(value, s.ends),
        ends: s.ends.map(reads),
        per: 1.0,
        rail: s.ends,
        log: false,
        stops: &[],
        settings: now.layout.settings,
    }
}

/// `steps` as one step: the one, or a `cmd.batch` of them, which the window sends as a batch.
pub(super) fn plain(mut steps: Vec<Step>) -> Step {
    if steps.len() == 1 {
        return steps.remove(0);
    }
    let commands: Vec<Value> = (steps.into_iter())
        .map(|s| json!({ "command": s.command, "args": s.args }))
        .collect();
    with_args("cmd.batch", json!({ "commands": commands }))
}

/// The open session's `phase` with `steps`' arguments as its transforms, or as its `smooth`
/// when they are one `mesh.smooth` (decision 0041 as amended for `DF-smooth`, section 5).
pub(crate) fn session(phase: &str, mut steps: Vec<Step>) -> Step {
    if steps.len() == 1 && steps[0].command == "mesh.smooth" {
        let smooth = steps.remove(0).args;
        return with_args(
            "gizmo.deformer",
            json!({ "phase": phase, "smooth": smooth }),
        );
    }
    let transforms: Vec<Value> = steps.into_iter().map(|s| s.args).collect();
    with_args(
        "gizmo.deformer",
        json!({ "phase": phase, "transforms": transforms }),
    )
}

/// The open session's `phase`, which names no transforms.
pub(crate) fn phase(phase: &str) -> Step {
    with_args("gizmo.deformer", json!({ "phase": phase }))
}
