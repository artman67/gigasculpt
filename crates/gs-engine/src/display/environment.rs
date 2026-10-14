//! `view.environment`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Environment;
use crate::errors::{args_invalid, error};
use crate::{Command, Ctx, Example, register_command};

/// Choose what Lit mode lights with: a shipped environment by name, a Radiance `.hdr` light
/// probe, or each look's own again.
///
/// The environment is fixed to the camera, so the light stays where it is as the view turns.
/// Its brightest light is taken out as light 1 (`light.set` moves, recolors or turns it off) and
/// the rest is what Ambient scales. Lights `lit`, `clay` and `zebra` shading; MatCaps keep the
/// light they were baked in. Each look's own environment (`studio` for most) is the start. A
/// file is read now (seconds for a large one) and its content hash is logged, so a replay
/// refuses a changed file. A display setting of the session: never undone, never saved with
/// the scene.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewEnvironment {
    /// A shipped environment: `studio`, `soft-studio`, `studio-small-09`, `studio-small-08` or
    /// `photo-studio-01`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub name: Option<String>,
    /// A Radiance `.hdr` file: an equirectangular light probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
    /// With `path`: degrees about the vertical axis, -360 to 360, positive turning the light to
    /// the viewer's right. Left out: the turn that puts its dominant light on the key side, as
    /// the shipped probes are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -360.0, max = 360.0))]
    pub turn: Option<f32>,
    /// Back to each look's own environment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
    /// The BLAKE3 hash of the file's bytes, as 64 hex digits. Filled in by the engine before it
    /// runs, so the log records which content was read; a call whose hash no longer matches the
    /// file is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// The environment Lit mode lights with now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentInfo {
    /// A shipped environment's name or the file's name; null while each look lights with its
    /// own.
    pub name: Option<String>,
    /// Its light 1 as taken out of it; null when nothing in it is bright enough, or the
    /// session cannot render.
    pub key: Option<KeyLight>,
}

/// Light 1 as an environment gives it, before `light.set` changes it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KeyLight {
    /// Toward the light in view space.
    pub direction: [f32; 3],
    /// Angular radius in degrees.
    pub size_deg: f32,
    /// 1 lights a white card facing the light to reflect 1.
    pub intensity: f32,
    /// Linear RGB at luminance 1.
    pub color: [f32; 3],
}

impl ViewEnvironment {
    /// The environment the arguments choose.
    fn chosen(&self) -> Result<Environment, CommandError> {
        let given = [self.name.is_some(), self.path.is_some(), self.reset];
        if given.iter().filter(|g| **g).count() != 1 {
            return Err(args_invalid("", "give one of name, path or reset"));
        }
        if self.turn.is_some() && self.path.is_none() {
            return Err(args_invalid("/turn", "turn goes with path"));
        }
        if let Some(name) = &self.name {
            return Ok(Environment::Shipped(name.trim().to_string()));
        }
        let Some(path) = &self.path else {
            return Ok(Environment::Own);
        };
        let hash = self.hash.clone().ok_or_else(|| {
            let message = format!("cannot read {path}: there is no such file");
            CommandError {
                path: Some("/path".into()),
                ..error(ErrorCode::NotFound, message)
            }
        })?;
        Ok(Environment::File {
            path: path.clone(),
            hash,
            turn: self.turn,
        })
    }
}

impl Command for ViewEnvironment {
    const NAME: &'static str = "view.environment";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Short;
    const READS_FILE: Option<&'static str> = Some("path");
    type Output = EnvironmentInfo;

    fn examples() -> Vec<Example<Self>> {
        let none = || Self {
            name: None,
            path: None,
            turn: None,
            reset: false,
            hash: None,
        };
        let soft = Self {
            name: Some("soft-studio".into()),
            ..none()
        };
        let probe = Self {
            path: Some("/nonexistent/sunset.hdr".into()),
            turn: Some(90.0),
            ..none()
        };
        let own = Self {
            reset: true,
            ..none()
        };
        vec![
            Example::new("light with the soft studio", soft),
            Example::new(
                "a light probe turned a quarter to the right (refused: no such file)",
                probe,
            )
            .fails_with(ErrorCode::NotFound),
            Example::new("back to each look's own environment", own),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<EnvironmentInfo, CommandError> {
        let chosen = self.chosen()?;
        // The render host reads and prepares it now, so a bad file is refused here and the
        // next frame lights with it; a session that cannot render only records the choice.
        let prepared = match &chosen {
            Environment::Own => None,
            other => cx.render.with(|r| r.environment(other)),
        };
        let info = match prepared {
            Some(Ok(info)) => info,
            Some(Err(e)) => {
                let code = match chosen {
                    Environment::Shipped(_) => ErrorCode::NotFound,
                    _ => ErrorCode::Precondition,
                };
                return Err(error(code, e));
            }
            None => EnvironmentInfo {
                name: match &chosen {
                    Environment::Own => None,
                    Environment::Shipped(name) => Some(name.clone()),
                    Environment::File { path, .. } => Some(path.clone()),
                },
                key: None,
            },
        };
        cx.display.environment = chosen;
        Ok(info)
    }
}
register_command!(ViewEnvironment, "Set environment");
