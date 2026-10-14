//! `brush.save` and `brush.load`: a brush written into its preset file's text, and a saved
//! preset file read back as the brush's settings (decision 0018, "Saving a brush").

use std::io::Write;

use gs_core::BrushPreset;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::BrushState;
use super::patch::{self, Settings};
use super::presets::{self, Shipped};
use crate::{Command, Ctx, Example, errors, register_command};

/// Save a brush to a preset file: its preset with every setting changed for it in this
/// session, written into the text of the file it came from (the shipped preset, or the file
/// `brush.load` read it from), so comments and layout are kept and a brush with nothing
/// changed writes that file byte for byte. The alpha is saved by name. `brush.load` reads the
/// file back. Refuses to replace a file unless asked. Not logged: it changes nothing in the
/// session, and a replay never writes files.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushSave {
    /// The file to write, a `.toml` preset. Give an absolute path: a relative one starts at the
    /// engine's folder.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The brush, by name or id (`brush.list`); left out, the one the pen strokes with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    /// Replace a file already at `path`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overwrite: bool,
}

/// A brush preset file written.
#[derive(Serialize, JsonSchema)]
pub struct BrushFile {
    /// The brush, by name.
    pub brush: String,
    /// Where it was written.
    pub path: String,
    /// The BLAKE3 hash of the bytes written, as 64 hex digits: what `brush.load` logs.
    pub hash: String,
}

impl Command for BrushSave {
    const NAME: &'static str = "brush.save";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = BrushFile;

    fn examples() -> Vec<Example<Self>> {
        let save = |path: &str, brush: Option<&str>| Self {
            path: path.into(),
            brush: brush.map(Into::into),
            overwrite: false,
        };
        vec![
            Example::new(
                "save the brush the pen strokes with (refused: no such folder)",
                save("/nonexistent/my-brush.toml", None),
            )
            .fails_with(ErrorCode::Precondition),
            Example::new(
                "save Clay Build as it is set now (refused: no such folder)",
                save("/nonexistent/clay-build.toml", Some("Clay Build")),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<BrushFile, CommandError> {
        let settings = &cx.sculpt.settings;
        let shipped = match &self.brush {
            Some(brush) => presets::find(brush).map_err(|e| at("/brush", e))?,
            None => presets::find(settings.in_effect())?,
        };
        let name = shipped.preset.name.clone();
        let changed = settings.patch_of(&name);
        let brush = patch::resolve(shipped, &changed, &[])?;
        let source = settings
            .sources
            .get(&name)
            .map_or(&*shipped.text, String::as_str);
        // A size curve the settings name is written even at its default: naming it keeps the
        // session's global one off the brush (decision 0036).
        let names_size = patch::get(&changed, &["pressure", "size"]).is_some();
        let named: &[&str] = if names_size { &["pressure.size"] } else { &[] };
        let text = (brush.write_into(source, named)).map_err(|e| errors::internal(&e.message))?;
        let write = |file: &mut std::fs::File| file.write_all(text.as_bytes());
        // A file already there asks for `overwrite`, so the window asks before replacing it.
        let exists = !self.overwrite && std::path::Path::new(&self.path).is_file();
        let asks = if exists { "/overwrite" } else { "/path" };
        crate::file::save_file(&self.path, self.overwrite, write).map_err(|e| at(asks, e))?;
        Ok(BrushFile {
            brush: name,
            path: self.path,
            hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
        })
    }
}
register_command!(BrushSave, "Save brush");

/// Load a brush from a preset file `brush.save` wrote: the shipped brush the file names by its
/// `id` becomes the brush the pen strokes with (on the held slot while one is held, as
/// `brush.set` chooses), and the settings the file changes in it replace those changed in this
/// session. A later `brush.save` writes into this file's text, so its comments stay. A file
/// must keep its brush's name, family, claims and kernel type and leave no table out; a file
/// in a later format is refused by its version. Its content hash is logged, so a replay
/// refuses a changed file. Answers with every setting, as `brush.set` does.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushLoad {
    /// The preset file. Give an absolute path: a relative one starts at the engine's folder.
    #[schemars(length(min = 1))]
    pub path: String,
    /// The BLAKE3 hash of the file's bytes, as 64 hex digits. Filled in by the engine before it
    /// runs, so the log records which content was read; a call whose hash no longer matches the
    /// file is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl Command for BrushLoad {
    const NAME: &'static str = "brush.load";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const READS_FILE: Option<&'static str> = Some("path");
    type Output = BrushState;

    fn examples() -> Vec<Example<Self>> {
        let load = |hash: Option<String>| Self {
            path: "/nonexistent/my-brush.toml".into(),
            hash,
        };
        let saved = "the file brush.save wrote, by the hash it answered (refused: no such file)";
        vec![
            Example::new("load a saved brush (refused: no such file)", load(None))
                .fails_with(ErrorCode::NotFound),
            Example::new(saved, load(Some("0".repeat(64)))).fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<BrushState, CommandError> {
        let path = &self.path;
        // A folder asks for a file name, as Open does, not the system's "Is a directory".
        crate::file::not_a_folder(std::path::Path::new(path))?;
        let text = std::fs::read_to_string(path)
            .map_err(|e| at("/path", crate::file::io_error(path, &e)))?;
        let refused = |m: String| patch::refused("/path", format!("{path}: {m}"));
        let (shipped, changed, brush) = settings_in(&text).map_err(refused)?;
        let settings = &mut cx.sculpt.settings;
        if let Some(alpha) = &brush.tip.alpha {
            settings.recent.used(&alpha.image);
        }
        settings.load(shipped.preset.name.clone(), changed, text);
        Ok(settings.state())
    }
}
register_command!(BrushLoad, "Load brush");

/// `e` as the refusal of the argument at JSON pointer `path`.
fn at(path: &str, e: CommandError) -> CommandError {
    CommandError {
        path: Some(path.into()),
        ..e
    }
}

/// The shipped brush a saved preset file's `text` names, the settings the file changes in it
/// (as `brush.set` would give them), and the brush they make; why not, if the file is not a
/// shipped brush with settings changed.
fn settings_in(text: &str) -> Result<(&'static Shipped, Settings, BrushPreset), String> {
    let loaded = BrushPreset::from_toml(text).map_err(|e| e.message)?;
    let shipped = (presets::all().iter())
        .find(|s| s.preset.id == loaded.id)
        .ok_or_else(|| {
            let id = &loaded.id;
            format!("{id} is not a shipped brush: a saved preset changes one of brush.list's")
        })?;
    // A table the shipped brush lacks brings only the settings its defaults do not mean.
    let mut changes = shipped.preset.changes(&loaded);
    changes.trim(|c| BrushPreset::resolve(&shipped.text, &c.set).is_ok_and(|p| p == loaded));
    let json = serde_json::to_value(&changes.set).map_err(|e| e.to_string())?;
    let mut changed = json.as_object().cloned().unwrap_or_default();
    // A size curve the file names keeps the session's global one off the brush (decision
    // 0036), even at its default, so the settings name it too unless the shipped file does.
    let file: toml::Table = toml::from_str(text).unwrap_or_default();
    let names_size = file.get("pressure").and_then(|p| p.get("size")).is_some();
    if names_size
        && !shipped.names("pressure", "size")
        && patch::get(&changed, &["pressure", "size"]).is_none()
    {
        let canonical: toml::Table = toml::from_str(&loaded.canonical()).unwrap_or_default();
        let size = canonical.get("pressure").and_then(|p| p.get("size"));
        let size = serde_json::to_value(size).map_err(|e| e.to_string())?;
        patch::put(&mut changed, &["pressure", "size"], size);
    }
    // Name, family, claims and kernel type cannot change: resolving refuses them by name.
    let brush = patch::resolve(shipped, &changed, &[]).map_err(|e| e.message)?;
    // A brush whose stroke changed class leaves out the old class's settings (decision 0051).
    let restroked = loaded.stroke.class() != shipped.preset.stroke.class();
    let gone = (changes.removed.iter()).find(|k| !(restroked && k.starts_with("stroke.")));
    if let Some(gone) = gone {
        let name = &shipped.preset.name;
        return Err(format!(
            "{name} has {gone} and the file leaves it out: a saved brush changes settings, \
             never takes them out"
        ));
    }
    if brush != loaded {
        let name = &shipped.preset.name;
        return Err(format!(
            "the file does not read as {name} with settings changed"
        ));
    }
    if let Some(alpha) = &brush.tip.alpha {
        crate::alpha::pyramid(alpha).map_err(|e| e.message)?;
    }
    Ok((shipped, changed, brush))
}
