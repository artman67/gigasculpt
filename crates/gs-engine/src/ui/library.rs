//! `ui.library`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Step};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::{Command, Ctx, Example, errors, register_command};

/// Open or close the Library, the drawer along the bottom of the 3D view that shows the
/// brushes, alphas, MatCaps, environments, layouts and themes as tiles (decision 0025, section
/// 13), and pin its tiles to the shelf as favorites. A tile is a control: it runs its command,
/// as the same control does anywhere, so a favorite is that command and its arguments
/// (`ui.describe` gives each tile's). Left out, a setting stays. Answers with the drawer and
/// the favorites as they are after the change.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiLibrary {
    /// True to open the drawer, false to close it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// The tab to show, by its id in the layout file (`brushes`, `alphas`, `matcaps`,
    /// `environments`, `layouts`, `themes` in the Workbench). It opens the drawer unless
    /// `open` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// Pins a tile to the shelf: the command it runs, with its arguments. Pinned already, it
    /// moves to `at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<Step>,
    /// Where in the favorites `pin` goes, 0 first. Left out: last, or where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    /// Takes a favorite off the shelf, by its command and arguments. The item stays in the
    /// Library: a favorite is a reference, never a copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpin: Option<Step>,
    /// True to show only the materials the scene uses on the Materials tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<bool>,
    /// True to draw the view in a material while the pen rests on its tile; nothing runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
}

/// The Library after `ui.library`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct LibraryInfo {
    pub open: bool,
    /// The tab shown, by id.
    pub tab: String,
    /// Every tab, by id, in order.
    pub tabs: Vec<String>,
    /// The favorites on the shelf, in order.
    pub favorites: Vec<Step>,
    /// Whether the Materials tab shows only the materials the scene uses.
    pub used: bool,
    /// Whether a pen resting on a material's tile previews it in the view.
    pub preview: bool,
}

impl Command for UiLibrary {
    const NAME: &'static str = "ui.library";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes what the window shows and the user's favorites, not the session.
    const LOGGED: bool = false;
    type Output = LibraryInfo;

    fn examples() -> Vec<Example<Self>> {
        let open = Self {
            open: Some(true),
            tab: Some("matcaps".into()),
            ..Self::default()
        };
        vec![
            Example::new("open the Library on its MatCaps (needs the window)", open)
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<LibraryInfo, CommandError> {
        if self.pin.is_some() && self.unpin.is_some() {
            let message = "give `pin` or `unpin`, not both";
            return Err(errors::args_invalid("/unpin", message));
        }
        if self.at.is_some() && self.pin.is_none() {
            return Err(errors::args_invalid("/at", "`at` places a `pin`"));
        }
        let shown = cx.window.with(|w| w.library(&self)).ok_or_else(no_window)?;
        shown.map_err(|e| errors::error(ErrorCode::NotFound, e))
    }
}
register_command!(UiLibrary, "Library");
