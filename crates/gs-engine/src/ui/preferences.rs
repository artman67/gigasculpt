//! The preferences window's part of `ui.settings` (decision 0052): what it takes to show a page,
//! search, Reset and pin the brush at start, and what it answers. The window keeps the
//! preferences file; every setting is set by its own command.

use gs_protocol::Step;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What Reset puts back to its default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResetScope {
    /// One setting, by its `session.state` name (`pen.lazy_pressure`).
    Setting(String),
    /// Every setting on a page, by its title (`Pen`).
    Page(String),
    /// Every setting.
    All,
}

/// A setting as the settings window lists it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SettingInfo {
    /// Its `session.state` name, which its value shows as and `preferences.toml` keeps it by.
    pub name: String,
    /// What the window calls it.
    pub label: String,
    /// The page it is on.
    pub page: String,
    /// The command that sets it.
    pub command: String,
    /// What Reset runs to put its default back.
    pub reset: Step,
}

/// The preferences window as `ui.settings` answers it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct PreferencesInfo {
    /// The page shown, by its title.
    pub page: String,
    /// Every page, by its title, in order.
    pub pages: Vec<String>,
    /// The search text; empty for none.
    pub search: String,
    /// The settings the search finds, in the window's order.
    pub found: Vec<SettingInfo>,
    /// Every setting, page by page.
    pub listed: Vec<SettingInfo>,
    /// The Reset question waiting for `confirm`; null while none is.
    pub asking: Option<String>,
    /// The brush the window starts with: `last`, the brush used last, or a pinned brush's
    /// name.
    pub launch_brush: String,
    /// The settings this call's confirmed Reset put back, by name: their commands run as the
    /// window's next calls.
    pub reset: Vec<String>,
}
