//! `curve.save` and `curve.load`: a curve kept in a file of its own, as the curve editor's Save
//! and Load buttons keep one (decision 0048, section 5). The file is TOML with one key,
//! `curve`, in the form preset files write curves, so every point's bits, kinds and settings
//! come back exactly. Neither is logged: a curve loaded goes into the session through the
//! command that sets it, which is.

use std::io::Write;

use gs_core::curve::Curve;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Command, Ctx, Example, errors, register_command};

/// The schema of a curve: a list of `[x, y]` points (all sharp), or a table with `points`,
/// `smooth`, `sharp` and the settings `steps`, `step_strength` and `tile`.
pub(crate) fn curve_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let points = json!({
        "type": "array",
        "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }
    });
    let table = json!({
        "type": "object",
        "properties": {
            "points": points,
            "smooth": { "type": "boolean" },
            "sharp": { "type": "array", "items": { "type": "integer", "minimum": 0 } },
            "steps": { "type": "integer", "minimum": 1 },
            "step_strength": { "type": "number" },
            "tile": { "type": "integer", "minimum": 1 }
        },
        "required": ["points"],
        "additionalProperties": false
    });
    schemars::json_schema!({ "anyOf": [points, table] })
}

/// A curve file's content, and `curve.load`'s answer.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurveFile {
    /// The curve: its points, their kinds and its settings.
    #[schemars(schema_with = "curve_schema")]
    pub curve: Curve,
}

/// Save a curve to a file: its points, their kinds and its settings, in the form a preset file
/// writes a curve, under the one key `curve`. `curve.load` reads it back exactly. Refuses to
/// replace a file unless asked. Not logged: it changes nothing in the session.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurveSave {
    /// The file to write, a `.toml`. Give an absolute path: a relative one starts at the
    /// engine's folder.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The curve, as `brush.set` takes one: `[[0, 0], [1, 1]]`, or `{"points": [...],
    /// "smooth": true, "sharp": [2], "steps": 4, "step_strength": 0.5, "tile": 2}`.
    #[schemars(schema_with = "curve_schema")]
    pub curve: Curve,
    /// Replace a file already at `path`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overwrite: bool,
}

/// A curve file written.
#[derive(Serialize, JsonSchema)]
pub struct CurveSaved {
    /// Where it was written.
    pub path: String,
}

impl Command for CurveSave {
    const NAME: &'static str = "curve.save";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = CurveSaved;

    fn examples() -> Vec<Example<Self>> {
        let soft = json!({ "points": [[0, 0], [0.5, 0.8], [1, 1]], "smooth": true });
        vec![
            Example::raw(
                "save a soft pressure curve (refused: no such folder)",
                json!({ "path": "/nonexistent/soft.toml", "curve": soft }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, _: &mut Ctx) -> Result<CurveSaved, CommandError> {
        let file = CurveFile { curve: self.curve };
        let text = toml::to_string(&file).map_err(|e| errors::internal(&e.to_string()))?;
        let write = |f: &mut std::fs::File| f.write_all(text.as_bytes());
        crate::file::save_file(&self.path, self.overwrite, write).map_err(at_path)?;
        Ok(CurveSaved { path: self.path })
    }
}
register_command!(CurveSave, "Save curve");

/// Load a curve from a file `curve.save` wrote. Answers with the curve, exactly as written,
/// for the command that sets it (`brush.set`'s pressure curves or the brush's falloff).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurveLoad {
    /// The curve file. Give an absolute path: a relative one starts at the engine's folder.
    #[schemars(length(min = 1))]
    pub path: String,
}

impl Command for CurveLoad {
    const NAME: &'static str = "curve.load";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = CurveFile;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "load a saved curve (refused: no such file)",
                json!({ "path": "/nonexistent/soft.toml" }),
            )
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, _: &mut Ctx) -> Result<CurveFile, CommandError> {
        let path = &self.path;
        let text =
            std::fs::read_to_string(path).map_err(|e| at_path(crate::file::io_error(path, &e)))?;
        toml::from_str(&text)
            .map_err(|e| errors::args_invalid("/path", format!("{path} is not a curve file: {e}")))
    }
}
register_command!(CurveLoad, "Load curve");

/// `e` as the refusal of the argument `path`.
fn at_path(e: CommandError) -> CommandError {
    CommandError {
        path: Some("/path".into()),
        ..e
    }
}
