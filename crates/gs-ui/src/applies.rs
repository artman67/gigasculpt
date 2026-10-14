//! Which of the window's controls can run now (decision 0025, section 6). After every call the
//! engine dry-runs what the controls drawn last run and reports those it refuses, with why
//! (`CallReport::refused`); a control whose command is refused is drawn disabled, the reason
//! on hover. A Reference View's controls name the view `reference`, and the Blender buttons
//! their folder by name, which the window fills in when one runs, so each is asked about as it
//! would run, and a control opening a form as its form's command too, with the arguments it
//! gives (amended 2026-10-14). The window can also hold a command back itself, saying why, and
//! dim a toggle that has no effect now while it still runs (decision 0025, section 6, amended
//! 2026-10-12).

use std::collections::BTreeMap;

use gs_engine::{PanelInfo, Shows};
use gs_protocol::{ErrorCode, Step};

use crate::shell::refusals::words;

/// The controls' steps as last asked about, and those the engine refused.
#[derive(Default)]
pub struct Applies {
    /// Each step as drawn, and as it would run (a reference's view filled in).
    asked: Vec<(Step, Step)>,
    /// Each step as drawn that cannot run now, with why.
    refused: Vec<(Step, String)>,
    /// Commands the window holds back itself, with why: the engine would run them, but the
    /// window knows they would do nothing (Read from Blender with nothing new).
    held: BTreeMap<String, String>,
    /// Steps the window holds back itself, with why, as `held` but one step of a command: a
    /// level arrow at its end.
    held_steps: Vec<(Step, String)>,
    /// The state values whose toggles the window dims, with why, though they still run: they
    /// have no effect now (the Live Detail switch under a brush whose automask keeps Live
    /// Detail off).
    idle: BTreeMap<String, String>,
}

impl Applies {
    /// The steps the controls in `panels` run, each once, as they would run now (`resolve`
    /// fills in what the window fills in when one runs). The palette's rows are not controls
    /// and are left out.
    pub fn ask(&mut self, panels: &[PanelInfo], resolve: impl Fn(&mut [Step])) -> Vec<Step> {
        let controls = panels.iter().filter(|p| p.id != "palette");
        let drawn = controls
            .flat_map(|p| &p.widgets)
            .filter_map(|w| w.command.as_ref());
        self.asked.clear();
        let mut out: Vec<Step> = Vec::new();
        for step in drawn {
            if self.asked.iter().any(|(d, _)| d == step) {
                continue;
            }
            let mut runs = [step.clone()];
            resolve(&mut runs);
            let [runs] = runs;
            for runs in [form_of(&runs), Some(runs)].into_iter().flatten() {
                if !out.contains(&runs) {
                    out.push(runs.clone());
                }
                self.asked.push((step.clone(), runs));
            }
        }
        out
    }

    /// Takes the engine's answer: the steps it refused, as they would run, with why, said as the
    /// window says a refusal of each (`shell::refusals::words`): a sentence, in a person's words
    /// where the engine's name what a person never sees (decision 0025, section 2, amended
    /// 2026-10-14).
    pub fn heard(&mut self, refused: &[(Step, String)]) {
        let of = |(drawn, runs): &(Step, Step)| {
            let (_, why) = refused.iter().find(|(s, _)| s == runs)?;
            let e = gs_engine::errors::error(ErrorCode::Precondition, why);
            Some((drawn.clone(), words(Some(runs), &e).0))
        };
        self.refused = self.asked.iter().filter_map(of).collect();
    }

    /// Holds back every step running `command`, saying `why`; `None` lets them run again.
    pub fn hold(&mut self, command: &str, why: Option<String>) {
        keep(&mut self.held, command, why);
    }

    /// Holds back `step` alone, as a control draws it, saying `why`; `None` lets it run again.
    pub fn hold_step(&mut self, step: Step, why: Option<String>) {
        self.held_steps.retain(|(s, _)| *s != step);
        self.held_steps.extend(why.map(|why| (step, why)));
    }

    /// Dims every toggle showing the state value `shows`, saying `why`, while they still run;
    /// `None` stops dimming them.
    pub fn idle(&mut self, shows: &str, why: Option<String>) {
        keep(&mut self.idle, shows, why);
    }

    /// Why `step`, as a control draws it, cannot run now, if it cannot: the window's reason
    /// for holding it back, which says more (Read from Blender with no folder yet is "Nothing
    /// new"), else the engine's.
    pub fn why(&self, step: &Step) -> Option<&str> {
        self.held
            .get(&step.command)
            .or_else(|| why_in(&self.held_steps, step))
            .or_else(|| why_in(&self.refused, step))
            .map(String::as_str)
    }

    /// Why a toggle showing `shows` is dimmed though it still runs, if it is.
    pub fn why_idle(&self, shows: &Shows) -> Option<&str> {
        self.idle.get(&shows.value).map(String::as_str)
    }
}

/// The step a control opening a form would run with the arguments it gives (`ui.palette` with
/// `form`), so the control is asked about as that step too: Revert… is gray while the scene has
/// no file, as Revert would refuse. An argument the form still asks for (Export…'s path) the
/// engine fills in from the command's examples to ask (gs-engine `engine/session.rs`,
/// `needed`), so Export… is gray in an empty scene too (amended 2026-10-14, UI-last).
fn form_of(step: &Step) -> Option<Step> {
    if step.command != "ui.palette" {
        return None;
    }
    let form = step.args.get("form")?.as_str()?;
    let args = step.args.get("args").cloned();
    Some(Step {
        command: form.into(),
        args: args.unwrap_or_else(|| serde_json::json!({})),
    })
}

/// Why `step` is in `list`, if it is.
fn why_in<'a>(list: &'a [(Step, String)], step: &Step) -> Option<&'a String> {
    list.iter().find(|(s, _)| s == step).map(|(_, why)| why)
}

/// Keeps `why` under `key` in `reasons`; `None` drops it.
fn keep(reasons: &mut BTreeMap<String, String>, key: &str, why: Option<String>) {
    match why {
        Some(why) => reasons.insert(key.into(), why),
        None => reasons.remove(key),
    };
}
