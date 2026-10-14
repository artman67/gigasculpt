//! `repro.save`.

use std::path::PathBuf;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Predicate, Source, Start, Step};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Draft, as_record, screenshot, unwritable, write};
use crate::errors::{args_invalid, error};
use crate::ui::WindowHost;
use crate::{Command, Ctx, Example, register_command};

/// Write a bug bundle: a start scene, every command since, and what "broken" means, as a
/// `.gsrepro` folder (decision 0031). The start is the newest autosave of the current log run,
/// as recovery would use, else the scene the run started from.
///
/// `gs repro check` replays it, `gs repro minimize` drops every command the bug does not need,
/// and once the bug is fixed `gs repro to-test` keeps it as a test. `then` adds calls after the
/// recorded ones: the call that failed or hangs, which the log never records, since only calls
/// that succeed are logged. With a window the bundle holds a screenshot too. It is written only
/// where `path` says, and nothing sends it anywhere.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReproSave {
    /// The bundle folder to write, ending in `.gsrepro`. It must not exist yet; its parent
    /// must. A relative path starts at the engine's working folder: give an absolute path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// What "broken" means, checked by replaying the bundle.
    pub predicate: Predicate,
    /// Calls to add after the recorded lines, each `{"command", "args"}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub then: Vec<Step>,
}

/// What `repro.save` wrote.
#[derive(Serialize, JsonSchema)]
pub struct SavedBundle {
    /// The bundle folder.
    pub path: String,
    /// How many command lines it holds.
    pub lines: u64,
    /// Where they start: `empty`, `file` (the bundle's `start.gss`) or `missing`.
    pub start: String,
    /// Why the start is missing, when it is: such a bundle cannot be replayed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
    /// True if it holds a screenshot of the window.
    pub screenshot: bool,
}

impl Command for ReproSave {
    const NAME: &'static str = "repro.save";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Short;
    type Output = SavedBundle;

    fn examples() -> Vec<Example<Self>> {
        let panics = Self {
            path: "/nonexistent/folder/bug.gsrepro".into(),
            predicate: Predicate::Panic {
                contains: "crashed on purpose".into(),
            },
            then: vec![Step {
                command: "debug.panic".into(),
                args: json!({}),
            }],
        };
        vec![
            Example::new(
                "bundle a sphere and a call that panics (refused here: the folder does not \
                 exist)",
                panics,
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::raw(
                "a bundle folder's name ends in .gsrepro",
                json!({
                    "path": "/tmp/bug",
                    "predicate": { "kind": "error", "code": "E_NOT_FOUND" },
                }),
            )
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<SavedBundle, CommandError> {
        let folder = PathBuf::from(&self.path);
        if folder.extension().is_none_or(|x| x != "gsrepro") {
            return Err(CommandError {
                hint: Some("name the folder like /some/where/bug.gsrepro".into()),
                ..args_invalid("/path", "a bug bundle's folder name ends in .gsrepro")
            });
        }
        if folder.exists() {
            return Err(CommandError {
                hint: Some("give a new name; a bundle is never written over".into()),
                ..error(
                    ErrorCode::Precondition,
                    format!("{} already exists", folder.display()),
                )
            });
        }
        let then = (self.then.into_iter())
            .map(|step| as_record(&step.command, step.args))
            .collect();
        let shot = screenshot(|| cx.window.with(WindowHost::screenshot));
        let has_shot = shot.is_some();
        let draft = Draft {
            recent: cx.log,
            at: cx.log_at,
            config: cx.config,
            file: cx.file,
            catalog: cx.catalog,
            then,
            predicate: self.predicate,
            source: Source::Save,
            screenshot: shot,
        };
        let info = write(&folder, draft).map_err(|e| unwritable(&folder, &e))?;
        let (start, missing) = match info.start {
            Start::Empty => ("empty", None),
            Start::File { .. } => ("file", None),
            Start::Missing { why } => ("missing", Some(why)),
        };
        Ok(SavedBundle {
            path: self.path,
            lines: info.lines,
            start: start.into(),
            missing,
            screenshot: has_shot,
        })
    }
}
register_command!(ReproSave, "Save bug bundle");
