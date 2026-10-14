//! `symmetry.set`: the session's symmetry, which the window's pen strokes with (decision 0044).
//! The session keeps a master switch, each axis on or off with its type, and the radial count;
//! a stroke carries the form they come to ([`SymmetryArg`]), empty while the switch is off.
//! `brush.stroke` never reads the session, so the log replays without it.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Axis, Radial, SymmetryArg};
use crate::{Command, Ctx, Example, errors, register_command};

/// The axes, by index.
const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];
/// The axes' names, by index, as arguments name them.
const NAMES: [&str; 3] = ["x", "y", "z"];

/// What an axis does while it is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SymmetryType {
    /// Repeat each dab mirrored across the plane through the part's origin square to the axis.
    #[default]
    Mirror,
    /// Repeat each dab at the mirrored place, going the way the stroke goes: a Move dragged
    /// toward -X moves its copy toward -X too.
    Parallel,
    /// Repeat each dab `radial_count` times around the axis.
    Radial,
}

/// The session's symmetry. Every field is kept while it is not in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the master and one switch per axis, each a control of its own"
)]
pub struct SymmetrySettings {
    /// The master switch: off, nothing repeats, and the axes keep their settings.
    pub on: bool,
    /// Axis X is on.
    pub x: bool,
    /// Axis Y is on.
    pub y: bool,
    /// Axis Z is on.
    pub z: bool,
    pub x_type: SymmetryType,
    pub y_type: SymmetryType,
    pub z_type: SymmetryType,
    /// Copies around the axis on as Radial, 2 to 64.
    pub radial_count: u32,
}

impl Default for SymmetrySettings {
    fn default() -> Self {
        Self {
            on: true,
            x: false,
            y: false,
            z: false,
            x_type: SymmetryType::Mirror,
            y_type: SymmetryType::Mirror,
            z_type: SymmetryType::Mirror,
            radial_count: 8,
        }
    }
}

impl SymmetrySettings {
    /// Each axis, on or off, and its type.
    fn axes(&self) -> [(bool, SymmetryType); 3] {
        [
            (self.x, self.x_type),
            (self.y, self.y_type),
            (self.z, self.z_type),
        ]
    }

    /// Axis `a`'s switch and type.
    fn axis(&mut self, a: usize) -> (&mut bool, &mut SymmetryType) {
        match a {
            0 => (&mut self.x, &mut self.x_type),
            1 => (&mut self.y, &mut self.y_type),
            _ => (&mut self.z, &mut self.z_type),
        }
    }

    /// The axis on as Radial, master switch or not.
    pub fn radial_axis(&self) -> Option<Axis> {
        let on = self
            .axes()
            .into_iter()
            .position(|a| a == (true, SymmetryType::Radial));
        on.map(|a| AXES[a])
    }

    /// What a stroke repeats with: each axis on in its type, nothing while the switch is off.
    pub fn stroke(&self) -> SymmetryArg {
        let mut out = SymmetryArg::default();
        let on = self.axes().into_iter().enumerate();
        for (a, (_, kind)) in on.filter(|&(_, (on, _))| on && self.on) {
            match kind {
                SymmetryType::Mirror => *out.mirror.at(AXES[a]) = true,
                SymmetryType::Parallel => *out.parallel.at(AXES[a]) = true,
                SymmetryType::Radial => {
                    let count = self.radial_count;
                    out.radial = Some(Radial {
                        axis: AXES[a],
                        count,
                    });
                }
            }
        }
        out
    }

    /// Takes a stroke's form `s` (`brush.set`'s `symmetry`): the switch on, each axis it names
    /// on in that type, the others off with their types kept, and its radial count. An axis
    /// with two types is refused, as the session gives each one (decision 0044, section 4).
    pub(super) fn take(&mut self, s: SymmetryArg) -> Result<(), CommandError> {
        let (m, p) = (s.mirror.array(), s.parallel.array());
        let r = s.radial.map(|r| r.axis as usize);
        if let Some(r) = s.radial {
            self.radial_count = count(r.count as f32, "/symmetry/radial/count")?;
            self.make_radial(r.axis as usize);
        }
        for a in 0..3 {
            let (on, kind) = self.axis(a);
            *on = m[a] || p[a] || r == Some(a);
            match (m[a], p[a], r == Some(a)) {
                (true, false, false) => *kind = SymmetryType::Mirror,
                (false, true, false) => *kind = SymmetryType::Parallel,
                (false, false, _) => {}
                _ => {
                    let axis = NAMES[a].to_uppercase();
                    let message = format!("axis {axis} has one type: Mirror, Parallel or Radial");
                    return Err(errors::args_invalid("/symmetry", message));
                }
            }
        }
        self.on = true;
        Ok(())
    }

    /// Turns axis `a` on or off, in its type (`brush.set`'s `flip_mirror`).
    pub(super) fn flip(&mut self, a: Axis) {
        let (on, _) = self.axis(a as usize);
        *on = !*on;
    }

    /// Makes axis `a` Radial and the one that was Radial Mirror: one axis at most is Radial.
    fn make_radial(&mut self, a: usize) {
        for b in 0..3 {
            let (_, kind) = self.axis(b);
            if b == a {
                *kind = SymmetryType::Radial;
            } else if *kind == SymmetryType::Radial {
                *kind = SymmetryType::Mirror;
            }
        }
    }
}

/// `n` rounded to a radial count, or `E_ARGS_INVALID` at `at` outside 2 to 64.
fn count(n: f32, at: &str) -> Result<u32, CommandError> {
    let whole = n.round();
    if (2.0..=64.0).contains(&whole) {
        Ok(whole as u32)
    } else {
        let message = format!("the radial count must be 2 to 64, not {n}");
        Err(errors::args_invalid(at, message))
    }
}

/// Change the session's symmetry, which the window's pen strokes with: the master switch, each
/// axis on or off, each axis' type (`mirror` repeats a dab mirrored across the plane through
/// the part's origin; `parallel` at the mirrored place, going the way the stroke goes;
/// `radial` around the axis, `radial_count` times) and the radial count. Leave a setting out to
/// keep it; every setting is kept while it is not in use, so turning the switch off and on
/// brings the axes back. One axis at most is Radial: making one Radial makes the one that was
/// Mirror. `brush.stroke` never reads these: a stroke carries its own `symmetry`, which the
/// window fills from here. Answers with every setting.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymmetrySet {
    /// The master switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Axis X on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<bool>,
    /// Axis Y on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<bool>,
    /// Axis Z on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<bool>,
    /// Axis X's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_type: Option<SymmetryType>,
    /// Axis Y's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_type: Option<SymmetryType>,
    /// Axis Z's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_type: Option<SymmetryType>,
    /// Copies around the axis on as Radial, 2 to 64; a fraction rounds to the nearest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 2.0, max = 64.0))]
    pub radial_count: Option<f32>,
}

impl Command for SymmetrySet {
    const NAME: &'static str = "symmetry.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = SymmetrySettings;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("mirror across X", json!({ "x": true })),
            Example::raw(
                "repeat across X going the same way, as a Move dragging both ears left",
                json!({ "x": true, "x_type": "parallel" }),
            ),
            Example::raw(
                "8 copies around Y",
                json!({ "y": true, "y_type": "radial", "radial_count": 8 }),
            ),
            Example::raw(
                "no symmetry for now, kept for later",
                json!({ "on": false }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<SymmetrySettings, CommandError> {
        let mut next = cx.sculpt.settings.symmetry;
        next.on = self.on.unwrap_or(next.on);
        let given = [
            (self.x, self.x_type),
            (self.y, self.y_type),
            (self.z, self.z_type),
        ];
        let mut radial = None;
        for (a, (on, kind)) in given.into_iter().enumerate() {
            let (was_on, was) = next.axis(a);
            *was_on = on.unwrap_or(*was_on);
            match kind {
                Some(SymmetryType::Radial) if radial.is_some() => {
                    let message = "one axis at most is Radial";
                    return Err(errors::args_invalid(
                        &format!("/{}_type", NAMES[a]),
                        message,
                    ));
                }
                Some(SymmetryType::Radial) => radial = Some(a),
                Some(kind) => *was = kind,
                None => {}
            }
        }
        if let Some(a) = radial {
            next.make_radial(a);
        }
        if let Some(n) = self.radial_count {
            next.radial_count = count(n, "/radial_count")?;
        }
        cx.sculpt.settings.symmetry = next;
        Ok(next)
    }
}
register_command!(SymmetrySet, "Set symmetry");
