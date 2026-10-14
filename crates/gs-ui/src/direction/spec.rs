//! What a layout file says, as the types it reads into: one layout ([`Direction`]) and its
//! parts: the panels on the window's edges, the header, the palette, the wheel, the Reference
//! View's place, the Library's tabs and each tool's settings.

use std::collections::BTreeMap;

use gs_engine::{Hand, StateValues};
use serde::{Deserialize, Serialize};

/// The edge of the window a panel sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// The edge for a hand: a left-handed layout swaps left and right.
    #[must_use]
    pub fn for_hand(self, hand: Hand) -> Edge {
        match (self, hand) {
            (Edge::Left, Hand::Left) => Edge::Right,
            (Edge::Right, Hand::Left) => Edge::Left,
            (edge, _) => edge,
        }
    }
}

/// Where the command palette opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    /// Near the top, centered.
    #[default]
    Top,
    /// In the middle of the window.
    Center,
    /// At the pen.
    Cursor,
}

/// How the command palette opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteSpec {
    #[serde(default)]
    pub place: Place,
    /// Width in points.
    pub width: f32,
}

impl Default for PaletteSpec {
    fn default() -> Self {
        Self {
            place: Place::Top,
            width: 560.0,
        }
    }
}

/// The brush wheel: which piece's controls it rings, and how far from the pen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WheelSpec {
    pub show: String,
    /// Distance from the pen to the middle of each slot, in points.
    pub radius: f32,
}

/// Where a layout shows the Reference View (decision 0020).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefPlace {
    /// A window over the 3D view, `width` by `height` points, starting at its top left; moved
    /// by its title and resized by its corner.
    #[default]
    Float,
    /// A panel docked on the left, inside the layout's own panels, `width` points wide and
    /// resized by its inner edge.
    Left,
    /// The same on the right.
    Right,
    /// Half the 3D view, on the right.
    Split,
}

/// Where a layout shows the Reference View, and how big.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSpec {
    #[serde(default)]
    pub place: RefPlace,
    /// In points: the floating window's or a docked panel's width.
    #[serde(default = "reference_width")]
    pub width: f32,
    /// In points: the floating window's height.
    #[serde(default = "reference_width")]
    pub height: f32,
}

fn reference_width() -> f32 {
    520.0
}

impl Default for ReferenceSpec {
    fn default() -> Self {
        Self {
            place: RefPlace::Float,
            width: reference_width(),
            height: reference_width(),
        }
    }
}

/// The 3D view's header (decision 0060, section 2): a bar along the view's top edge, one control
/// tall, that grows a row when it does not fit. `menus` are its menu titles, each a piece by
/// name (a set opens as a menu; a set with a slider, or a piece drawn in code, as a dropdown
/// panel), at the end away from the pen; `show` are the pieces at the pen end, in order, each a
/// piece or the id of one of `options` (decision 0060 as amended for H5).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderSpec {
    #[serde(default)]
    pub menus: Vec<String>,
    #[serde(default)]
    pub show: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<HeaderOptions>,
    /// The button the menus fold into while their titles do not fit one row (decision 0060 as
    /// amended for H7); left out, it reads Menus with no hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold: Option<HeaderFold>,
}

/// The header's fold: its label, and what hovering it says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderFold {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// A view button at the header's pen end that is there for its options (Look; decision 0025,
/// rule 8.5): a tap opens them under it, showing `show`'s pieces, and Escape or a press
/// elsewhere shuts them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderOptions {
    /// Its section in `ui.describe`, and the options' panel while they are open.
    pub id: String,
    pub label: String,
    /// What hovering it says under its label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub show: Vec<String>,
    /// The `session.state` value that lights it while true. It is lit while its options are
    /// open too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shows: Option<String>,
}

/// A tool's settings, which Brush tray > Brush shows while it is active (decision 0025,
/// section 3, amended 2026-10-11). The section shows the first tool whose `when` is true; one
/// without `when` always matches, so the brush's comes last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// Names it in a refused file.
    pub id: String,
    /// The boolean `session.state` value that is true while the tool is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Its settings, in order: sets, `set/Group`, or shared widgets other than `brush`.
    pub show: Vec<String>,
    /// The sections of the tray that shows `brush` while it is active, in place of the
    /// panel's own (the Gizmo tool's: its header, Basics and Pose); empty keeps the panel's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tray: Vec<String>,
}

/// A tab of the Library (decision 0025, section 13): one piece, its controls drawn as tiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryTab {
    /// Unique in its layout: `ui.library` `tab` names it.
    pub id: String,
    pub title: String,
    /// A set, `set/Group`, or a shared widget with controls (`brushes`, `alphas`, `looks`,
    /// `layouts`).
    pub show: String,
}

/// One panel on an edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSpec {
    /// Unique in its layout; `ui.describe` reports it.
    pub id: String,
    pub title: String,
    pub edge: Edge,
    /// Width of a left or right panel, in points, until its inner edge is dragged (then
    /// `layout.toml` keeps the width); a top or bottom one's least height, which grows to fit
    /// its rows up to a third of the window's.
    pub size: f32,
    /// What it shows, in order: sets, `set/Group`, or shared widgets.
    pub show: Vec<String>,
}

/// One layout file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Direction {
    /// Its file name without `.toml`; not in the file.
    #[serde(skip)]
    pub name: String,
    pub title: String,
    /// Where it comes in the list of layouts.
    pub order: u32,
    #[serde(default)]
    pub palette: PaletteSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel: Option<WheelSpec>,
    /// Where the Reference View shows while one is open. Left out: a floating window.
    #[serde(default)]
    pub reference: ReferenceSpec,
    /// The 3D view's header. Left out: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<HeaderSpec>,
    /// The pieces of the settings window (`ui.settings`), top to bottom. Left out: an empty
    /// window.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<String>,
    /// The pieces of the quick menu (`ui.quick`), top to bottom. Left out: no quick menu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick: Vec<String>,
    /// The Library's tabs, in order. Left out: no Library.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<LibraryTab>,
    /// What the `brush` widget shows for each tool. Left out: nothing.
    #[serde(rename = "tool", default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    #[serde(rename = "panel", default)]
    pub panels: Vec<PanelSpec>,
}

impl Direction {
    /// This layout saved as the arrangement `name`, coming at `order`: its side panels as wide
    /// as `widths` has them (by panel id; the others as they are), titled after the name
    /// (`arrangement-1` is Arrangement 1).
    #[must_use]
    pub fn arranged(&self, name: &str, widths: &BTreeMap<String, f32>, order: u32) -> Direction {
        let mut saved = self.clone();
        let words = name.replace('-', " ");
        let mut first = words.chars();
        let initial = first.next().map(|c| c.to_ascii_uppercase());
        saved.title = initial.into_iter().chain(first).collect();
        saved.name = name.to_string();
        saved.order = order;
        for panel in &mut saved.panels {
            let side = matches!(panel.edge, Edge::Left | Edge::Right);
            if let Some(width) = widths.get(&panel.id).filter(|_| side) {
                panel.size = *width;
            }
        }
        saved
    }

    /// The active tool: the first whose `when` is true in `state`, or one without `when`.
    pub fn tool(&self, state: &StateValues) -> Option<&ToolSpec> {
        let active = |t: &&ToolSpec| {
            (t.when.as_ref()).is_none_or(|v| state.get(v) == Some(&serde_json::Value::Bool(true)))
        };
        self.tools.iter().find(active)
    }
}
