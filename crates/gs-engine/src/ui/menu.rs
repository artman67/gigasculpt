//! `ui.menu`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::{Command, Ctx, Example, errors, register_command};

/// Open one of the 3D view's header menus under its title, or close it (decision 0060,
/// section 2), as a tap on its title does, so a key or an agent can. A pick in a menu runs and
/// closes it; a dropdown panel stays open after each change. Escape or a press outside closes
/// either, and that press does nothing else. `ui.describe` lists an open menu as the panel
/// `menu`, titled with its name. Left out, `open` changes nothing.
/// Answers the menu open, if one is, and the header's menus in order.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiMenu {
    /// The menu to open, by its set's name as the layout's `[header]` `menus` gives it
    /// (`file`), or false to close the one open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<MenuOpen>,
}

/// Which menu `ui.menu` opens: one by name, or none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MenuOpen {
    /// The menu of this name.
    Name(String),
    /// False: none.
    Shut(bool),
}

/// The header's menus after `ui.menu`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct MenuInfo {
    /// The menu open, by name; null when none is.
    pub open: Option<String>,
    /// The header's menus, in order.
    pub menus: Vec<String>,
}

impl Command for UiMenu {
    const NAME: &'static str = "ui.menu";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes what the window shows, not the session; what a menu runs is logged.
    const LOGGED: bool = false;
    type Output = MenuInfo;

    fn examples() -> Vec<Example<Self>> {
        let file = Self {
            open: Some(MenuOpen::Name("file".into())),
        };
        let shut = Self {
            open: Some(MenuOpen::Shut(false)),
        };
        vec![
            Example::new("open the File menu (needs the window)", file)
                .fails_with(ErrorCode::Precondition),
            Example::new("close the menu open (needs the window)", shut)
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MenuInfo, CommandError> {
        if self.open == Some(MenuOpen::Shut(true)) {
            let message = "give a menu's name to open it, or false to close it";
            return Err(errors::args_invalid("/open", message));
        }
        let shown = cx.window.with(|w| w.menu(&self)).ok_or_else(no_window)?;
        shown.map_err(|e| errors::error(ErrorCode::NotFound, e))
    }
}
register_command!(UiMenu, "Header menu");
