//! The brush tip's falloff: how strength fades from the center of a dab to its rim. The one home
//! of falloff; every kernel weighs vertices through
//! [`Tip::falloff`].
//!
//! For a vertex at distance `d` from the dab center, in brush radii:
//!
//! 1. `d >= 1` gives 0.
//! 2. Hardness `h` keeps full strength out to `h` and squeezes the curve into the rest:
//!    `d = max(0, (d - h) / (1 - h))`.
//! 3. Falloff `f` (the Falloff setting, -100 to 100) bends the curve: `d = d ^ (2 ^ (-f / 50))`,
//!    so positive values narrow the bump and negative values widen it, with the same peak.
//! 4. The profile curve gives the strength: smooth `3d^4 - 4d^3 + 1`, sphere `sqrt(1 - d^2)`,
//!    sharp `(1 - d)^2`, linear `1 - d`, constant `1`, the presets soft `1 - 3d^2 + 2d^3`, hard
//!    `1 - d^8`, flat top `(1 - d^4)^2` and sharp center `(1 - d)^4`, or a custom curve, held to
//!    0 to 1.
//! 5. With Curve By Pen, the Zero Curve `z` at the same `d` is the strength at no pressure, and a
//!    dab whose Pen Curve gives `k` takes `z + k (e - z)`, `e` the profile's (decision 0059).

use crate::curve::Curve;

/// The shape of the falloff curve.
#[derive(Clone, Debug, PartialEq)]
pub enum Profile {
    Smooth,
    Sphere,
    Sharp,
    Linear,
    Constant,
    Soft,
    Hard,
    FlatTop,
    SharpCenter,
    Custom(Curve),
}

impl Profile {
    fn at(&self, d: f32) -> f32 {
        let d2 = d * d;
        match self {
            Profile::Smooth => d * d * d * (3.0 * d - 4.0) + 1.0,
            Profile::Sphere => (1.0 - d * d).sqrt(),
            Profile::Sharp => (1.0 - d) * (1.0 - d),
            Profile::Linear => 1.0 - d,
            Profile::Constant => 1.0,
            Profile::Soft => d2 * (2.0 * d - 3.0) + 1.0,
            Profile::Hard => 1.0 - d2 * d2 * d2 * d2,
            Profile::FlatTop => (1.0 - d2 * d2) * (1.0 - d2 * d2),
            Profile::SharpCenter => (1.0 - d) * (1.0 - d) * (1.0 - d) * (1.0 - d),
            Profile::Custom(c) => c.eval(d),
        }
    }
}

/// A brush tip: profile, hardness, Falloff and Curve By Pen's Zero Curve.
#[derive(Clone, Debug, PartialEq)]
pub struct Tip {
    profile: Profile,
    hardness: f32,
    /// The Falloff exponent, `None` for Falloff 0.
    gamma: Option<f32>,
    /// The Zero Curve, the falloff at no pressure; `None` with Curve By Pen off.
    zero: Option<Curve>,
}

impl Tip {
    /// `hardness` from 0 to just under 1, `focal` (the Falloff setting) from -100 to 100. A
    /// custom curve's points are held to 0 to 1 here, once, so its falloff stays in 0 to 1 at
    /// every vertex without a check there (presets refuse such a curve; this guards code).
    pub fn new(profile: Profile, hardness: f32, focal: f32) -> Tip {
        let profile = match profile {
            Profile::Custom(c) => Profile::Custom(c.clamped()),
            other => other,
        };
        Tip {
            profile,
            hardness,
            gamma: (focal != 0.0).then(|| libm::exp2f(-focal / 50.0)),
            zero: None,
        }
    }

    /// This tip with Curve By Pen on: `zero`, its points held to 0 to 1, is the falloff at no
    /// pressure.
    #[must_use]
    pub fn by_pen(mut self, zero: &Curve) -> Tip {
        self.zero = Some(zero.clamped());
        self
    }

    /// This tip with Accurate Curve on: a custom profile and the Zero Curve are read exactly,
    /// not through their tables (decision 0059).
    #[must_use]
    pub fn accurate(mut self) -> Tip {
        if let Profile::Custom(c) = &self.profile {
            self.profile = Profile::Custom(c.exact());
        }
        self.zero = self.zero.as_ref().map(Curve::exact);
        self
    }

    /// Strength at distance `d`, in brush radii: 1 or less at the center, 0 from the rim out.
    pub fn falloff(&self, d: f32) -> f32 {
        self.falloff_at(d, 1.0)
    }

    /// Strength at distance `d` for a dab whose Pen Curve gives `share`: the profile's at 1,
    /// the Zero Curve's at 0 with Curve By Pen on, and the profile's at any share with it off.
    pub fn falloff_at(&self, d: f32, share: f32) -> f32 {
        if d.is_nan() || d >= 1.0 {
            return 0.0;
        }
        let d = ((d - self.hardness) / (1.0 - self.hardness)).max(0.0);
        let d = self.gamma.map_or(d, |g| libm::powf(d, g));
        let e = self.profile.at(d);
        match &self.zero {
            Some(z) if share < 1.0 => {
                let z = z.eval(d);
                z + share * (e - z)
            }
            _ => e,
        }
    }
}
