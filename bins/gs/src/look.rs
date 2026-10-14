//! Looking at the scene, for `gs look` and the `gs_look` tool: one view is a `view.snapshot`,
//! several are one `view.contact_sheet`, a single labelled PNG. Either renders offscreen and
//! never moves the window's camera.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::connection::Connection;
use crate::failure::Failure;

/// What to render.
pub struct Look {
    /// Each a view name or an orbit object, as the view commands take them. None: their
    /// default view.
    pub views: Vec<Value>,
    /// Image width in pixels.
    pub size: u32,
    pub shading: Option<String>,
    pub wireframe: bool,
    /// Where the engine writes the PNG; without it, the PNG comes back as base64.
    pub path: Option<PathBuf>,
}

/// Renders the views and returns the engine's response.
pub fn look(connection: &mut Connection, look: &Look) -> Result<Value, Failure> {
    let mut args = json!({ "size": look.size, "wireframe": look.wireframe });
    if let Some(shading) = &look.shading {
        args["shading"] = json!(shading);
    }
    if let Some(path) = &look.path {
        args["path"] = json!(path.display().to_string());
    }
    let command = match look.views.as_slice() {
        [] => "view.snapshot",
        [one] => {
            args["view"] = one.clone();
            "view.snapshot"
        }
        several => {
            args["views"] = json!(several);
            "view.contact_sheet"
        }
    };
    connection.run(command, args)
}

/// One line on what was rendered, from the command's result: `front, iso: 1024x512 PNG,
/// matcap shading`.
pub fn summary(result: &Value) -> String {
    let views: Vec<&str> = (result["views"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    format!(
        "{}: {}x{} PNG, {} shading",
        views.join(", "),
        result["width"],
        result["height"],
        result["shading"].as_str().unwrap_or_default()
    )
}
