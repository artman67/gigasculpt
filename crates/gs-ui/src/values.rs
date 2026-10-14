//! The three brush values the interface sets at the cursor and on its sliders: size, strength
//! and Softness (`brush.set`'s `falloff`). Each is one argument of `brush.set`; this is the one
//! place that says where each starts, what the window calls it, how far it may go and how fast a
//! held key moves it.

use gs_engine::BrushState;

/// A brush value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Size,
    Strength,
    Falloff,
}

impl Value {
    /// The value a `brush.set` argument sets, if it is one of these.
    pub fn from_arg(arg: &str) -> Option<Value> {
        match arg {
            "size_px" => Some(Value::Size),
            "strength" => Some(Value::Strength),
            "falloff" => Some(Value::Falloff),
            _ => None,
        }
    }

    /// Its `brush.set` argument.
    pub fn arg(self) -> &'static str {
        match self {
            Value::Size => "size_px",
            Value::Strength => "strength",
            Value::Falloff => "falloff",
        }
    }

    /// What it is called in the window: Falloff is Softness everywhere (decision 0060, amended
    /// 2026-10-13).
    pub fn title(self) -> &'static str {
        match self {
            Value::Size => "Size",
            Value::Strength => "Strength",
            Value::Falloff => "Softness",
        }
    }

    /// The layout piece that draws its slider (`direction::WIDGETS`), which a lookup goes by
    /// rather than by its title.
    pub fn name(self) -> &'static str {
        match self {
            Value::Size => "size",
            Value::Strength => "strength",
            Value::Falloff => "falloff",
        }
    }

    /// Its value in `brush`. A size set as a length shows as the default pixels.
    pub fn of(self, brush: &BrushState) -> f32 {
        match self {
            Value::Size => brush.size_px.unwrap_or(64.0),
            Value::Strength => brush.strength,
            Value::Falloff => brush.falloff,
        }
    }

    /// `brush` showing `value` for this.
    pub fn put(self, brush: &mut BrushState, value: f32) {
        match self {
            Value::Size => brush.size_px = Some(value),
            Value::Strength => brush.strength = value,
            Value::Falloff => brush.falloff = value,
        }
    }

    /// The lowest and highest it can be.
    pub fn range(self) -> [f32; 2] {
        match self {
            Value::Size => [1.0, 10000.0],
            Value::Strength => [0.0, 1.0],
            Value::Falloff => [-100.0, 100.0],
        }
    }

    /// How much it changes per point the pointer moves while its key is held, at `ppp` pixels
    /// per point: the size follows the pointer pixel for pixel; strength and Softness cross
    /// their whole range in 400 points.
    pub fn rate(self, ppp: f32) -> f32 {
        let [low, high] = self.range();
        match self {
            Value::Size => ppp,
            Value::Strength | Value::Falloff => (high - low) / 400.0,
        }
    }

    /// What one of a number typed for it is worth, as the window writes it: strength in percent.
    pub fn unit(self) -> f32 {
        match self {
            Value::Strength => 0.01,
            Value::Size | Value::Falloff => 1.0,
        }
    }

    /// The value as the window writes it.
    pub fn text(self, value: f32) -> String {
        match self {
            Value::Size => format!("{value:.0} px"),
            Value::Strength => format!("{:.0}%", value * 100.0),
            Value::Falloff => format!("{value:+.0}"),
        }
    }
}
