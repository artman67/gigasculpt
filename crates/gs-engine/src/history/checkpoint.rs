//! `history.checkpoint` and `history.restore`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::errors::args_invalid;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Save the whole scene under a name, to come back to with `history.restore`.
///
/// Checkpoints are never dropped to save memory. They share the scene's data, so they cost
/// nothing until the scene changes, and then only what changed. Use one before an experiment.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryCheckpoint {
    /// Name of the checkpoint, without a `/`: `mesh.diff` reads `checkpoint:NAME/PART` up to
    /// the first one.
    #[schemars(length(min = 1))]
    pub name: String,
    /// Replace an existing checkpoint with this name.
    #[serde(default)]
    pub replace: bool,
}

/// A saved checkpoint.
#[derive(Serialize, JsonSchema)]
pub struct Saved {
    /// Its name.
    pub name: String,
    /// The sequence number of the call that saved it.
    pub seq: u64,
}

impl Command for HistoryCheckpoint {
    const NAME: &'static str = "history.checkpoint";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Instant;
    type Output = Saved;

    fn examples() -> Vec<Example<Self>> {
        let name = "base".into();
        vec![
            Example::new(
                "save the scene as base",
                Self {
                    name,
                    replace: false,
                },
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        if self.replace || cx.history.checkpoint_named(&self.name).is_err() {
            return Ok(());
        }
        Err(
            Unavailable::new(format!("a checkpoint named '{}' already exists", self.name))
                .fix("pass replace=true to overwrite it, or pick another name"),
        )
    }

    fn run(self, cx: &mut Ctx) -> Result<Saved, CommandError> {
        // Review 0007, m2: `mesh.diff` could not reach a checkpoint named "a/b".
        if self.name.contains('/') {
            return Err(args_invalid(
                "/name",
                format!(
                    "a checkpoint name cannot hold a '/': mesh.diff reads \
                     checkpoint:NAME/PART up to the first one, so '{}' could not be compared",
                    self.name
                ),
            ));
        }
        let history = cx.history.session()?;
        history.checkpoint(&self.name, cx.seq, cx.doc.document());
        Ok(Saved {
            name: self.name,
            seq: cx.seq,
        })
    }
}
register_command!(HistoryCheckpoint, "Save checkpoint");

/// Bring the whole scene back to a checkpoint.
///
/// This is an ordinary edit: one undo step, so `history.undo` returns to the state before the
/// restore. Each part comes back exactly as saved, with its vertex numbering and what it
/// remembers (the cell size Voxel Remesh keeps, the scale and offset of the file it came from).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryRestore {
    /// Name of the checkpoint.
    pub name: String,
}

/// The checkpoint that was restored.
#[derive(Serialize, JsonSchema)]
pub struct Restored {
    /// Its name.
    pub name: String,
}

impl Command for HistoryRestore {
    const NAME: &'static str = "history.restore";
    const KIND: Kind = Kind::Edit;
    type Output = Restored;

    fn examples() -> Vec<Example<Self>> {
        let name = "base".into();
        vec![
            Example::new("go back to before a move and a new part", Self { name })
                .after("primitive.sphere", json!({}))
                .after("history.checkpoint", json!({ "name": "base" }))
                .after(
                    "mesh.translate",
                    json!({ "direction": [1, 0, 0], "distance": 2 }),
                )
                .after("primitive.cube", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Restored, CommandError> {
        let saved = &cx.history.checkpoint_named(&self.name)?.parts;
        let current: Vec<String> = cx.doc.part_names().map(str::to_string).collect();
        for name in current.iter().filter(|name| !saved.contains_key(*name)) {
            cx.doc.remove_part(name);
        }
        for (name, part) in saved {
            cx.doc.restore_part(name, part);
        }
        Ok(Restored { name: self.name })
    }
}
register_command!(HistoryRestore, "Restore checkpoint");
