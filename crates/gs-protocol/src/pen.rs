//! Pen input as one shared type. The window turns the tablet's events into `PenSample`s, and
//! tests, replays, the CLI and MCP inject the same samples through `pen.input`, so every pen
//! path above the platform layer is one path (ARCHITECTURE section 9).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which end of the pen is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PenTool {
    /// The tip.
    #[default]
    Pen,
    /// The eraser end. Only known once it touches: a hovering eraser reads as a pen.
    Eraser,
}

/// One pen report: everything the tablet said in one compositor frame, merged.
///
/// Coordinates are physical window pixels from the window's top-left corner (window points
/// times the display's pixel density). Pressure is the tablet's raw 0 to 1 value; pressure
/// curves are applied later, by brushes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PenSample {
    /// When the tablet reported it, in nanoseconds on the platform clock (1 ms steps on
    /// Wayland). Only differences between samples matter. Left out, 0.
    #[serde(default)]
    pub t_ns: u64,
    /// Horizontal position in physical window pixels, growing to the right.
    pub x: f64,
    /// Vertical position in physical window pixels, growing downward.
    pub y: f64,
    /// Tip pressure from 0 to 1. Left out, 1: a sample that says nothing of pressure presses
    /// fully, as a `brush.stroke` point does (decision 0036, section 3 as amended).
    #[serde(default = "full")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub pressure: f32,
    /// Tilt in degrees, positive when the pen leans to the right. Left out, 0.
    #[serde(default)]
    #[schemars(range(min = -90.0, max = 90.0))]
    pub tilt_x: f32,
    /// Tilt in degrees, positive when the pen leans toward the user. Left out, 0.
    #[serde(default)]
    #[schemars(range(min = -90.0, max = 90.0))]
    pub tilt_y: f32,
    /// Which end of the pen. Left out, the tip.
    #[serde(default)]
    pub tool: PenTool,
    /// Barrel buttons held down: bit 0 is button 1, bit 1 is button 2. Left out, none.
    #[serde(default)]
    pub buttons: u8,
    /// True while the tip touches the tablet. Left out, false (hovering).
    #[serde(default)]
    pub contact: bool,
    /// False on the last sample of a hover session, when the pen leaves the tablet's range.
    /// Left out, true.
    #[serde(default = "in_range")]
    pub in_proximity: bool,
}

fn in_range() -> bool {
    true
}

fn full() -> f32 {
    1.0
}

impl PenSample {
    /// A pen hovering at (`x`, `y`) with nothing pressed: the state a hover session starts in.
    pub fn hover(t_ns: u64, x: f64, y: f64) -> Self {
        Self {
            t_ns,
            x,
            y,
            pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            tool: PenTool::Pen,
            buttons: 0,
            contact: false,
            in_proximity: true,
        }
    }
}
