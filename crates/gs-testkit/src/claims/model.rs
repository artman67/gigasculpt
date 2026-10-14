//! The history model: a plain list of full scene states with a cursor, the dumbest possible
//! undo history. The runner feeds it every call and its outcome and compares the engine's
//! scene after every call. Research 06 (section 5) found a "new edit forgets to clear redo"
//! bug that 18 other tests missed and only this kind of model caught.

use std::collections::BTreeMap;

use serde_json::Value;

/// What kind of call a step was, as the model sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call<'a> {
    /// Any edit command except `history.restore`.
    Edit,
    /// `history.undo`.
    Undo,
    /// `history.redo`.
    Redo,
    /// `history.checkpoint` with this name, replacing any old one.
    Checkpoint(&'a str),
    /// `history.restore` of this checkpoint.
    Restore(&'a str),
}

impl<'a> Call<'a> {
    /// The kind of call a command with these arguments is.
    pub fn of(command: &str, args: &'a Value) -> Self {
        let name = args["name"].as_str().unwrap_or_default();
        match command {
            "history.undo" => Self::Undo,
            "history.redo" => Self::Redo,
            "history.checkpoint" => Self::Checkpoint(name),
            "history.restore" => Self::Restore(name),
            _ => Self::Edit,
        }
    }
}

/// Every state the document has been in that undo and redo can reach, as canonical scene
/// bytes, and the saved checkpoints.
#[derive(Clone, Debug)]
pub struct Model {
    states: Vec<Vec<u8>>,
    at: usize,
    saves: BTreeMap<String, Vec<u8>>,
}

impl Model {
    /// A model whose only state is `start`.
    pub fn new(start: Vec<u8>) -> Self {
        Self {
            states: vec![start],
            at: 0,
            saves: BTreeMap::new(),
        }
    }

    /// Steps that can be undone.
    pub fn undoable(&self) -> usize {
        self.at
    }

    /// Steps that can be redone.
    pub fn redoable(&self) -> usize {
        self.states.len() - 1 - self.at
    }

    /// Checks one call against the model and moves the model along. `ok` says whether the
    /// call succeeded, `made_step` whether it reported a new undo step, `after` is the scene
    /// afterwards.
    pub fn step(
        &mut self,
        call: Call,
        ok: bool,
        made_step: bool,
        after: Vec<u8>,
    ) -> Result<(), String> {
        let now = self.states[self.at].clone();
        match call {
            Call::Undo | Call::Redo => self.travel(matches!(call, Call::Redo), ok, &after),
            Call::Checkpoint(name) => {
                self.saves.insert(name.to_string(), now.clone());
                expect(
                    ok && after == now,
                    "a checkpoint must succeed and change nothing",
                )
            }
            _ if !ok => expect(after == now, "a failed call changed the scene"),
            Call::Restore(name) => match self.saves.get(name).cloned() {
                None => Err(format!(
                    "restoring '{name}', which was never saved, succeeded"
                )),
                Some(saved) if saved != after => {
                    Err(format!("restoring '{name}' did not bring it back"))
                }
                Some(_) => self.edit(made_step, &now, after),
            },
            Call::Edit => self.edit(made_step, &now, after),
        }
    }

    fn travel(&mut self, forward: bool, ok: bool, after: &[u8]) -> Result<(), String> {
        let target = if forward {
            self.at + 1
        } else {
            self.at.wrapping_sub(1)
        };
        let Some(state) = self.states.get(target) else {
            return expect(
                !ok,
                "undo or redo succeeded with nothing to go back or forward to",
            );
        };
        expect(ok, "undo or redo failed with a step to go to")?;
        expect(
            state == after,
            "undo or redo did not bring back the recorded state",
        )?;
        self.at = target;
        Ok(())
    }

    fn edit(&mut self, made_step: bool, now: &[u8], after: Vec<u8>) -> Result<(), String> {
        if !made_step {
            return expect(
                after == now,
                "an edit changed the scene without making an undo step",
            );
        }
        expect(
            after != now,
            "an edit made an undo step without changing anything",
        )?;
        self.states.truncate(self.at + 1);
        self.states.push(after);
        self.at += 1;
        Ok(())
    }
}

fn expect(holds: bool, otherwise: &str) -> Result<(), String> {
    if holds {
        Ok(())
    } else {
        Err(otherwise.to_string())
    }
}
