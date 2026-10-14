//! `ui.settings`: the settings window, and the window's own settings that no other command
//! keeps.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::no_window;
use super::preferences::{PreferencesInfo, ResetScope};
use crate::sculpt::presets;
use crate::{Command, Ctx, Example, errors, register_command};

/// The brush wheel's furthest offset from the pen, in points: inside the wheel's middle (48
/// points across its radius), so letting go where it opened picks nothing.
pub const WHEEL_OFFSET_MAX: f32 = 40.0;

/// The window's own settings, which `ui.settings` changes: the held keys, the brush cursor,
/// sliders (`interface.sliders`), the brush wheel (`interface.brush-wheel`), Isolate's options
/// (`parts.isolate`), the Parts list's rows (`parts.list`) and opening the last scene at start
/// (`interface.scene-files`). The settings window's are kept in `preferences.toml`; Isolate's
/// options and the Parts list's rows with the layout state (decision 0052 as amended).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent window switches, one per setting"
)]
pub struct WindowSettings {
    /// Sticky modifier keys (`interface.modifier-keys`): a tap of Shift, Ctrl or Alt turns it
    /// on until the next tap, and a long hold stays momentary. Off, a key is on only while
    /// held.
    pub sticky_modifiers: bool,
    /// The status line says what the held keys do (`Shift: Smooth`), as the cursor does.
    pub status_label: bool,
    /// Sticky keys (`interface.on-canvas-adjust`): a slider's key held past `sticky_delay`
    /// sets the slider as the pointer moves, and a short tap shows the slider. Off, a slider's
    /// key only shows the slider.
    pub sticky_keys: bool,
    /// How long a slider's key is held, in seconds, before it sets the slider.
    pub sticky_delay: f32,
    /// The brush's rings lie flat on the surface under the pen (`interface.cursor-ring`), seen
    /// narrower where it turns away; off, they are circles on the screen.
    pub ring_on_surface: bool,
    /// With symmetry on, a quieter ring shows wherever symmetry repeats the dab.
    pub mirror_rings: bool,
    /// A slider shows its value beside its title; off, only while the pen is over it or drags
    /// it.
    pub show_values: bool,
    /// A drag on a slider whose rail runs through 0 (Falloff, exposure) stops at 0 in a small
    /// dead zone.
    pub center_snap: bool,
    /// Shift held as a drag on the brush's Falloff starts takes it only to -100, 0 and 100,
    /// until Shift is let go.
    pub falloff_snap: bool,
    /// How far the brush wheel opens from the pen, away from the hand that holds it (the
    /// layout's hand), in points: so the hand hides less of it.
    pub wheel_offset: f32,
    /// Dynamic isolate (decision 0028, section 10): while the camera moves with Isolate off,
    /// the 3D view draws only the active part, and every part that shows once it stops.
    pub dynamic_isolate: bool,
    /// A click on empty canvas isolates the active part, or ends Isolate while it is on.
    pub click_isolate: bool,
    /// How many rows Scene tray > Parts shows before it scrolls, 1 to 24.
    pub part_rows: u32,
    /// At start, with no crashed session offered back, the window opens the newest recent
    /// scene (`file.open` with `recent: 1`; decision 0016, section 17).
    pub open_last: bool,
}

/// The most rows Scene tray > Parts shows before it scrolls.
pub const PART_ROWS_MAX: u32 = 24;

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            sticky_modifiers: false,
            status_label: true,
            sticky_keys: true,
            sticky_delay: 0.25,
            ring_on_surface: true,
            mirror_rings: true,
            show_values: true,
            center_snap: true,
            falloff_snap: false,
            wheel_offset: 24.0,
            dynamic_isolate: false,
            click_isolate: false,
            part_rows: 8,
            open_last: false,
        }
    }
}

/// Open or close the settings window: the pen, the held keys, navigation, the sliders and the
/// interface scale (decision 0025), as pages, with a search and Reset (decision 0052), and the
/// brush the window starts with; and change the window's own settings: sticky modifier keys
/// (a tap of Shift, Ctrl or Alt turns it on until the next tap; a long hold stays momentary),
/// whether the status line says what the held keys do, sticky keys (a slider's key held a
/// moment sets the slider with the pointer; a tap shows it), the brush cursor's rings (flat on
/// the surface, and the mirror rings), how sliders and the brush wheel behave, Isolate's options
/// (Dynamic isolate, a click on empty canvas), the Parts list's row count, and whether the
/// window opens the last scene at start. The window's other settings are controls that run
/// their own commands (`ui.navigation`, `ui.layout`). Left out, a setting stays, and so does
/// the window. Answers whether it is open, every setting, and the window's pages, search and
/// settings listed, each with its command and the step that puts its default back. Kept in
/// the window's preferences file; not logged.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiSettings {
    /// True to open it over the 3D view, false to close it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// True: a tap of Shift, Ctrl or Alt turns it on until the next tap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky_modifiers: Option<bool>,
    /// True: the status line says what the held keys do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_label: Option<bool>,
    /// True: a slider's key held a moment sets the slider with the pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky_keys: Option<bool>,
    /// Seconds a slider's key is held before it sets the slider, 0 to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky_delay: Option<f32>,
    /// True: the brush's rings lie flat on the surface under the pen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring_on_surface: Option<bool>,
    /// True: a quieter ring shows wherever symmetry repeats the dab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_rings: Option<bool>,
    // How sliders behave (interface.sliders).
    /// True: sliders show their values; false: only under the pen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_values: Option<bool>,
    /// True: a drag through 0 on a rail across it stops there a moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_snap: Option<bool>,
    /// True: Shift held as a Falloff drag starts takes only -100, 0 and 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub falloff_snap: Option<bool>,
    // The brush wheel (interface.brush-wheel).
    /// Points the wheel opens away from the pen hand, 0 to 40 (mirrored with the hand).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 40.0))]
    pub wheel_offset: Option<f32>,
    // Isolate's options (decision 0028, section 10).
    /// True: while the camera moves with Isolate off, only the active part is drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_isolate: Option<bool>,
    /// True: a click on empty canvas isolates the active part, or ends Isolate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_isolate: Option<bool>,
    /// Rows Scene tray > Parts shows before it scrolls, 1 to 24, rounded to whole rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 24.0))]
    pub part_rows: Option<f32>,
    /// True: at start the window opens the newest recent scene, unless a crash is offered back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_last: Option<bool>,
    // The preferences window (decision 0052).
    /// Show this page, by its title (`Pen`, `Navigation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Find settings whose label, command, name or page holds every word; "" clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    /// Put defaults back: `{"setting": "pen.lazy_pressure"}`, `{"page": "Pen"}` or `"all"`.
    /// The window asks first, unless `confirm` is true in the same call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<ResetScope>,
    /// True does the Reset asked, false cancels it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<bool>,
    /// The brush the window starts with: "last" (the brush used last), "current" (pin the brush
    /// in use) or a brush's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_brush: Option<String>,
}

impl UiSettings {
    /// `now` with this change.
    pub fn apply(&self, now: WindowSettings) -> WindowSettings {
        let delay = self.sticky_delay.map(|d| d.clamp(0.0, 2.0));
        WindowSettings {
            sticky_modifiers: self.sticky_modifiers.unwrap_or(now.sticky_modifiers),
            status_label: self.status_label.unwrap_or(now.status_label),
            sticky_keys: self.sticky_keys.unwrap_or(now.sticky_keys),
            sticky_delay: delay.unwrap_or(now.sticky_delay),
            ring_on_surface: self.ring_on_surface.unwrap_or(now.ring_on_surface),
            mirror_rings: self.mirror_rings.unwrap_or(now.mirror_rings),
            show_values: self.show_values.unwrap_or(now.show_values),
            center_snap: self.center_snap.unwrap_or(now.center_snap),
            falloff_snap: self.falloff_snap.unwrap_or(now.falloff_snap),
            wheel_offset: self.wheel_offset.unwrap_or(now.wheel_offset),
            dynamic_isolate: self.dynamic_isolate.unwrap_or(now.dynamic_isolate),
            click_isolate: self.click_isolate.unwrap_or(now.click_isolate),
            part_rows: (self.part_rows).map_or(now.part_rows, |r| {
                r.round().clamp(1.0, PART_ROWS_MAX as f32) as u32
            }),
            open_last: self.open_last.unwrap_or(now.open_last),
        }
    }
}

/// The settings window and the window's own settings after `ui.settings`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct SettingsInfo {
    pub open: bool,
    #[serde(flatten)]
    pub settings: WindowSettings,
    #[serde(flatten)]
    pub prefs: PreferencesInfo,
}

impl Command for UiSettings {
    const NAME: &'static str = "ui.settings";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes what the window shows and how it reads the keys, not the session.
    const LOGGED: bool = false;
    type Output = SettingsInfo;

    fn examples() -> Vec<Example<Self>> {
        let (snap, offset) = (Some(false), Some(WHEEL_OFFSET_MAX + 10.0));
        vec![
            Example::raw(
                "open the settings window, and a tap of a modifier key turns it on (needs the \
                 window)",
                json!({ "open": true, "sticky_modifiers": true }),
            )
            .fails_with(ErrorCode::Precondition),
            Example::new(
                "turn the sliders' center snap off (needs the window)",
                Self {
                    center_snap: snap,
                    ..Self::default()
                },
            )
            .fails_with(ErrorCode::Precondition),
            Example::new(
                "open the brush wheel further from the pen than its middle reaches",
                Self {
                    wheel_offset: offset,
                    ..Self::default()
                },
            )
            .fails_with(ErrorCode::ArgsInvalid),
            Example::raw(
                "start the window with Move, whatever brush was used last (needs the window)",
                json!({ "launch_brush": "Move" }),
            )
            .fails_with(ErrorCode::Precondition),
            Example::raw(
                "put back every setting on the Pen page, without asking (needs the window)",
                json!({ "reset": { "page": "Pen" }, "confirm": true }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(mut self, cx: &mut Ctx) -> Result<SettingsInfo, CommandError> {
        let offset = self.wheel_offset;
        if offset.is_some_and(|v| !(0.0..=WHEEL_OFFSET_MAX).contains(&v)) {
            let message = format!("must be from 0 to {WHEEL_OFFSET_MAX}");
            return Err(errors::args_invalid("/wheel_offset", message));
        }
        let rows = 1.0..=PART_ROWS_MAX as f32;
        if self.part_rows.is_some_and(|r| !rows.contains(&r)) {
            let message = format!("must be from 1 to {PART_ROWS_MAX}");
            return Err(errors::args_invalid("/part_rows", message));
        }
        // The brush at start by its name; "current" is the session's own brush, not a held
        // key's.
        self.launch_brush = match self.launch_brush.take().as_deref() {
            None => None,
            Some("last") => Some("last".into()),
            Some("current") => Some(cx.sculpt.settings.own().to_string()),
            Some(name) => Some(presets::find(name)?.preset.name.clone()),
        };
        cx.window
            .with(|w| w.settings(Some(&self)))
            .ok_or_else(no_window)?
    }
}
register_command!(UiSettings, "Settings");
