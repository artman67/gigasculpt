//! `ui.screenshot`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::errors::internal;
use crate::view::deliver;
use crate::{Command, Ctx, Example, register_command};

/// A PNG of the real window, panels included, exactly as last presented.
///
/// Returns the image as base64 text, or writes it to `path` and returns the path. To see the
/// model without the interface, or from another angle, render a snapshot instead: that never
/// moves the window's camera.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiScreenshot {
    /// Write the PNG to this file instead of returning it. Left out, it is returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
}

/// The screenshot.
#[derive(Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    /// Size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Where the PNG was written, when a path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The PNG as base64, when no path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub png_base64: Option<String>,
}

impl Command for UiScreenshot {
    const NAME: &'static str = "ui.screenshot";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Short;
    type Output = ScreenshotOutput;

    fn examples() -> Vec<Example<Self>> {
        let to_file = Self {
            path: Some("/tmp/window.png".into()),
        };
        vec![
            Example::new("save the window to a file (needs the window)", to_file)
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<ScreenshotOutput, CommandError> {
        super::gizmo::show(cx);
        let shot = cx
            .window
            .with(super::WindowHost::screenshot)
            .ok_or_else(no_window)?;
        let shot = shot.map_err(|e| internal(&e))?;
        let (path, png_base64) = deliver(&shot.png, self.path)?;
        Ok(ScreenshotOutput {
            width: shot.width,
            height: shot.height,
            path,
            png_base64,
        })
    }
}
register_command!(UiScreenshot, "Window screenshot");
