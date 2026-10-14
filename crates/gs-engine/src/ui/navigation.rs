//! `ui.navigation`: how the window turns pen and mouse input into view commands (decision
//! 0017). The window keeps these settings with its layout state; the moves themselves are the
//! View commands, so this changes nothing in the session.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::{Command, Ctx, Example, errors, register_command};

/// What a click of the pen's barrel button (or the right mouse button) on the 3D view does when
/// it does not drag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BarrelClick {
    /// Open or let go of the brush wheel.
    #[default]
    Wheel,
    /// Open the command palette.
    Palette,
    /// Open or close the quick menu.
    Quick,
    /// Nothing.
    None,
}

/// What the pen's barrel button 2 (and the middle mouse button, which the window reads as it)
/// does: pan the view, or stand for a modifier key while it is down, doing all the key does
/// (`interface.modifier-keys`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BarrelTwo {
    /// Pan the view.
    #[default]
    Pan,
    /// Shift: the Smooth slot's brush.
    Shift,
    /// Ctrl: the Mask slot's brush.
    Ctrl,
    /// Alt: the stroke works the other way.
    Alt,
}

/// What the pen's eraser end does once it touches (decision 0036, section 3 as amended): stroke
/// as the tip does, or stand for held keys, doing all they do (`interface.modifier-keys`). The
/// tablet only says which end is in use when it touches, so a hovering eraser reads as the tip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Eraser {
    /// Stroke as the tip does.
    #[default]
    Tip,
    /// Shift: the Smooth slot's brush.
    Shift,
    /// Ctrl: the Mask slot's brush.
    Ctrl,
    /// Alt: the stroke works the other way.
    Alt,
    /// Ctrl and Alt: the Mask slot's brush the other way, erasing the mask.
    CtrlAlt,
}

impl Eraser {
    /// The keys it stands for: Shift, Ctrl and Alt.
    pub fn keys(self) -> [bool; 3] {
        match self {
            Eraser::Tip => [false; 3],
            Eraser::Shift => [true, false, false],
            Eraser::Ctrl => [false, true, false],
            Eraser::Alt => [false, false, true],
            Eraser::CtrlAlt => [false, true, true],
        }
    }
}

/// The window's navigation settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Navigation {
    /// What a barrel click without a drag does.
    pub barrel_click: BarrelClick,
    /// What barrel button 2 does: pans, or stands for a modifier key.
    pub barrel_two: BarrelTwo,
    /// What the eraser end does: strokes as the tip, or stands for held keys.
    pub eraser: Eraser,
    /// A pen press on empty canvas turns the view instead of sculpting (the classic
    /// navigation); a locked view sculpts anyway.
    pub classic: bool,
    /// Degrees the view turns per point dragged.
    pub orbit_speed: f32,
    /// How much the distance changes per point dragged: 0.005 is half a percent.
    pub dolly_speed: f32,
    /// How much the distance changes per wheel notch: 0.1 is 10%.
    pub wheel_step: f32,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            barrel_click: BarrelClick::Wheel,
            barrel_two: BarrelTwo::Pan,
            eraser: Eraser::Tip,
            classic: true,
            // 0.008 radians, the window's speed before navigation became commands.
            orbit_speed: 0.458_366_23,
            dolly_speed: 0.005,
            wheel_step: 0.1,
        }
    }
}

/// Change how the window navigates: what a barrel click does (`wheel`, the brush wheel;
/// `palette`, the command palette; `quick`, the quick menu; `none`), what barrel button 2 does
/// (`pan`, or `shift`, `ctrl` or `alt`: it stands for that key while down), what the eraser
/// end does once it touches (`tip`, or `shift`, `ctrl`, `alt` or `ctrl_alt`: it stands for
/// those keys), whether a pen press on empty canvas turns the view (the classic scheme),
/// and the speeds in points. A barrel 1 drag always navigates: it orbits, with Alt it pans,
/// with Ctrl it zooms. Left out, a setting stays. Answers with every setting. Kept with the
/// window's layout state; not logged.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiNavigation {
    /// What a barrel click without a drag does: `wheel`, `palette`, `quick` or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub barrel_click: Option<BarrelClick>,
    /// What barrel button 2 does: `pan`, or `shift`, `ctrl` or `alt`, the key it stands for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub barrel_two: Option<BarrelTwo>,
    /// What the eraser end does once it touches: `tip`, or `shift`, `ctrl`, `alt` or
    /// `ctrl_alt`, the keys it stands for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eraser: Option<Eraser>,
    /// True: a pen press on empty canvas turns the view (the classic scheme).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classic: Option<bool>,
    /// Degrees per point dragged, above 0 and at most 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0), range(max = 10.0))]
    pub orbit_speed: Option<f32>,
    /// Fraction of the distance per point dragged, above 0 and at most 0.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0), range(max = 0.1))]
    pub dolly_speed: Option<f32>,
    /// Fraction of the distance per wheel notch, above 0 and below 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0), range(max = 0.9))]
    pub wheel_step: Option<f32>,
}

impl UiNavigation {
    /// `now` with this change.
    pub fn apply(&self, now: Navigation) -> Navigation {
        Navigation {
            barrel_click: self.barrel_click.unwrap_or(now.barrel_click),
            barrel_two: self.barrel_two.unwrap_or(now.barrel_two),
            eraser: self.eraser.unwrap_or(now.eraser),
            classic: self.classic.unwrap_or(now.classic),
            orbit_speed: self.orbit_speed.unwrap_or(now.orbit_speed),
            dolly_speed: self.dolly_speed.unwrap_or(now.dolly_speed),
            wheel_step: self.wheel_step.unwrap_or(now.wheel_step),
        }
    }

    /// Every speed is in range.
    fn check_speeds(&self) -> Result<(), CommandError> {
        let speeds = [
            ("/orbit_speed", self.orbit_speed, 10.0),
            ("/dolly_speed", self.dolly_speed, 0.1),
            ("/wheel_step", self.wheel_step, 0.9),
        ];
        for (path, value, max) in speeds {
            if value.is_some_and(|v| !(v > 0.0 && v <= max)) {
                return Err(errors::args_invalid(
                    path,
                    format!("must be above 0 and at most {max}"),
                ));
            }
        }
        Ok(())
    }
}

impl Command for UiNavigation {
    const NAME: &'static str = "ui.navigation";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes how the window reads input, not the session: the moves it leads to are
    /// logged themselves.
    const LOGGED: bool = false;
    type Output = Navigation;

    fn examples() -> Vec<Example<Self>> {
        let palette = Self {
            barrel_click: Some(BarrelClick::Palette),
            ..Self::default()
        };
        vec![
            Example::new(
                "a barrel click opens the palette (needs the window)",
                palette,
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Navigation, CommandError> {
        self.check_speeds()?;
        cx.window
            .with(|w| w.navigation(Some(&self)))
            .ok_or_else(no_window)
    }
}
register_command!(UiNavigation, "Set navigation");
