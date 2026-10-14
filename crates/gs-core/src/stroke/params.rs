//! What a stroke takes and gives back: its samples, the camera it is drawn through, the colors
//! a paint stroke uses, everything else beside its preset (checked before it starts), what it
//! did, and why it was refused.

use std::fmt;
use std::sync::Arc;

use crate::alpha::Pyramid;
use crate::brush::BrushPreset;
use crate::config::NodeId;
use crate::history::StepSummary;
use crate::live::{LiveDetail, LiveReport};
use crate::math::Vec3;
use crate::stroke::PenFeel;
use crate::symmetry::Symmetry;

/// One input sample, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSample {
    pub pos: Vec3,
    /// Pen pressure, 0 to 1; 1 for a mouse.
    pub pressure: f32,
    /// Pen tilt in degrees, carried for presets that will use it.
    pub tilt: [f32; 2],
    /// Brush radius at this sample, in world units, before the size pressure curve.
    pub radius: f32,
}

/// The camera a stroke is drawn through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The direction the camera looks.
    pub dir: Vec3,
    /// The eye, for a perspective camera; gestures sweep along rays from it.
    pub eye: Option<Vec3>,
}

/// The colors a stroke that paints uses (decision 0039, section 7), as the color column holds
/// them: sRGB bytes, red lowest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintColors {
    /// What the brush paints.
    pub main: u32,
    /// What it paints with Alt held.
    pub secondary: u32,
    /// Gradient: paint Main at the brush's centre and Secondary at its rim, mixed by the
    /// falloff in linear light (Alt swaps the two).
    pub gradient: bool,
    /// Alternate: Alt swaps Main and Secondary. Off, Alt does nothing for color (decision
    /// 0039, section 11).
    pub alternate: bool,
}

impl Default for PaintColors {
    /// White and black, without Gradient, with Alternate.
    fn default() -> PaintColors {
        PaintColors {
            main: u32::MAX,
            secondary: 0xff00_0000,
            gradient: false,
            alternate: true,
        }
    }
}

/// Everything about a stroke that is not in its (resolved) preset.
#[derive(Clone, Debug)]
pub struct StrokeParams {
    /// Alt: work the other way (a Subtract preset is inverted once more).
    pub invert: bool,
    pub view: View,
    /// The screen's up, for a stroke drawn through a camera: a rectangle's sides are square to
    /// that screen (decision 0051, section 4). Left out, a rectangle's sides run along the
    /// gesture plane's own axes.
    pub up: Option<Vec3>,
    pub symmetry: Symmetry,
    /// No movement along these axes.
    pub lock: [bool; 3],
    /// The picture the preset's `tip.alpha` names, through its tone settings: the caller
    /// (the engine) resolves the name. Left out, the stroke stamps a plain round tip.
    pub alpha: Option<Arc<Pyramid>>,
    /// Refine the mesh under every dab (decision 0021). Ignored for a preset that never
    /// refines ([`live::refuses`](crate::live::refuses)); left out, the topology never changes.
    pub live_detail: Option<LiveDetail>,
    /// The seed of the stroke command's random stream ([`CommandRng`](crate::rng::CommandRng)):
    /// a paint stroke's dither when it rounds its colors at its end (decision 0039, section 6).
    pub seed: u64,
    /// The session's colors, for a brush that paints color.
    pub colors: PaintColors,
    /// The pen's feel: its sensitivities and lazy pressure.
    pub pen: PenFeel,
}

impl StrokeParams {
    /// Defaults for a stroke seen through `view`: no screen, no symmetry, nothing locked, seed
    /// 0, white and black, the pen's feel at its defaults.
    pub fn new(view: View) -> StrokeParams {
        StrokeParams {
            invert: false,
            view,
            up: None,
            symmetry: Symmetry::default(),
            lock: [false; 3],
            alpha: None,
            live_detail: None,
            seed: 0,
            colors: PaintColors::default(),
            pen: PenFeel::default(),
        }
    }
}

/// What a stroke did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StrokeReport {
    /// Kernel steps run (dabs, drag updates, gestures).
    pub dabs: u32,
    /// Nodes whose positions or attributes were written, ascending.
    pub nodes: Vec<NodeId>,
    /// The most nodes one step wrote: the work counter that keeps small dabs small.
    pub max_dab_nodes: usize,
    /// Vertices reached by walking along edges from the stroke start (the connected move and
    /// the topology automask): the work counter that keeps those walks to the footprint.
    pub walked: usize,
    /// The finest alpha level any dab read (0 is the full-size picture), when the stroke has an
    /// alpha: the work counter that keeps a small dab off a large picture's full size.
    pub alpha_finest: Option<usize>,
    /// What Live Detail did, when the stroke refined.
    pub live: Option<LiveReport>,
    /// The history step, once the stroke finished.
    pub step: Option<StepSummary>,
}

/// Why a stroke or a batch of samples was refused. Nothing was changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrokeError {
    pub message: String,
}

impl fmt::Display for StrokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StrokeError {}

pub(super) fn refuse<T>(message: String) -> Result<T, StrokeError> {
    Err(StrokeError { message })
}

/// Refuses parameters no stroke can run with (see [`Stroke::begin`](super::Stroke::begin)).
pub(super) fn check(preset: &BrushPreset, params: &StrokeParams) -> Result<(), StrokeError> {
    let strength = preset.strength;
    let dir = params.view.dir;
    let sym = params.symmetry;
    if !(0.0..=1.0).contains(&strength) {
        return refuse(format!("strength must be from 0 to 1, got {strength}"));
    }
    if !(dir.len2() > 0.0 && dir.len2().is_finite()) {
        return refuse("the view direction must be a finite direction, not zero".into());
    }
    if sym
        .radial
        .is_some_and(|(axis, n)| axis > 2 || !(2..=64).contains(&n))
    {
        return refuse("radial symmetry needs an axis 0 to 2 and 2 to 64 copies".into());
    }
    if (0..3).any(|a| sym.mirror[a] && sym.parallel[a]) {
        return refuse("a symmetry plane is Mirror or Parallel, not both".into());
    }
    if !(sym.clip >= 0.0 && sym.clip.is_finite()) {
        return refuse("the symmetry clamp distance must be finite and not negative".into());
    }
    if preset.tip.wrap > 1 && sym.passes().len() > 1 {
        return refuse(format!(
            "{} repeats its stroke over the part's tiles (tip.wrap {}), and a symmetry copy \
             would double a dab where it meets a repeated one: turn symmetry off, or set \
             tip.wrap to 1",
            preset.name, preset.tip.wrap
        ));
    }
    if let Some(pyramid) = &params.alpha {
        let Some(spec) = &preset.tip.alpha else {
            return refuse(format!(
                "an alpha was given, but {} stamps none",
                preset.name
            ));
        };
        let other_picture = spec
            .hash_named()
            .is_some_and(|h| h != pyramid.picture_hash());
        if other_picture || pyramid.tone_key() != spec.tone_key() {
            return refuse(format!(
                "the alpha given is not {} through {}'s tone settings",
                spec.image, preset.name
            ));
        }
    }
    if let Some(Err(e)) = params.live_detail.as_ref().map(LiveDetail::check) {
        return refuse(e);
    }
    if let Err(e) = params.pen.check() {
        return refuse(e);
    }
    Ok(())
}
