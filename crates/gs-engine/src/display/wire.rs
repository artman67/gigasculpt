//! How the wireframe is drawn (catalog item `canvas.wireframe-view`): `view.wire` sets it.
//! Whether a view draws its wire is `view.look`'s `wireframe`, and a snapshot's own.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::color;
use crate::{Command, Ctx, Example, register_command};

/// How the wireframe is drawn, and the density view.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, each one view.wire sets on its own"
)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WireStyle {
    /// The polygon edges.
    pub line: bool,
    /// The face group colors in place of each part's paint while its wire shows.
    pub fill: bool,
    /// The edges' color, `"#rrggbb"`.
    #[serde(with = "color::as_hex")]
    #[schemars(with = "String")]
    pub color: [u8; 3],
    /// How much an edge covers the surface, 0 to 1.
    pub opacity: f32,
    /// Every part colored by the size of its polygons on screen: red large, green, blue small.
    pub density: bool,
    /// The window's views draw their wire only while the view moves.
    pub dots: bool,
}

impl Default for WireStyle {
    /// The edges near-black at half opacity (the catalog's default), no fill, no density view,
    /// the wire always drawn.
    fn default() -> Self {
        WireStyle {
            line: true,
            fill: false,
            color: [20, 20, 26],
            opacity: 0.5,
            density: false,
            dots: false,
        }
    }
}

impl WireStyle {
    /// Its values by the names `session.state` gives them.
    pub(crate) fn values(&self) -> [(&'static str, Value); 6] {
        [
            ("view.wire.line", json!(self.line)),
            ("view.wire.fill", json!(self.fill)),
            ("view.wire.color", json!(color::hex(self.color))),
            ("view.wire.opacity", json!(self.opacity)),
            ("view.wire.density", json!(self.density)),
            ("view.wire.dots", json!(self.dots)),
        ]
    }
}

/// Set how the wireframe is drawn. Arguments left out keep their value.
///
/// Each view's wire is switched on by `view.look`'s `wireframe`, and a snapshot draws one when
/// it passes `wireframe`; every wire is drawn in this style. The density view colors every
/// part in the window's views (a snapshot passes its own `density`). A display setting of the
/// session: never undone, never saved with the scene.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewWire {
    /// Draw the polygon edges. Starts on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<bool>,
    /// Show the face group colors in place of each part's paint while its wire shows (off,
    /// the start: a part's own paint). With `line` off, the wire is the group colors alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<bool>,
    /// The edges' color, `"#rrggbb"`. Starts at `"#14141a"`, near black.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = color::PATTERN))]
    pub color: Option<String>,
    /// How much an edge covers the surface, 0 to 1. Starts at 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub opacity: Option<f32>,
    /// Color every part by the size of its polygons on screen in place of its paint: red where
    /// they are 32 pixels across or more, green at 8, blue at 2 or less. Starts off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<bool>,
    /// The window's views draw their wire only while the view is being moved, so it shows how
    /// the model turns without covering it at rest. Starts off. Snapshots are not affected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dots: Option<bool>,
}

impl Command for ViewWire {
    const NAME: &'static str = "view.wire";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = WireStyle;

    fn examples() -> Vec<Example<Self>> {
        let clean = Self {
            fill: Some(false),
            color: Some("#ffffff".into()),
            opacity: Some(1.0),
            ..Self::default()
        };
        let groups = Self {
            line: Some(false),
            fill: Some(true),
            ..Self::default()
        };
        let density = Self {
            density: Some(true),
            ..Self::default()
        };
        let dots = Self {
            dots: Some(true),
            ..Self::default()
        };
        vec![
            Example::new("white edges at full opacity", clean),
            Example::new("the wire as face group colors alone, no edges", groups),
            Example::new("color the parts by polygon size on screen", density),
            Example::new("draw the wire only while the view moves", dots),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<WireStyle, CommandError> {
        let w = &mut cx.display.wire;
        let color = self.color.as_deref().map(|c| color::parse("/color", c));
        w.color = color.transpose()?.unwrap_or(w.color);
        w.line = self.line.unwrap_or(w.line);
        w.fill = self.fill.unwrap_or(w.fill);
        w.opacity = self.opacity.unwrap_or(w.opacity);
        w.density = self.density.unwrap_or(w.density);
        w.dots = self.dots.unwrap_or(w.dots);
        Ok(*w)
    }
}
register_command!(ViewWire, "Set wireframe style");
