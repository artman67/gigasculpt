//! The recovery dialog (decision 0016, section 11, as amended 2026-10-13): when the window starts
//! and a crashed session left work behind, a small dialog offers what `file.recovery` lists.
//! Recover, Autosave only and Later are one click. Discard, the one button in the window that
//! deletes files, first turns the dialog into a question, and only its Delete runs
//! `file.recovery`'s `discard`; Cancel goes back to the offer. Escape is Later on the offer and
//! Cancel on the question, while the dialog is in front (`shell/escape.rs`); Enter is the
//! offer's first choice and the question's Cancel, so a stray key never deletes. Each button
//! that runs something runs a command (`file.recover`, `file.recovery`'s `discard`), so an
//! agent can answer the same prompt. The offer goes once that command lands; while it waits (on
//! the question about unsaved changes, say) the dialog steps aside, and comes back if it is
//! cancelled or refused (review 0009, C10). Later puts every offer away until File > Recover…
//! or the next start. Discard and Delete read in the theme's warning color.

use egui::{Align2, Key, Modifiers, RichText};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::record::{Recorder, Role, Widget, with_args};
use crate::shell::escape::{To, entered};
use crate::theme::theme;

/// The dialog's egui window, which Escape and Enter order by (`shell/escape.rs`).
pub(crate) const WINDOW: &str = "recovery";

/// What Discard asks before the autosaves go.
const QUESTION: &str = "Delete the autosaves of the session that closed? This cannot be undone.";

/// A crashed session the window offers to recover.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The session's id, as `file.recovery` names it.
    pub id: String,
    /// What recovering brings back, in words.
    pub text: String,
    /// True if the log after the autosave replays bit for bit.
    pub replayable: bool,
}

/// The crashed sessions still to answer for, and Discard's question.
#[derive(Clone, Debug, Default)]
pub struct Recovery {
    pub offers: Vec<Offer>,
    /// Discard was pressed on the first offer: the dialog asks before its autosaves go.
    asking: bool,
    /// Later put the offers away, until File > Recover… ([`Recovery::reopen`]).
    later: bool,
    /// The command a button sent for the first offer, which answers it once it lands
    /// ([`Recovery::landed`]); while it waits, the dialog steps aside.
    sent: Option<String>,
}

/// The step File > Recover… runs: Recover's form, which shows the offer again while there is
/// one (`Shell::palette_request`).
pub fn row() -> Step {
    with_args("ui.palette", json!({ "form": "file.recover" }))
}

/// What a button of the dialog, or a key, answers.
enum Choice {
    /// Runs the step, which answers the offer.
    Run(Step),
    /// Asks before Discard deletes.
    Discard,
    /// Shuts the offer until the next start.
    Later,
    /// Leaves the question for the offer.
    Cancel,
}

impl Recovery {
    /// The offers in `file.recovery`'s result.
    pub fn from_result(result: &Value) -> Self {
        let offers = (result["candidates"].as_array().into_iter().flatten())
            .filter(|c| c["autosave"].is_string())
            .map(|c| Offer {
                id: c["id"].as_str().unwrap_or_default().to_string(),
                text: format!(
                    "Session {} closed without saving. Recover {}?",
                    c["session"].as_str().unwrap_or_default(),
                    c["offer"].as_str().unwrap_or_default()
                ),
                replayable: c["replayable"].as_bool().unwrap_or(false),
            })
            .collect();
        Self {
            offers,
            ..Self::default()
        }
    }

    /// Whether the dialog shows: an offer is open, not put away with Later and not waiting for
    /// the command a button sent.
    pub(crate) fn shows(&self) -> bool {
        !self.offers.is_empty() && !self.later && self.sent.is_none()
    }

    /// File > Recover…: the offers put away with Later show again.
    pub fn reopen(&mut self) {
        (self.later, self.asking) = (false, false);
    }

    /// `command` landed for the person: when it is the one a button sent, the first offer is
    /// answered and the next one shows, if any.
    pub fn landed(&mut self, command: &str) {
        if self.sent.as_deref() == Some(command) {
            self.sent = None;
            self.close();
        }
    }

    /// Ends a frame: when nothing `waits` with the command a button sent any more (it was
    /// refused, or the question it waited on was cancelled), the offer shows again, as it lost
    /// nothing.
    pub fn settle(&mut self, waits: impl Fn(&str) -> bool) {
        if self.sent.as_deref().is_some_and(|c| !waits(c)) {
            self.sent = None;
        }
    }

    /// Enter, given by the window (`shell/escape.rs`) while the dialog is in front, and taken
    /// before anything else in the frame sees it: on the offer its first choice (Recover, or
    /// Autosave only), never Discard; on the question Cancel. Returns the step the answer runs.
    pub fn keys(&mut self, ui: &egui::Ui) -> Option<Step> {
        if !entered(ui.ctx(), To::Recovery) {
            return None;
        }
        if !ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            return None;
        }
        let choice = if self.asking {
            Choice::Cancel
        } else {
            self.buttons().into_iter().next()?.1
        };
        self.answer(choice)
    }

    /// Escape, given by the window (`shell/escape.rs`) while the dialog is in front: Cancel on
    /// the question, Later on the offer. Returns the step the answer runs.
    pub(crate) fn escape(&mut self) -> Option<Step> {
        let choice = if self.asking {
            Choice::Cancel
        } else {
            Choice::Later
        };
        self.answer(choice)
    }

    /// Draws the dialog for the first offer, if there is one, in the middle of the window: the
    /// offer, or Discard's question in its place. Returns the step a click runs.
    pub fn show(&mut self, ui: &mut egui::Ui, rec: &mut Recorder) -> Option<Step> {
        let offer = self.offers.first().filter(|_| self.shows())?;
        let text = if self.asking {
            QUESTION
        } else {
            offer.text.as_str()
        }
        .to_string();
        let buttons = self.buttons();
        let warning = theme(ui.ctx()).colors.warning();
        let mut clicked = None;
        let shown = egui::Window::new("Recover your work")
            .id(egui::Id::new(WINDOW))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                rec.panel("recovery", "Recover your work", ui.max_rect());
                let said = ui.label(&text);
                rec.widget(Widget::plain(&text, Role::Label), said.rect);
                ui.horizontal(|ui| {
                    for (label, choice) in buttons {
                        let deletes = ["Discard", "Delete"].contains(&label);
                        let text = RichText::new(label);
                        let button = ui.button(if deletes { text.color(warning) } else { text });
                        rec.widget(widget(label, &choice), button.rect);
                        if button.clicked() {
                            clicked = Some(choice);
                        }
                    }
                });
            });
        if let Some(shown) = shown {
            rec.fit(shown.response.rect);
        }
        self.answer(clicked?)
    }

    /// The dialog's buttons for the first offer, in order: the question's while it asks,
    /// otherwise the offer's, safest first. None without an offer.
    fn buttons(&self) -> Vec<(&'static str, Choice)> {
        let Some(offer) = self.offers.first() else {
            return Vec::new();
        };
        let id = &offer.id;
        if self.asking {
            let discard = with_args("file.recovery", json!({ "discard": id }));
            return vec![("Delete", Choice::Run(discard)), ("Cancel", Choice::Cancel)];
        }
        let autosave = with_args("file.recover", json!({ "session": id, "replay": false }));
        let mut buttons = vec![
            ("Autosave only", Choice::Run(autosave)),
            ("Discard", Choice::Discard),
            ("Later", Choice::Later),
        ];
        if offer.replayable {
            let recover = with_args("file.recover", json!({ "session": id }));
            buttons.insert(0, ("Recover", Choice::Run(recover)));
        }
        buttons
    }

    /// Answers the first offer with `choice`. Returns the step it runs, which answers the
    /// offer once it lands.
    fn answer(&mut self, choice: Choice) -> Option<Step> {
        match choice {
            Choice::Discard => self.asking = true,
            Choice::Cancel => self.asking = false,
            Choice::Later => (self.later, self.asking) = (true, false),
            Choice::Run(step) => {
                self.asking = false;
                self.sent = Some(step.command.clone());
                return Some(step);
            }
        }
        None
    }

    /// The first offer is answered: the next one shows, if any.
    fn close(&mut self) {
        if !self.offers.is_empty() {
            self.offers.remove(0);
        }
        self.asking = false;
    }
}

/// A button as `ui.describe` lists it: with the command it runs, if it runs one.
fn widget(label: &str, choice: &Choice) -> Widget {
    match choice {
        Choice::Run(step) => Widget::runs(label, Role::Button, step.clone(), None),
        _ => Widget::plain(label, Role::Button),
    }
}
