//! The floor grid's style (catalog item `canvas.floor-grid`): `view.floor` sets it or loads a
//! preset, and `view.floor_save` writes the preset. Whether the floor shows is `view.display`'s
//! `floor`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::{args_invalid, error};
use crate::{Command, Ctx, Example, register_command};

/// How the floor grid is drawn while it shows.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one switch per plane and one for snap, as the floor's settings are named"
)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FloorGrid {
    /// The plane square to x (red).
    pub x: bool,
    /// The plane square to y, the floor itself (green).
    pub y: bool,
    /// The plane square to z (blue).
    pub z: bool,
    /// Where each plane sits along its axis, -1 to 1: -1 just past the low side of the bounds
    /// of everything drawn, 0 through their center, 1 just past the high side.
    pub elevation: f32,
    /// Each plane sits just past the side of the bounds away from the eye, always behind what
    /// is drawn; `elevation` is not used.
    pub snap: bool,
    /// 0: endless, lines at powers of ten of the scene unit. Otherwise a square this many
    /// times the largest side of the bounds, centered on them.
    pub size: f32,
    /// Cells along a side of a sized grid.
    pub tiles: u32,
    /// 0 opaque, 1 slightly see-through, 2 strongly see-through, 3 slightly see-through with
    /// the parts see-through where they face the eye.
    pub fill: u32,
    /// How strongly each plane's lines take its axis color, 0 to 1.
    pub tint: f32,
}

impl Default for FloorGrid {
    /// The floor decision 0023 shows: the y plane at -1, endless, opaque, 16 tiles, untinted
    /// (gs-render's `FloorGrid::default()` draws the same).
    fn default() -> Self {
        FloorGrid {
            x: false,
            y: true,
            z: false,
            elevation: -1.0,
            snap: false,
            size: 0.0,
            tiles: 16,
            fill: 0,
            tint: 0.0,
        }
    }
}

impl FloorGrid {
    /// Its values by the names `session.state` gives them.
    pub(crate) fn values(&self) -> [(&'static str, Value); 9] {
        [
            ("view.floor.x", json!(self.x)),
            ("view.floor.y", json!(self.y)),
            ("view.floor.z", json!(self.z)),
            ("view.floor.elevation", json!(self.elevation)),
            ("view.floor.snap", json!(self.snap)),
            ("view.floor.size", json!(self.size)),
            ("view.floor.tiles", json!(self.tiles)),
            ("view.floor.fill", json!(self.fill)),
            ("view.floor.tint", json!(self.tint)),
        ]
    }
}

/// The whole floor setup, as a preset file holds it: whether it shows and how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FloorPreset {
    pub shown: bool,
    pub grid: FloorGrid,
}

/// Set how the floor grid is drawn, or load a floor preset. Arguments left out keep their
/// value; with `preset`, the file replaces the whole floor setup (whether it shows too) and the
/// other arguments then change it.
///
/// The planes are placed from the bounds of everything a view draws, so Isolate or a hidden
/// part moves them. `view.display`'s `floor` shows or hides the floor; snapshots draw it only
/// when they pass `grid`, in this style. A display setting of the session: never undone, never
/// saved with the scene. A preset file is read now and its content hash is logged, so a replay
/// refuses a changed file.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewFloor {
    /// A floor preset file (TOML, as `view.floor_save` writes it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub preset: Option<String>,
    /// The plane square to x (red). Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<bool>,
    /// The plane square to y, the floor itself (green). Starts on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<bool>,
    /// The plane square to z (blue). Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<bool>,
    /// Where the planes sit, -1 to 1: -1 just past the low side of everything drawn (the
    /// start), 0 through its center, 1 just past the high side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -1.0, max = 1.0))]
    pub elevation: Option<f32>,
    /// Each plane just past the far side of everything drawn, so it is always behind it.
    /// Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<bool>,
    /// 0 (the start): endless, lines at powers of ten of the scene unit. Otherwise a square
    /// this many times the largest side of everything drawn, up to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 100.0))]
    pub size: Option<f32>,
    /// Cells along a side of a sized grid, 1 to 1000, rounded to a whole number. Starts at 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 1000.0))]
    pub tiles: Option<f32>,
    /// 0 opaque (the start), 1 slightly see-through, 2 strongly see-through, 3 slightly
    /// see-through with the parts see-through where they face the eye.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 3))]
    pub fill: Option<u32>,
    /// How strongly each plane's lines take its axis color, 0 (the start) to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub tint: Option<f32>,
    /// The BLAKE3 hash of the preset file's bytes, as 64 hex digits. Filled in by the engine
    /// before it runs, so the log records which content was read; a call whose hash no longer
    /// matches the file is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Reads the floor preset at `path`.
fn read(path: &str, hashed: bool) -> Result<FloorPreset, CommandError> {
    let missing = || CommandError {
        path: Some("/preset".into()),
        ..error(
            ErrorCode::NotFound,
            format!("cannot read {path}: there is no such file"),
        )
    };
    if !hashed {
        return Err(missing());
    }
    let text = std::fs::read_to_string(path).map_err(|_| missing())?;
    toml::from_str(&text)
        .map_err(|e| args_invalid("/preset", format!("{path} is not a floor preset: {e}")))
}

impl Command for ViewFloor {
    const NAME: &'static str = "view.floor";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const READS_FILE: Option<&'static str> = Some("preset");
    type Output = FloorPreset;

    fn examples() -> Vec<Example<Self>> {
        let walls = Self {
            x: Some(true),
            z: Some(true),
            tint: Some(1.0),
            ..Self::default()
        };
        let through = Self {
            elevation: Some(0.0),
            ..Self::default()
        };
        let behind = Self {
            snap: Some(true),
            ..Self::default()
        };
        let tiled = Self {
            size: Some(2.0),
            tiles: Some(8.0),
            ..Self::default()
        };
        let fresnel = Self {
            fill: Some(3),
            ..Self::default()
        };
        let preset = Self {
            preset: Some("/nonexistent/side-walls.toml".into()),
            ..Self::default()
        };
        vec![
            Example::new("add the x and z planes, each line in its axis color", walls),
            Example::new("the floor through the middle of the parts", through),
            Example::new("every plane behind the parts, from any side", behind),
            Example::new("a square twice the parts' size with 8 cells a side", tiled),
            Example::new("see through the parts where they face you", fresnel),
            Example::new("load a floor preset (refused: no such file)", preset)
                .fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<FloorPreset, CommandError> {
        let d = &mut *cx.display;
        if let Some(path) = &self.preset {
            let p = read(path, self.hash.is_some())?;
            d.floor = p.shown;
            d.floor_grid = p.grid;
        }
        let g = &mut d.floor_grid;
        g.x = self.x.unwrap_or(g.x);
        g.y = self.y.unwrap_or(g.y);
        g.z = self.z.unwrap_or(g.z);
        g.elevation = self.elevation.unwrap_or(g.elevation);
        g.snap = self.snap.unwrap_or(g.snap);
        g.size = self.size.unwrap_or(g.size);
        g.tiles = self.tiles.map_or(g.tiles, |t| t.round() as u32);
        g.fill = self.fill.unwrap_or(g.fill);
        g.tint = self.tint.unwrap_or(g.tint);
        Ok(FloorPreset {
            shown: d.floor,
            grid: d.floor_grid,
        })
    }
}
register_command!(ViewFloor, "Set floor grid");

/// Write the floor setup (whether it shows and how it is drawn) as a preset file that
/// `view.floor`'s `preset` loads in any scene. The file is TOML.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewFloorSave {
    /// The file to write. Its folder must exist. A relative path starts at the engine's working
    /// folder: give an absolute path.
    #[schemars(length(min = 1))]
    pub path: String,
    /// Replace a file already there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overwrite: bool,
}

impl Command for ViewFloorSave {
    const NAME: &'static str = "view.floor_save";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = FloorPreset;

    fn examples() -> Vec<Example<Self>> {
        let call = Self {
            path: "/nonexistent/folder/floor.toml".into(),
            overwrite: false,
        };
        vec![
            Example::new(
                "save the floor as a preset (refused here: the folder does not exist)",
                call,
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<FloorPreset, CommandError> {
        let preset = FloorPreset {
            shown: cx.display.floor,
            grid: cx.display.floor_grid,
        };
        let path = std::path::Path::new(&self.path);
        if path.exists() && !self.overwrite {
            let message = format!("{} exists; pass overwrite to replace it", self.path);
            return Err(error(ErrorCode::Precondition, message));
        }
        let body =
            toml::to_string(&preset).map_err(|e| error(ErrorCode::Internal, e.to_string()))?;
        let text = format!("# A GigaSculpt floor preset: view.floor loads it.\n{body}");
        std::fs::write(path, text).map_err(|e| {
            let message = format!("cannot write the floor preset {}: {e}", self.path);
            error(ErrorCode::Precondition, message)
        })?;
        Ok(preset)
    }
}
register_command!(ViewFloorSave, "Save floor preset");
