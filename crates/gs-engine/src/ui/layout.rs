//! `ui.layout`.

use std::path::Path;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{LayoutInfo, no_window};
use crate::{Command, Ctx, Example, errors, register_command};

/// The hand that holds the pen. Trays sit on the side away from it, and the header's view
/// buttons at the bar's end on its side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Hand {
    #[default]
    Right,
    Left,
}

/// A part of the settings file `ui.layout` `export` writes and `import` reads (decision 0050).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LayoutPart {
    /// The layout shown, for which hand, at what scale, with which sections folded and trays
    /// hidden or widened.
    Arrangement,
    /// Every quick menu and the one shown.
    Quick,
    /// The keymap's changes, as `keymap.toml` holds them.
    Keymap,
}

impl LayoutPart {
    /// Every part, in the file's order.
    pub const ALL: [Self; 3] = [Self::Arrangement, Self::Quick, Self::Keymap];

    /// Its name, as the file's table and `parts` give it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Arrangement => "arrangement",
            Self::Quick => "quick",
            Self::Keymap => "keymap",
        }
    }
}

/// The settings file's version: the one `export` writes and the highest `import` reads.
pub const SETTINGS_VERSION: u32 = 1;

/// Switch the window's layout (by name, or the next or previous one), the hand it is arranged
/// for, its scale or its theme, show or hide its trays and the rest of its panels, save the
/// arrangement shown as a layout of its own, restore the default layout as shipped, or write
/// and read back one settings file. The window shows one interface, `workbench`
/// (decision 0025); a layout file the user adds arranges the same controls another way, and a
/// saved arrangement is such a file (Ctrl+1 to 3 show those saved as `arrangement-1` to 3).
/// Every control runs a command, so switching changes nothing in the scene, the brush or the
/// keymap. Left out, a setting stays. Answers with the layout now shown and every layout there
/// is.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiLayout {
    /// The layout to show, by name (its file name without `.toml`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The hand that holds the pen: `left` mirrors the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<Hand>,
    /// The interface's size on top of the scale the display asks for: 1 is as the display
    /// asks, 1.5 half as big again. From 0.5 to 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.5, max = 3.0))]
    pub scale: Option<f32>,
    /// Saves the arrangement shown (the layout with its trays at their present widths) as a
    /// layout file of this name in the user's `layouts` folder, replacing one there of that
    /// name, and shows it. Lowercase letters, digits and dashes; not with `name`. The file is
    /// the user's: the command is not logged, so a replay never writes layouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<String>,
    /// The theme to show, by name: `dark`, `light`, `high-contrast` or one of the user's own
    /// (the files in their `themes` folder); the answer's `themes` lists them all. It applies
    /// from the next frame and is kept with the layout state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Steps through the themes in order, wrapping at either end: 1 shows the next, -1 the one
    /// before. Not with `theme`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -1, max = 1))]
    pub theme_step: Option<i32>,
    /// Shows (true) or hides (false) the tray on the window's left edge. Kept by the tray, so
    /// arranged for the other hand it goes with it to the right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<bool>,
    /// Shows or hides the tray on the window's right edge, as `left` does the left one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<bool>,
    /// False hides the shelf, the trays and the header with the header's view buttons at once
    /// (minimal chrome), leaving the 3D view and the status line; true shows them again as they
    /// were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome: Option<bool>,
    /// One section open at a time in each tray: a tap that opens a section shuts the others
    /// in its tray, unless Shift is held. On at first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_open: Option<bool>,
    /// Steps through the layouts in order, wrapping at either end: 1 shows the next, -1 the
    /// one before. The theme stays. Not with `name`, `save` or `restore`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -1, max = 1))]
    pub step: Option<i32>,
    /// Shows the default layout as shipped: the Workbench with its sections folded as at
    /// first, its trays at their own widths and every panel showing. The hand, the scale and
    /// the theme stay. Not with `name`, `save` or `step`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore: bool,
    /// Writes the keymap's changes, the quick menus and the arrangement to one settings file at
    /// this absolute path, replacing a file there (decision 0050). Not with `name`, `save`,
    /// `step`, `restore` or `import`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<String>,
    /// Reads a settings file `export` wrote, from this absolute path: every part it holds, or
    /// those `parts` names, each in place of the window's, and the others stay exactly as they
    /// were. All or nothing. Not with `name`, `save`, `step`, `restore` or `export`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<String>,
    /// The parts `import` reads: `arrangement`, `quick` and `keymap`. Left out, every part the
    /// file holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<LayoutPart>,
}

/// The interface scale `ui.layout` accepts.
const SCALES: [f32; 2] = [0.5, 3.0];

impl Command for UiLayout {
    const NAME: &'static str = "ui.layout";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes how the window looks, not the session: a replay never needs it.
    const LOGGED: bool = false;
    type Output = LayoutInfo;

    fn examples() -> Vec<Example<Self>> {
        let workbench = Self {
            name: Some("workbench".into()),
            ..Self::default()
        };
        let save = Self {
            save: Some("arrangement-1".into()),
            ..Self::default()
        };
        let light = Self {
            theme: Some("light".into()),
            ..Self::default()
        };
        let next = Self {
            step: Some(1),
            ..Self::default()
        };
        let export = Self {
            export: Some("/path/to/settings.toml".into()),
            ..Self::default()
        };
        let import = Self {
            import: Some("/nowhere/settings.toml".into()),
            parts: vec![LayoutPart::Keymap],
            ..Self::default()
        };
        vec![
            Example::new(
                "write the keymap, quick menus and arrangement to one file (needs the window)",
                export,
            )
            .fails_with(ErrorCode::Precondition),
            Example::new("read back only the keymap from a settings file", import)
                .fails_with(ErrorCode::NotFound),
            Example::new("show the Workbench layout (needs the window)", workbench)
                .fails_with(ErrorCode::Precondition),
            Example::new("show the light theme (needs the window)", light)
                .fails_with(ErrorCode::Precondition),
            Example::new("show the next layout (needs the window)", next)
                .fails_with(ErrorCode::Precondition),
            Example::new(
                "save the arrangement shown for Ctrl+1 (needs the window)",
                save,
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<LayoutInfo, CommandError> {
        if let Some(scale) = self.scale.filter(|s| !(SCALES[0]..=SCALES[1]).contains(s)) {
            let [low, high] = SCALES;
            let message = format!("{scale} is not from {low} to {high}");
            return Err(errors::args_invalid("/scale", message));
        }
        if let Some(step) = self.theme_step {
            if self.theme.is_some() {
                let message = "give `theme` to show a theme or `theme_step` to step, not both";
                return Err(errors::args_invalid("/theme_step", message));
            }
            if step.abs() != 1 {
                let message = format!("{step}: 1 for the next theme, -1 for the one before");
                return Err(errors::args_invalid("/theme_step", message));
            }
        }
        let picks = [
            self.name.is_some(),
            self.save.is_some(),
            self.step.is_some(),
            self.restore,
            self.export.is_some(),
            self.import.is_some(),
        ];
        if picks.iter().filter(|p| **p).count() > 1 {
            let message = "give one of `name`, `save`, `step`, `restore`, `export` and `import`";
            return Err(errors::args_invalid("", message));
        }
        if !self.parts.is_empty() && self.import.is_none() {
            let message = "`parts` names what `import` reads; give `import` too";
            return Err(errors::args_invalid("/parts", message));
        }
        for (at, path) in [("/export", &self.export), ("/import", &self.import)] {
            if let Some(p) = path.as_ref().filter(|p| !Path::new(p).is_absolute()) {
                return Err(errors::args_invalid(
                    at,
                    format!("`{p}`: give an absolute path"),
                ));
            }
        }
        if let Some(path) = &self.import {
            check_import(path, &self.parts)?;
        }
        if let Some(step) = self.step.filter(|s| s.abs() != 1) {
            let message = format!("{step}: 1 for the next layout, -1 for the one before");
            return Err(errors::args_invalid("/step", message));
        }
        if let Some(save) = &self.save {
            let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
            if save.is_empty() || !save.chars().all(plain) {
                let message = format!("`{save}`: use lowercase letters, digits and dashes");
                return Err(errors::args_invalid("/save", message));
            }
        }
        let shown = cx.window.with(|w| w.layout(&self)).ok_or_else(no_window)?;
        shown.map_err(|e| errors::error(ErrorCode::NotFound, e))
    }
}
register_command!(UiLayout, "Set layout");

/// `import`'s file, read far enough to refuse it before the window is asked (decision 0050
/// section 4): it is there, reads as TOML, is a settings file of a version this program
/// reads, and holds every part `parts` names.
fn check_import(path: &str, parts: &[LayoutPart]) -> Result<(), CommandError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        let code = match e.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            _ => ErrorCode::Precondition,
        };
        errors::error(code, format!("{path}: {e}"))
    })?;
    let table: toml::Table = (text.parse())
        .map_err(|e: toml::de::Error| errors::error(ErrorCode::Damaged, format!("{path}: {e}")))?;
    let refused = |message: String, hint: &str| CommandError {
        hint: Some(hint.into()),
        ..errors::error(ErrorCode::Precondition, message)
    };
    match table.get("version").and_then(toml::Value::as_integer) {
        Some(v) if v > i64::from(SETTINGS_VERSION) => {
            let message = format!(
                "{path} was made by a newer GigaSculpt (version {v}); this one reads version \
                 {SETTINGS_VERSION}"
            );
            return Err(refused(message, "open it in a newer GigaSculpt"));
        }
        Some(v) if v >= 1 => {}
        _ => {
            let message = format!("{path} is not a settings file: it has no `version`");
            return Err(refused(message, "`ui.layout` with `export` writes one"));
        }
    }
    let held: Vec<&str> = (LayoutPart::ALL.iter())
        .map(|p| p.name())
        .filter(|n| table.contains_key(*n))
        .collect();
    if let Some(p) = parts.iter().find(|p| !held.contains(&p.name())) {
        let holds = if held.is_empty() {
            "none".into()
        } else {
            held.join(", ")
        };
        let message = format!("{path} holds no `{}` part; it holds {holds}", p.name());
        return Err(errors::error(ErrorCode::NotFound, message));
    }
    Ok(())
}
