//! The pen's feel (decision 0036, section 3 as amended): how this pen in this hand reaches every
//! brush. The sensitivities change the stroke's pressure curves once, when it begins; lazy
//! pressure eases each sample's pressure as the samples are fed. At the defaults nothing changes.

use crate::brush::Pressure;
use crate::curve::Curve;

/// The pen's feel, as fractions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenFeel {
    /// Size Sensitivity, -1 to 1: how much of the size curve in effect the stroke keeps
    /// ([`Curve::with_sensitivity`]); below 0 the curve runs backward.
    pub size: f32,
    /// Strength Sensitivity, -1 to 1, on the strength curve.
    pub strength: f32,
    /// Color Sensitivity, -1 to 1: paint's share of the strength curve. While it equals
    /// `strength`, paint takes the dab's own amount.
    pub color: f32,
    /// Lazy pressure, 0 to 1: each sample's pressure moves `1 - lazy` of the way from the last
    /// one's toward its own; the stroke's first sample is taken as it is.
    pub lazy: f32,
}

impl Default for PenFeel {
    /// Every sensitivity 1 and lazy pressure 0: the stroke as its curves give it.
    fn default() -> PenFeel {
        PenFeel {
            size: 1.0,
            strength: 1.0,
            color: 1.0,
            lazy: 0.0,
        }
    }
}

impl PenFeel {
    /// Why no stroke can take this feel, if none can.
    pub(super) fn check(&self) -> Result<(), String> {
        let sensitivities = [self.size, self.strength, self.color];
        if !sensitivities.iter().all(|k| (-1.0..=1.0).contains(k)) {
            return Err(format!(
                "the pen's sensitivities must be from -1 to 1, got {sensitivities:?}"
            ));
        }
        if !(0.0..=1.0).contains(&self.lazy) {
            return Err(format!(
                "lazy pressure must be from 0 to 1, got {}",
                self.lazy
            ));
        }
        Ok(())
    }

    /// Feels `pressure`'s size and strength curves through the sensitivities, and returns the
    /// curve paint takes when Color Sensitivity differs from Strength Sensitivity.
    pub(super) fn feel(&self, pressure: &mut Pressure) -> Option<Curve> {
        pressure.size = pressure.size.with_sensitivity(self.size);
        let color = (self.color.to_bits() != self.strength.to_bits())
            .then(|| pressure.strength.with_sensitivity(self.color));
        pressure.strength = pressure.strength.with_sensitivity(self.strength);
        color
    }

    /// The pressure a sample at `p` takes after a sample that took `last`, if any.
    pub(super) fn ease(&self, last: Option<f32>, p: f32) -> f32 {
        last.map_or(p, |q| q + (1.0 - self.lazy) * (p - q))
    }
}
