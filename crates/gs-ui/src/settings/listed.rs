//! The settings the settings window lists (decision 0052, sections 2, 6, 7 and 8). A setting is
//! a `session.state` name that the window's controls both show and set; its pieces' sets name
//! their pages. What each runs to take a value, to search them and their defaults are here, so
//! the window, Reset and the preferences file read one list.

use std::path::PathBuf;
use std::sync::LazyLock;

use gs_engine::{
    LayoutInfo, Navigation, ResetScope, SessionState, SettingInfo, StateFolder, StateValues,
    WindowSettings,
};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::direction::{Control, put, split, title};
use crate::pieces::{Now, controls_of};
use crate::preferences::same;
use crate::record::with_args;

/// A setting: what shows it and sets it.
#[derive(Clone, Debug)]
pub struct Setting {
    /// Its `session.state` name.
    pub name: String,
    /// A choice's row label, else its group, else its set's title; else its control's label.
    pub label: String,
    pub page: String,
    pub command: String,
    /// The controls that show it, in the window's order.
    pub controls: Vec<Control>,
}

/// The settings window's pieces, each with its page, in the layout's order: a set names its
/// page, a piece with none joins the page of the one before it, and a first piece with none
/// names its page after its title.
pub fn paged<'a>(now: &Now<'a>) -> Vec<(&'a str, String)> {
    let Some(direction) = now.direction else {
        return Vec::new();
    };
    let controls = &now.directions.controls;
    let mut page: Option<String> = None;
    let mut paged = Vec::new();
    for piece in &direction.settings {
        let named = controls.piece(piece).and_then(|(set, _)| set.page.clone());
        let here = named
            .or_else(|| page.clone())
            .unwrap_or_else(|| title(controls, piece).to_string());
        page = Some(here.clone());
        paged.push((piece.as_str(), here));
    }
    paged
}

/// The pages, in the order they first come.
pub fn pages(paged: &[(&str, String)]) -> Vec<String> {
    let mut pages: Vec<String> = Vec::new();
    for (_, page) in paged {
        if !pages.contains(page) {
            pages.push(page.clone());
        }
    }
    pages
}

/// Every setting, page by page, in the window's order.
pub fn settings(now: &Now) -> Vec<Setting> {
    let mut listed: Vec<Setting> = Vec::new();
    for (piece, page) in paged(now) {
        let set_title = title(&now.directions.controls, split(piece).0);
        for control in controls_of(piece, now) {
            let (Some(name), Some(command)) = (&control.shows, &control.command) else {
                continue;
            };
            if let Some(setting) = listed.iter_mut().find(|s| &s.name == name) {
                setting.controls.push(control);
                continue;
            }
            listed.push(Setting {
                name: name.clone(),
                label: named(&control, set_title),
                page: page.clone(),
                command: command.clone(),
                controls: vec![control],
            });
        }
    }
    listed
}

/// What names a setting `c` shows, as its page does: a choice by its row's label, else its
/// group's, else its set's title `set`; a switch or a slider by its own label.
fn named(c: &Control, set: &str) -> String {
    if c.on.is_none() {
        return c.label.clone();
    }
    let line = c.line.clone().filter(|l| !l.is_empty());
    (line.or_else(|| c.group.clone())).unwrap_or_else(|| set.to_string())
}

/// The settings `text` finds: each word of it, case aside, in the setting's label, its
/// controls' labels, its command, its name or its page. Empty text finds none.
pub fn found<'s>(listed: &'s [Setting], text: &str) -> Vec<&'s Setting> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let finds = |s: &Setting| {
        let labels = s.controls.iter().map(|c| c.label.as_str());
        let mut said = [s.label.as_str(), &s.command, &s.name, &s.page].join("\n");
        labels.for_each(|l| said.extend(["\n", l]));
        let said = said.to_lowercase();
        words.iter().all(|w| said.contains(w.as_str()))
    };
    listed.iter().filter(|s| finds(s)).collect()
}

impl Setting {
    /// What its controls run to set it to `value`: a slider's command with the value at its
    /// `drag` argument, a switch's with the value for its `flip`, the choice lit by the value,
    /// or the first choice with the value in place of its own. `None` when none can.
    pub fn step_to(&self, value: &Value) -> Option<Step> {
        let at = StateValues([(self.name.clone(), value.clone())].into());
        for c in &self.controls {
            let mut args = serde_json::to_value(&c.args).unwrap_or_default();
            if let Some(path) = c.dragged() {
                put(&mut args, &path, value.clone());
                return Some(with_args(c.command.as_deref()?, args));
            }
            if c.args.values().any(|v| v.as_str() == Some("flip")) {
                flip(&mut args, value);
                return Some(with_args(c.command.as_deref()?, args));
            }
            if c.lit(&at) == Some(true) {
                return c.step();
            }
        }
        // A choice the buttons do not offer (a scale of 1.1 written by hand): the first
        // button's command with the value in place of the one it lights for.
        let c = self.controls.iter().find(|c| c.on.is_some())?;
        let on = serde_json::to_value(c.on.as_ref()?).ok()?;
        let mut args = serde_json::to_value(&c.args).ok()?;
        let arg = args.as_object_mut()?.values_mut().find(|v| same(v, &on))?;
        *arg = value.clone();
        Some(with_args(c.command.as_deref()?, args))
    }

    /// As `ui.settings` lists it, with what puts its default back.
    pub fn info(&self) -> SettingInfo {
        let default = DEFAULTS.get(&self.name).cloned().unwrap_or_default();
        let none = || with_args(&self.command, json!({}));
        SettingInfo {
            name: self.name.clone(),
            label: self.label.clone(),
            page: self.page.clone(),
            command: self.command.clone(),
            reset: self.step_to(&default).unwrap_or_else(none),
        }
    }

    /// Its default, if it is not at it now in `state`: what Reset puts back.
    pub fn changed(&self, state: &StateValues) -> Option<Step> {
        let now = state.get(&self.name).filter(|v| !v.is_null())?;
        let default = DEFAULTS.get(&self.name)?;
        (!same(now, default))
            .then(|| self.step_to(default))
            .flatten()
    }

    /// Whether Reset of `scope` puts it back.
    pub fn reset_by(&self, scope: &ResetScope) -> bool {
        match scope {
            ResetScope::Setting(name) => &self.name == name,
            ResetScope::Page(page) => &self.page == page,
            ResetScope::All => true,
        }
    }
}

/// What the window asks before Reset of `scope` among `listed`.
pub fn question(scope: &ResetScope, listed: &[Setting]) -> String {
    match scope {
        ResetScope::Setting(name) => {
            let setting = listed.iter().find(|s| &s.name == name);
            let label = setting.map_or(name.as_str(), |s| s.label.as_str());
            format!("Put back the default of {label}?")
        }
        ResetScope::Page(page) => {
            let n = listed.iter().filter(|s| &s.page == page).count();
            format!("Put back the defaults of the {n} settings on the {page} page?")
        }
        ResetScope::All => "Put back the default of every setting?".into(),
    }
}

/// Every `"flip"` in `args` given as `value`.
fn flip(args: &mut Value, value: &Value) {
    match args {
        Value::String(s) if s == "flip" => *args = value.clone(),
        Value::Object(o) => o.values_mut().for_each(|v| flip(v, value)),
        _ => {}
    }
}

/// Each setting's default (decision 0052, section 8): what a fresh session has, with the
/// window's own settings at theirs, the first theme at the interface scale 1, autosave as a new
/// state folder has it and the brush used last at start. Each from its type's own `Default`.
static DEFAULTS: LazyLock<StateValues> = LazyLock::new(|| {
    let layout = LayoutInfo {
        scale: 1.0,
        theme: crate::theme::first().name.clone(),
        ..LayoutInfo::default()
    };
    SessionState {
        settings: Some(WindowSettings::default()),
        navigation: Some(Navigation::default()),
        layout: Some(layout),
        autosave: Some(StateFolder::in_folder(PathBuf::new()).settings()),
        launch_brush: Some("last".into()),
        ..SessionState::default()
    }
    .values()
});
