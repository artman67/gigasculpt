//! `view.contact_sheet`.

use gs_protocol::{CommandError, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Draws, ImageOutput, Look, NamedView, Orbit, Shading, ViewSpec, check, look};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Most tiles across a sheet.
const MAX_COLUMNS: u32 = 4;

/// Render the scene from several cameras as one PNG: square tiles left to right and top to
/// bottom, each labelled with its view.
///
/// One picture an agent reads at once costs far fewer tokens than one picture per view. Each
/// tile renders offscreen as `view.snapshot` does, so the window's camera never moves, and it
/// works the same in a headless session. Returns the PNG as base64, or writes it to `path`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewContactSheet {
    /// The cameras, 1 to 16, as `view.snapshot` takes them: standard views, session views,
    /// exact cameras or orbits.
    #[schemars(length(min = 1, max = 16))]
    pub views: Vec<ViewSpec>,
    /// Tiles across, 1 to 4. Left out: as square as it gets (2 for 2 to 4 views).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 4))]
    pub columns: Option<u32>,
    /// Width of the whole sheet in pixels; each tile is that over the columns. Left out:
    /// 1024.
    #[serde(default = "default_size")]
    #[schemars(range(min = 64, max = 4096))]
    pub size: u32,
    /// How surfaces are shaded: matcap, lit, clay, zebra or normals. Left out: the session's
    /// (`view.display`; matcap until it is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<Shading>,
    /// What every tile draws besides the shaded parts: `wireframe`, `antialias`, `grid`,
    /// `active_outline`, `face_groups` and `density`, given beside the other arguments, as
    /// `view.snapshot` takes them.
    #[serde(flatten)]
    pub draws: Draws,
    /// Write the PNG to this file instead of returning it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
}

fn default_size() -> u32 {
    1024
}

impl Command for ViewContactSheet {
    const NAME: &'static str = "view.contact_sheet";
    const KIND: Kind = Kind::Query;
    type Output = ImageOutput;

    fn examples() -> Vec<Example<Self>> {
        let orbit = Orbit {
            azimuth_deg: Some(30.0),
            elevation_deg: Some(15.0),
            ..Orbit::default()
        };
        let views = [NamedView::Front, NamedView::Right, NamedView::Top];
        let mut views: Vec<ViewSpec> = views.into_iter().map(ViewSpec::Named).collect();
        views.push(ViewSpec::Orbit(orbit));
        let four = Self {
            views,
            columns: None,
            size: 1024,
            shading: Some(Shading::Zebra),
            draws: Draws::default(),
            path: None,
        };
        vec![
            Example::new(
                "a sphere from four sides in zebra stripes, 2 by 2 (a session without a \
                 renderer refuses)",
                four,
            )
            .after("primitive.sphere", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check(cx)
    }

    fn run(self, cx: &mut Ctx) -> Result<ImageOutput, CommandError> {
        let count = self.views.len() as u32;
        let square = (1..MAX_COLUMNS)
            .find(|c| c * c >= count)
            .unwrap_or(MAX_COLUMNS);
        let columns = self.columns.unwrap_or(square).min(count);
        let settings = Look {
            views: self.views,
            tile: [self.size / columns; 2],
            columns,
            shading: self.shading,
            draws: self.draws,
            transparent: false,
            path: self.path,
        };
        look(cx, settings)
    }
}
register_command!(ViewContactSheet, "Contact sheet");
