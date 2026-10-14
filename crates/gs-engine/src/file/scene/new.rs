//! `file.new` (decision 0016, section 16): an empty scene, landed as an open is.

use std::collections::BTreeMap;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Landing, Opened};
use crate::document::Document;
use crate::view::saved::reopen;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Replace the scene with an empty one that has no file, in the background.
///
/// Refused while the document has unsaved changes, unless `discard_changes` is on, as
/// `file.open` is. It lands as an open of an empty scene: the history empties, the Reference
/// Views close and the command log starts a new run with this call. The session seed and
/// command counter go on, and so does the material palette, with its undo and any palette
/// steps no file holds yet, so those are not asked about. The empty document counts as saved,
/// so the next open needs no `discard_changes` unless the palette has such steps, and its
/// first save needs a path.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileNew {
    /// Start a new scene even though this one has unsaved changes, which are lost.
    #[serde(default)]
    pub discard_changes: bool,
}

/// The new scene.
#[derive(Serialize, JsonSchema)]
pub struct NewScene {
    /// Its name until its first save: `untitled`.
    pub name: String,
    /// True if unsaved changes were let go.
    pub discarded: bool,
}

impl Command for FileNew {
    const NAME: &'static str = "file.new";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Short;
    const BACKGROUND: bool = true;
    type Output = NewScene;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("start a new scene", Self::default()),
            Example::new(
                "start a new scene with changes unsaved (refused)",
                Self::default(),
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::new(
                "start a new scene, letting unsaved changes go",
                Self {
                    discard_changes: true,
                },
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    /// Asks only about the document's changes: the palette and its unsaved steps stay.
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        super::ask_first(cx.file.edited(cx.rev), self.discard_changes)
    }

    fn run(self, cx: &mut Ctx) -> Result<NewScene, CommandError> {
        let (references, _) = reopen(cx, None, &mut BTreeMap::new(), &mut Vec::new());
        let discarded = self.discard_changes && cx.file.edited(cx.rev);
        cx.land(Landing::Opened(Box::new(Opened {
            doc: Document::default(),
            path: None,
            clean: true,
            forget: None,
            session: None,
            id: None,
            format: None,
            rev: cx.rev,
            discard: self.discard_changes,
            replay: Vec::new(),
            log_as: None,
            references,
            recent: None,
            background: None,
            palette: None,
        })));
        Ok(NewScene {
            name: "untitled".into(),
            discarded,
        })
    }
}
register_command!(FileNew, "New scene");
