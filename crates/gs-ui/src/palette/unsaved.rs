//! The one question before unsaved changes are lost (decision 0016, section 17, amended
//! 2026-10-13; review 0009, C8): closing the window, New, Open and Recover each ask Save, Don't
//! save or Cancel in the palette. Save saves first, through Save as when the scene has no file,
//! and goes on once the save lands; Don't save goes on letting the changes go; Cancel, Escape, a
//! cancelled save or a failed one go on with nothing. Enter answers Save. With nothing to lose,
//! there is no question.

use std::path::Path;

use egui::RichText;
use gs_protocol::{Catalog, Step};
use serde_json::{Value, json};

use super::form::warned;
use super::{Palette, dialog, missing};
use crate::pieces::{Now, Press, label};
use crate::record::{Recorder, with_args};

/// What waits on the question's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    /// The window closes.
    Close,
    /// The step runs, asking first for the arguments named (Open's path).
    Run(Step, Vec<String>),
}

/// The question's state.
#[derive(Clone, Debug, Default)]
pub struct Unsaved {
    /// Asked, waiting for an answer.
    pub asking: Option<Then>,
    /// Answered Save: what goes on once the save lands.
    pub saving: Option<Then>,
    /// Answered so that the window closes.
    pub closes: bool,
}

impl Unsaved {
    /// Lets the question and a save it waits for go: nothing goes on.
    pub fn stop(&mut self) {
        (self.asking, self.saving) = (None, None);
    }

    /// The steps waiting on the question: asked about, or waiting for the save first.
    pub fn waiting(&self) -> impl Iterator<Item = &Step> {
        let then = [&self.asking, &self.saving].into_iter().flatten();
        then.filter_map(|t| match t {
            Then::Run(step, _) => Some(step),
            Then::Close => None,
        })
    }
}

/// Whether `command` drops the scene, so it asks about unsaved changes first.
pub(crate) fn guards(command: &str) -> bool {
    matches!(command, "file.new" | "file.open" | "file.recover")
}

/// The question's answers.
#[derive(Clone, Copy)]
enum Reply {
    Save,
    Discard,
    Cancel,
}

impl Palette {
    /// Asks before unsaved changes are lost to `then`; with none to lose, goes on at once.
    /// Gives the step to run now, if any.
    pub fn first(&mut self, then: Then, catalog: &Catalog) -> Result<Option<Step>, String> {
        if !self.scene.unsaved {
            return self.go_on(then, false, catalog);
        }
        self.set(true, None);
        self.unsaved.asking = Some(then);
        Ok(None)
    }

    /// The scene was heard of again: once a save the question waits for has landed, goes on.
    pub fn heard(&mut self, catalog: &Catalog) -> Result<Option<Step>, String> {
        match self.unsaved.saving.take() {
            Some(then) if !self.scene.unsaved => self.go_on(then, false, catalog),
            waiting => {
                self.unsaved.saving = waiting;
                Ok(None)
            }
        }
    }

    /// Goes on to `then`: the window closes, or its step runs, letting the changes go if
    /// `discard`, once what it lacks is asked.
    fn go_on(
        &mut self,
        then: Then,
        discard: bool,
        catalog: &Catalog,
    ) -> Result<Option<Step>, String> {
        let (step, also) = match then {
            Then::Close => {
                self.unsaved.closes = true;
                return Ok(None);
            }
            Then::Run(step, also) => (step, also),
        };
        let step = if discard {
            dialog::given(&step, "discard_changes", Value::Bool(true))
        } else {
            step
        };
        let info = catalog.commands.iter().find(|c| c.name == step.command);
        if also.is_empty() && info.is_none_or(|i| missing(i, &step.args).is_empty()) {
            return Ok(Some(step));
        }
        self.ask(step, catalog, &also)?;
        Ok(None)
    }

    /// Answers the question: Save saves first (asking where when the scene has no file, the
    /// step it gives running now), Don't save goes on, Cancel stops.
    fn replied(&mut self, reply: Reply, catalog: &Catalog) -> Option<Step> {
        let then = self.unsaved.asking.take()?;
        self.set(false, None);
        let done = match reply {
            Reply::Cancel => return None,
            Reply::Discard => self.go_on(then, true, catalog),
            Reply::Save => {
                self.unsaved.saving = Some(then);
                let save = with_args("file.save", json!({}));
                match self.scene.own {
                    Some(_) => Ok(Some(save)),
                    None => self.ask(save, catalog, &["path".into()]).map(|()| None),
                }
            }
        };
        // Only a dialog already open refuses: the question goes, and asking again works.
        done.unwrap_or_else(|_| {
            self.unsaved.stop();
            None
        })
    }

    /// Draws the question for `then`, if it is asked: its line, then Save, Don't save and
    /// Cancel. Enter answers Save. Gives the step to run now, if any.
    pub(super) fn question(
        &mut self,
        ui: &mut egui::Ui,
        enter: bool,
        catalog: &Catalog,
        (now, rec): (&Now, &mut Recorder),
    ) -> Option<Step> {
        let then = self.unsaved.asking.as_ref()?;
        label(
            ui,
            RichText::new(line(then, self.scene.own.as_deref())),
            rec,
        );
        let close = with_args("ui.palette", json!({ "open": false }));
        let discard = match then {
            Then::Run(step, _) => dialog::given(step, "discard_changes", Value::Bool(true)),
            Then::Close => close.clone(),
        };
        let buttons = [
            ("Save", with_args("file.save", json!({})), Reply::Save),
            ("Don't save", discard, Reply::Discard),
            ("Cancel", close, Reply::Cancel),
        ];
        let mut reply = enter.then_some(Reply::Save);
        ui.horizontal(|ui| {
            for (name, step, answer) in buttons {
                // Save is Enter's, and says so (review 0009, R13).
                let press = Press {
                    label: name,
                    step,
                    key: matches!(answer, Reply::Save).then_some("Enter"),
                    lit: false,
                    shows: None,
                };
                // Don't save lets the changes go: it reads in the warning color.
                let drops = matches!(answer, Reply::Discard);
                if warned(ui, press, drops, now, rec).clicked() {
                    reply = Some(answer);
                }
            }
        });
        self.replied(reply?, catalog)
    }
}

/// The question's line: whose changes, and what would lose them.
fn line(then: &Then, own: Option<&str>) -> String {
    let name = own
        .and_then(|f| Path::new(f).file_name())
        .and_then(|n| n.to_str());
    let what = match then {
        Then::Close => "closing",
        Then::Run(step, _) => match step.command.as_str() {
            "file.new" => "starting a new scene",
            "file.open" => "opening another scene",
            "file.recover" => "recovering",
            _ => "going on",
        },
    };
    let scene = name.unwrap_or("this scene");
    format!("Save the changes to {scene} before {what}?")
}
