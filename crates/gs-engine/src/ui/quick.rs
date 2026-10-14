//! `ui.quick`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::{Command, Ctx, Example, errors, register_command};

/// Open the quick menu at the pointer, or close it, pick which of its menus shows, or change
/// what a menu holds (decision 0025, section 2). It holds a few pieces, each running its own
/// commands, and stays open while they are used; a press outside it or Escape closes it too.
/// At first it has one menu, the pieces the layout file's `quick` lists; it can have up to
/// five, and holding its key while swiping across it steps through them. What the menus hold
/// is kept with the layout state, so it lasts after a restart. The same window can hold one
/// slider alone (`slider`). Left out, a setting stays.
/// Answers whether it is open, the menu shown and what every menu holds: a menu that holds
/// nothing keeps it shut.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiQuick {
    /// True to open it at the pointer, false to close it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// The menu to show and to change, from 1 to 5. One past the last starts a new menu, and
    /// needs `add`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 5))]
    pub menu: Option<usize>,
    /// Steps through the menus, wrapping at either end: 1 shows the next, -1 the one before.
    /// Not with `menu`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -1, max = 1))]
    pub step: Option<i32>,
    /// Adds a piece to the end of the menu: a set of controls (`brush`), one group of a set
    /// (`symmetry/Switches`), a family of brushes (`brushes/Build`) or a shared widget, as a layout
    /// file names them. One already there stays where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<String>,
    /// Takes a piece out of the menu; a menu left with nothing goes, unless it is the only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<String>,
    /// Puts back the menus as shipped: one, the layout file's `quick` list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
    /// Opens the menu's window at the pointer holding only this slider, by its label as
    /// `ui.describe` reports it (`Size`, `Strength`, `Softness`, `Exposure`), as a short tap of
    /// its key does; it closes as the menu does. Not with `open` false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slider: Option<String>,
}

/// The quick menu after `ui.quick`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct QuickInfo {
    pub open: bool,
    /// The slider shown alone, by its label, while one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slider: Option<String>,
    /// The menu shown, from 1.
    pub menu: usize,
    /// What each menu holds, in order: its pieces, top to bottom.
    pub menus: Vec<Vec<String>>,
}

/// The most menus there can be.
const MENUS: usize = 5;

impl Command for UiQuick {
    const NAME: &'static str = "ui.quick";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes what the window shows, not the session; what it runs is logged.
    const LOGGED: bool = false;
    type Output = QuickInfo;

    fn examples() -> Vec<Example<Self>> {
        let open = Self {
            open: Some(true),
            ..Self::default()
        };
        let add = Self {
            menu: Some(2),
            add: Some("history".into()),
            ..Self::default()
        };
        let strength = Self {
            slider: Some("Strength".into()),
            ..Self::default()
        };
        vec![
            Example::new("open the quick menu (needs the window)", open)
                .fails_with(ErrorCode::Precondition),
            Example::new(
                "show the Strength slider alone at the pointer (needs the window)",
                strength,
            )
            .fails_with(ErrorCode::Precondition),
            Example::new(
                "start a second quick menu holding History (needs the window)",
                add,
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<QuickInfo, CommandError> {
        if self.menu.is_some_and(|m| !(1..=MENUS).contains(&m)) {
            let message = format!("menus are numbered 1 to {MENUS}");
            return Err(errors::args_invalid("/menu", message));
        }
        if let Some(step) = self.step {
            if self.menu.is_some() {
                let message = "give `menu` to show a menu or `step` to step, not both";
                return Err(errors::args_invalid("/step", message));
            }
            if step.abs() != 1 {
                let message = format!("{step}: 1 for the next menu, -1 for the one before");
                return Err(errors::args_invalid("/step", message));
            }
        }
        if self.slider.is_some() && self.open == Some(false) {
            let message = "`slider` opens it; leave out `open` or give true";
            return Err(errors::args_invalid("/slider", message));
        }
        let changes = [self.add.is_some(), self.remove.is_some(), self.reset];
        if changes.iter().filter(|c| **c).count() > 1 {
            let message = "give one of `add`, `remove` and `reset`";
            return Err(errors::args_invalid("", message));
        }
        let shown = cx.window.with(|w| w.quick(&self)).ok_or_else(no_window)?;
        shown.map_err(|e| errors::error(ErrorCode::NotFound, e))
    }
}
register_command!(UiQuick, "Quick menu");
