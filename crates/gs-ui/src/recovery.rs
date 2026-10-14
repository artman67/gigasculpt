//! The recovery prompt (decision 0016, section 11): when the window starts and a crashed session
//! left work behind, a small dialog offers what `file.recovery` lists. Each button is a command
//! (`file.recover`, `file.recovery`'s `discard`), so an agent can answer the same prompt; Later
//! closes it until the next start.

use egui::Align2;
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::record::{Recorder, Role, Widget, with_args};

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

/// The crashed sessions still to answer for.
#[derive(Clone, Debug, Default)]
pub struct Recovery {
    pub offers: Vec<Offer>,
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
        Self { offers }
    }

    /// Draws the prompt for the first offer, if there is one. Returns the command a button
    /// asks for; any answer closes it.
    pub fn show(&mut self, ui: &mut egui::Ui, rec: &mut Recorder) -> Option<Step> {
        let offer = self.offers.first()?.clone();
        let mut asked = None;
        let mut closed = false;
        egui::Window::new("Recover your work")
            .id(egui::Id::new("recovery"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                rec.panel("recovery", "Recover your work", ui.max_rect());
                ui.label(&offer.text);
                ui.horizontal(|ui| {
                    let session = json!({ "session": offer.id });
                    let autosave = json!({ "session": offer.id, "replay": false });
                    let discard = json!({ "discard": offer.id });
                    let mut buttons = vec![
                        ("Autosave only", with_args("file.recover", autosave)),
                        ("Discard", with_args("file.recovery", discard)),
                    ];
                    if offer.replayable {
                        buttons.insert(0, ("Recover", with_args("file.recover", session)));
                    }
                    for (label, step) in buttons {
                        let clicked = ui.button(label);
                        rec.widget(
                            Widget::runs(label, Role::Button, step.clone(), None),
                            clicked.rect,
                        );
                        if clicked.clicked() {
                            asked = Some(step);
                        }
                    }
                    let later = ui.button("Later");
                    rec.widget(Widget::plain("Later", Role::Button), later.rect);
                    closed = later.clicked();
                });
            });
        if asked.is_some() || closed {
            self.offers.remove(0);
        }
        asked
    }
}
