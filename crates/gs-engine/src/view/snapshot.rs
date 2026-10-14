//! `view.snapshot`.

use gs_protocol::{CommandError, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Draws, ImageOutput, Look, NamedView, Shading, ViewSpec, check, look};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Render the scene offscreen as one PNG, from a camera of its own or through a view.
///
/// No view moves: `"main"` looks from exactly where the window looks. Works the same in a
/// headless session. Edges are anti-aliased unless `antialias` is false. With `transparent`,
/// the background is left out. Returns the PNG as base64, or writes it to `path`. For several
/// views in one picture, use `view.contact_sheet`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewSnapshot {
    /// The camera: a standard view (front, back, left, right, top, bottom, iso), a session
    /// view's name ("main", "agent:<name>"), an exact camera as `view.get` reports it, or an
    /// orbit. Left out: iso.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewSpec>,
    /// Width and height in pixels, 16 to 4096: one number for a square, or `[width, height]`.
    /// Left out: 1024.
    #[serde(default = "default_size")]
    pub size: Size,
    /// How surfaces are shaded: matcap, lit, clay, zebra or normals. Left out: the session's
    /// (`view.display`; matcap until it is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<Shading>,
    /// What the picture draws besides the shaded parts: `wireframe`, `antialias`, `grid`,
    /// `active_outline`, `face_groups` and `density`, given beside the other arguments.
    #[serde(flatten)]
    pub draws: Draws,
    /// Leave the background out: where no part is, the PNG is transparent, and along the
    /// parts' anti-aliased edges partly so, to lay the picture over another (an app icon's,
    /// say). It is drawn twice, over black and over white, so it takes twice as long. Left
    /// out: false.
    #[serde(default)]
    pub transparent: bool,
    /// Write the PNG to this file instead of returning it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
}

fn default_size() -> Size {
    Size::Square(1024)
}

/// A picture's size in pixels: one number for a square, or `[width, height]`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Size {
    Square(#[schemars(range(min = 16, max = 4096))] u32),
    Sides(#[schemars(length(min = 2, max = 2))] [u32; 2]),
}

impl Size {
    /// Width and height, or why they are no picture size.
    fn sides(self) -> Result<[u32; 2], CommandError> {
        let sides = match self {
            Size::Square(s) => [s, s],
            Size::Sides(s) => s,
        };
        if sides.iter().all(|s| (16..=4096).contains(s)) {
            Ok(sides)
        } else {
            let message = "each side must be 16 to 4096 pixels";
            Err(crate::errors::args_invalid("/size", message))
        }
    }
}

impl Command for ViewSnapshot {
    const NAME: &'static str = "view.snapshot";
    const KIND: Kind = Kind::Query;
    type Output = ImageOutput;

    fn examples() -> Vec<Example<Self>> {
        let front = Self {
            view: Some(ViewSpec::Named(NamedView::Front)),
            size: Size::Square(512),
            shading: Some(Shading::Clay),
            draws: Draws::default(),
            transparent: false,
            path: None,
        };
        vec![
            Example::new(
                "a sphere from the front, 512 px, in clay (a session without a renderer refuses)",
                front,
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
            Example::raw("an empty scene has nothing to look at", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check(cx)
    }

    fn run(self, cx: &mut Ctx) -> Result<ImageOutput, CommandError> {
        let view = self.view.unwrap_or(ViewSpec::Named(NamedView::Iso));
        let settings = Look {
            views: vec![view],
            tile: self.size.sides()?,
            columns: 1,
            shading: self.shading,
            draws: self.draws,
            transparent: self.transparent,
            path: self.path,
        };
        look(cx, settings)
    }
}
register_command!(ViewSnapshot, "Snapshot");
