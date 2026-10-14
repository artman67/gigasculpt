//! `history.undo` and `history.redo`: one implementation serving both commands.

use std::marker::PhantomData;

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Which way a `Travel` command moves through the history.
pub trait Direction: Send + Sized + 'static {
    const NAME: &'static str;
    const DOCS: &'static str;
    /// True to redo, false to undo.
    const FORWARD: bool;
    fn examples() -> Vec<Example<Travel<Self>>>;
}

/// `history.undo`.
pub struct Back;

/// `history.redo`.
pub struct Forward;

impl Direction for Back {
    const NAME: &'static str = "history.undo";
    const DOCS: &'static str = "Undo the newest step.\n\nA human undoes the newest step whoever \
        made it. An agent's plain undo only undoes a step made under its own client name, and \
        otherwise refuses with `E_STALE` naming the step, so it never undoes a human's stroke. \
        Pass `expect` (the step id from the edit's `history_id` or from `history.list`) to undo \
        exactly that step and refuse with `E_STALE` if the newest step is a different one.";
    const FORWARD: bool = false;

    fn examples() -> Vec<Example<Travel<Self>>> {
        vec![
            Example::new("undo adding a sphere", Travel::default())
                .after("primitive.sphere", json!({})),
            Example::new(
                "undo step 2, the move",
                Travel {
                    expect: Some(2),
                    direction: PhantomData,
                },
            )
            .after("primitive.sphere", json!({}))
            .after(
                "mesh.translate",
                json!({ "direction": [0, 1, 0], "distance": 1 }),
            ),
        ]
    }
}

impl Direction for Forward {
    const NAME: &'static str = "history.redo";
    const DOCS: &'static str = "Redo the step undone last.\n\nThe same rules as `history.undo` \
        decide whose step may be redone. Pass `expect` (the step id) to refuse with `E_STALE` if \
        the step to redo is a different one. Any new edit clears what could be redone.";
    const FORWARD: bool = true;

    fn examples() -> Vec<Example<Travel<Self>>> {
        vec![
            Example::new("redo adding a cube", Travel::default())
                .after("primitive.cube", json!({}))
                .after("history.undo", json!({})),
        ]
    }
}

/// Move one step through the undo history.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, bound = "")]
#[schemars(bound = "")]
pub struct Travel<D: Direction> {
    /// Id of the step you mean. Refuses with `E_STALE` if the step is a different one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<u64>,
    #[serde(skip)]
    #[schemars(skip)]
    direction: PhantomData<D>,
}

impl<D: Direction> Default for Travel<D> {
    fn default() -> Self {
        Self {
            expect: None,
            direction: PhantomData,
        }
    }
}

/// The step that was undone or redone.
#[derive(Serialize, JsonSchema)]
pub struct Stepped {
    /// Its id.
    pub step: u64,
    /// The command that made it.
    pub command: String,
}

impl<D: Direction> Command for Travel<D> {
    const NAME: &'static str = D::NAME;
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Instant;
    const DOCS: &'static str = D::DOCS;
    type Output = Stepped;

    fn examples() -> Vec<Example<Self>> {
        D::examples()
    }

    /// Unavailable with no step to travel to, so the window draws Undo and Redo disabled then
    /// (decision 0016, amended). `run` refuses the same way; this says so before it runs.
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        let ends = cx.history.ends();
        let (next, action) = if D::FORWARD {
            (ends.redo, "redo")
        } else {
            (ends.undo, "undo")
        };
        match next {
            Some(_) => Ok(()),
            None => Err(Unavailable::new(format!("there is nothing to {action}"))
                .fix("run history.list to see the steps")),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<Stepped, CommandError> {
        let doc = cx.doc.document_mut();
        let history = cx.history.session()?;
        let entry = if D::FORWARD {
            history.redo(doc, self.expect, cx.origin)?
        } else {
            history.undo(doc, self.expect, cx.origin)?
        };
        let stepped = Stepped {
            step: entry.id,
            command: entry.command.clone(),
        };
        cx.doc_replaced = true;
        Ok(stepped)
    }
}
register_command!(Travel<Back>, "Undo");
register_command!(Travel<Forward>, "Redo");
