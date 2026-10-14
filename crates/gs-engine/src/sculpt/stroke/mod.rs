//! `brush.stroke`: its arguments and what it runs. Every stroke class goes the same way here
//! (gs-core's stroke pipeline tells the classes apart), so the command splits by stage instead:
//! `pin` fills in what the log line needs to replay alone, `start` checks the samples and
//! aims the stroke at the part, through a camera or at world points, and this file runs it.

mod pin;
mod start;

use gs_core::MeshStore;
use gs_protocol::{CommandError, ErrorCode, Kind, Length, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::aim::{Driver, Finished, Input};
use super::colors::Colors;
use super::detail::{DetailDone, DetailSettings, Refining};
use super::patch::{self, Settings};
use super::pen::PenSettings;
use super::{Axes, SymmetryArg};
use crate::command::PartArg;
use crate::document::pinned;
use crate::errors;
use crate::view::{NamedView, ViewSpec, own_view};
use crate::{Command, Ctx, Example, register_command};
pub(crate) use pin::{carry_presets, pin_pressure};
pub(crate) use start::aim_camera;
use start::check_samples;

/// The size of a world stroke whose size is left out.
pub(super) const DEFAULT_SIZE: Length = Length::Percent(5.0);

/// Sculpt one stroke with a brush: one undo step, however many samples.
///
/// The samples are pen samples in order: world points `[x, y, z]`, or, with a `camera`, points
/// `[x, y]` in pixels of the image the camera sees (a `view.snapshot` through the same view
/// shows exactly that image), each aimed at the part through the camera as the stroke goes.
/// Between samples the brush lays dabs at its own spacing, and strength is per length of
/// stroke, so the result does not depend on how densely the path is sampled. `brush.list` lists
/// the brushes and `brush.describe` says what each does. A stroke that changes nothing says
/// why, for example how far from the part its nearest sample passed.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushStroke {
    /// The part to sculpt. Defaults to the active part (decision 0033).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The brush: its name from `brush.list` ("Draw", "Clay Build") or its id ("brush.draw").
    pub brush: String,
    /// The samples, oldest first: 1 to 100,000.
    #[schemars(length(min = 1, max = 100_000))]
    pub samples: Vec<StrokePoint>,
    /// The camera screen samples were drawn through, as `view.snapshot` takes it: a standard
    /// view ("front"), a session view ("main", pinned to its exact camera), an exact camera or
    /// an orbit. Left out, the samples are world points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The size of the camera's image, `[width, height]` in pixels, 1 to 16384 each. Left out,
    /// `[1024, 1024]`, a `view.snapshot`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// Brush radius: scene units, or a percentage of the part's bounding-box diagonal ("5%").
    /// Left out, 5% for world samples and `size_px` for screen samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Length>,
    /// Brush radius in pixels on the camera's image, kept at every depth. Only with a camera.
    /// Left out (and no `size`), 64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 100000.0))]
    pub size_px: Option<f32>,
    /// Strength, 0 to 1 (the setting `strength`). Left out, the brush's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub strength: Option<f32>,
    /// Falloff, -100 to 100: how strength fades from the center of each dab to its rim.
    /// Positive values narrow the bump and negative ones widen it, with the same peak (the
    /// setting `tip.focal`). Left out, the brush's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub falloff: Option<f32>,
    /// Work the other way (Alt): dig instead of build, unmask instead of mask.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
    /// The Brush Modifier, -100 to 100 (the setting `modifier`): on the brushes that push
    /// (Draw, Clay, Inflate, ...), positive pinches toward the stroke and negative inflates.
    /// Left out, the brush's own (0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub modifier: Option<f32>,
    /// Any of the brush's settings for this stroke, in its preset's own names, for example
    /// `{"tip": {"focal": 40}, "stroke": {"spacing": 0.2}}` (`brush.describe` lists them).
    /// A window stroke carries what the session changed for the brush. `strength`, `falloff`
    /// and `modifier` are shortcuts for three of them; give each one way only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "patch::settings_schema")]
    pub settings: Option<Settings>,
    /// The BLAKE3 hash of the brush's preset, as 64 hex digits. Filled in by the engine
    /// before the stroke runs, so the log records which brush it used; a call whose hash no
    /// longer matches the brush is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_hash: Option<String>,
    /// The brush's preset file in full, as the stroke was recorded with it (decision 0042). A
    /// log carries it on the first stroke of each run that uses the preset; a stroke given it
    /// sculpts with it, and so does a later stroke pinning its hash, when the brush has changed
    /// since. It must hash to `preset_hash` and be the stroke's brush.
    #[serde(default, skip_serializing)]
    pub preset: Option<String>,
    /// The BLAKE3 hash of the alpha picture the brush stamps with, as 64 hex digits, pinned the
    /// same way. Left out for a brush without an alpha.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_hash: Option<String>,
    /// Repeat every dab mirrored, parallel or around an axis. Left out, none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<SymmetryArg>,
    /// Axes along which nothing moves. Left out, none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<Axes>,
    /// For world samples: the direction the stroke is seen from, which brushes that sample only
    /// the surface facing the viewer, and gestures, use. Left out, straight at the surface under
    /// the first sample. With a camera, the camera's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_dir: Option<Vec3>,
    /// Live Detail: add triangles under the brush where the surface needs them and remove them
    /// where it has too many, as the stroke goes (decision 0021). Left out, the topology never
    /// changes. Give `{}` for the session's settings (`live_detail.set`); a field left out is
    /// the brush's own, else the session's, filled in before the stroke runs so the log
    /// replays alone. Parts with detail levels or UV columns, and brushes that drag, move or
    /// paint the mask, sculpt as usual and the result says why they did not refine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_detail: Option<DetailSettings>,
    /// The seed of the stroke's random numbers: a brush that paints color rounds its colors to
    /// 8 bits once, at the stroke's end, dithered with them (decision 0039). Filled in by the
    /// engine for such a brush, so the log replays alone; refused for a brush that paints none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// The colors a brush that paints color paints with: Main, and Secondary with Alt held
    /// (decision 0039, section 7). Filled in by the engine from the session's (`color.set`)
    /// for such a brush, so the log replays alone; refused for a brush that paints none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<Colors>,
    /// The pen's feel the stroke takes: its sensitivities and lazy pressure (`pen.set`). Filled
    /// in by the engine from the session's when that is not the default, so the log replays
    /// alone; a field left out is its default. Left out, pressure as the curves give it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pen: Option<PenSettings>,
}

/// One pen sample.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrokePoint {
    /// Where: `[x, y, z]` in scene units, or with a camera `[x, y]` in pixels from the image's
    /// top-left corner (the center of pixel column c, row r is c + 0.5, r + 0.5).
    #[schemars(length(min = 2, max = 3))]
    pub position: Vec<f32>,
    /// Pen pressure, 0 to 1. Left out, 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub pressure: Option<f32>,
    /// Pen tilt in degrees, `[x, y]`. Left out, upright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<[f32; 2]>,
    /// When the pen reported it, in nanoseconds; only differences matter. No brush uses it
    /// yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_ns: Option<u64>,
    /// Whether Alt was held at this sample: the dabs this sample lays, a drag's update to it
    /// and, on the last sample, what the stroke lays when it ends work the other way. Left
    /// out, the stroke's `invert`; the window's strokes write it only where Alt changed from
    /// that (decision 0036).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    /// A point of the path on this sample, for a brush whose stroke runs through points (Clay
    /// Line, Clip Curve): the path bends through a `soft` one and corners at a `sharp` one
    /// (decision 0055). The window writes one for each Alt tap during the stroke, `sharp` for a
    /// quick second tap. Left out, none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<PathPoint>,
    /// Whether this sample moves the path instead of extending it (Space held, for a brush
    /// whose stroke runs through points, or Drag Rect): everything the stroke holds, the press,
    /// the points and the pen, moves by the pen's step since its last sample (decision 0055;
    /// 0062, row 782).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moves: bool,
}

/// A point of a curve stroke's path (decision 0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PathPoint {
    /// The path bends smoothly through it.
    Soft,
    /// A corner.
    Sharp,
}

/// What a stroke did.
#[derive(Serialize, JsonSchema)]
pub struct Stroked {
    pub part: String,
    /// The brush's name.
    pub brush: String,
    /// Samples the stroke used.
    pub samples_used: u64,
    /// Screen samples whose ray missed the part; they are left out.
    pub samples_missed: u64,
    /// Kernel steps run: dabs, drag updates, gestures.
    pub dabs: u32,
    /// The brush radius in scene units at the first sample used.
    pub radius: Option<f32>,
    /// What Live Detail did, when the call asked for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_detail: Option<DetailDone>,
    /// When the brush has changed since the stroke was recorded: that it sculpted with the
    /// preset it was recorded with, in words (decision 0042).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_preset: Option<String>,
}

impl BrushStroke {
    /// A stroke with `brush` through `samples`, everything else left out.
    pub(crate) fn new(brush: &str, samples: Vec<StrokePoint>) -> Self {
        Self {
            part: None,
            brush: brush.into(),
            samples,
            camera: None,
            viewport: None,
            size: None,
            size_px: None,
            strength: None,
            falloff: None,
            invert: false,
            modifier: None,
            settings: None,
            preset_hash: None,
            preset: None,
            alpha_hash: None,
            symmetry: None,
            lock: None,
            view_dir: None,
            live_detail: None,
            seed: None,
            colors: None,
            pen: None,
        }
    }

    /// Pins `seed` and `colors` for a brush that paints color (decision 0039, sections 6 and
    /// 7), each unless the call gave it; refuses either for a brush that paints none.
    pub(crate) fn pin_paint(&mut self, seed: u64, colors: &Colors) -> Result<(), CommandError> {
        if self.resolve()?.paints_color() {
            self.seed.get_or_insert(seed);
            self.colors.get_or_insert_with(|| colors.clone());
            return Ok(());
        }
        let given = [
            ("seed", self.seed.is_some()),
            ("colors", self.colors.is_some()),
        ];
        match given.into_iter().find(|&(_, set)| set) {
            Some((field, _)) => {
                let message = format!("{} paints no color, so it takes no {field}", self.brush);
                Err(errors::args_invalid(&format!("/{field}"), message))
            }
            None => Ok(()),
        }
    }
}

impl StrokePoint {
    /// A sample at `position` with full pressure.
    pub(crate) fn at(position: &[f32]) -> Self {
        Self {
            position: position.to_vec(),
            pressure: None,
            tilt: None,
            t_ns: None,
            invert: None,
            point: None,
            moves: false,
        }
    }

    pub(crate) fn input(&self) -> Input {
        let p = |i: usize| self.position.get(i).copied().unwrap_or(0.0);
        Input {
            at: gs_core::Vec3::new(p(0), p(1), p(2)),
            pressure: self.pressure.unwrap_or(1.0),
            tilt: self.tilt.unwrap_or_default(),
            invert: self.invert,
            point: self.point.map(|p| p == PathPoint::Sharp),
            moves: self.moves,
        }
    }
}

/// What a finished stroke reports, and why it changed nothing if it did.
pub(crate) fn outcome(
    part: &str,
    brush: String,
    done: &Finished,
    refining: &Refining,
) -> (Stroked, String) {
    let why = match (done.used, done.gap, done.radius) {
        (0, Some(gap), _) => {
            format!("the stroke never hit {part}: its nearest sample passed {gap} units from it")
        }
        (_, Some(gap), Some(r)) if gap > r => format!(
            "the stroke never reached {part}: its nearest sample was {gap} units from the \
             surface, farther than the brush radius {r}"
        ),
        _ => format!(
            "the stroke reached {part} but changed nothing: its strength is 0, a mask covers \
             everything under it, or the brush has nothing to do there"
        ),
    };
    let stroked = Stroked {
        part: part.to_string(),
        brush,
        samples_used: done.used,
        samples_missed: done.missed,
        dabs: done.report.dabs,
        radius: done.radius,
        live_detail: DetailDone::of(refining, done.report.live.as_ref()),
        recorded_preset: None,
    };
    (stroked, why)
}

impl Command for BrushStroke {
    const NAME: &'static str = "brush.stroke";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CAMERA: Option<&'static str> = Some("camera");
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Stroked;

    fn examples() -> Vec<Example<Self>> {
        // Dabs at the middle as big as the part reach all of it, whatever the part.
        let broad = |brush: &str, points: &[[f32; 3]]| {
            let mut stroke = Self::new(brush, points.iter().map(|p| StrokePoint::at(p)).collect());
            stroke.size = Some(Length::Percent(60.0));
            stroke
        };
        let middle = [[0.0, 0.0, 0.0]];
        let mut mirrored = broad("Clay Build", &[[0.2, 0.0, 0.0]]);
        mirrored.symmetry = Some(SymmetryArg {
            mirror: Axes {
                x: true,
                y: false,
                z: false,
            },
            ..SymmetryArg::default()
        });
        let mut dig = broad("brush.draw", &middle);
        dig.invert = true;
        dig.strength = Some(0.5);
        let mut hard = broad("Clay", &middle);
        hard.falloff = Some(80.0);
        let pixels = [[400.0, 480.0], [512.0, 470.0], [620.0, 490.0]];
        let mut screen = Self::new("Draw", pixels.iter().map(|p| StrokePoint::at(p)).collect());
        screen.camera = Some(ViewSpec::Named(NamedView::Front));
        screen.size_px = Some(40.0);
        let sphere = json!({ "subdivisions": 12 });
        vec![
            Example::new("draw over the whole part", broad("Draw", &middle))
                .after("primitive.sphere", sphere.clone()),
            Example::new("draw where a front snapshot shows the sphere", screen)
                .after("primitive.sphere", sphere.clone()),
            Example::new("build clay with X symmetry", mirrored)
                .after("primitive.sphere", sphere.clone()),
            Example::new("dig at half strength", dig).after("primitive.sphere", sphere.clone()),
            Example::new("clay with a hard edge", hard).after("primitive.sphere", sphere.clone()),
            Example::new("mask the whole part", broad("Mask Pen", &middle))
                .after("primitive.sphere", sphere.clone()),
            Example::new(
                "drag the whole part along z",
                broad("Move", &[[0.0, 0.0, 0.0], [0.0, 0.0, 0.3]]),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::new("a brush that does not exist", broad("Drawing", &middle))
                .after("primitive.sphere", sphere)
                .fails_with(ErrorCode::NotFound),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        self.recall(&cx.sculpt.recorded);
        self.pin_brush(&cx.sculpt.settings)?;
        self.pin_paint(cx.rng().seed(), &cx.sculpt.settings.colors)
    }

    fn run(self, cx: &mut Ctx) -> Result<Stroked, CommandError> {
        let part = pinned(self.part.clone())?;
        let recorded = self.remember(&mut cx.sculpt.recorded);
        let camera = self.camera(cx.doc.document(), cx.views)?;
        check_samples(&self.samples, if camera.is_some() { 2 } else { 3 })?;
        let levels = (cx.doc.document().get(&part))
            .is_some_and(|p| p.levels().is_some() || p.frozen().is_some());
        let t = cx.doc.transform(&part)?;
        let (here, camera) = self.in_part(cx.doc.get_part(&part)?, camera);
        let mut store = cx.doc.write_part(&part)?;
        let (done, brush, refining) = store.sculpt(|s| {
            let started = here.begin(s, camera, levels);
            let done = started.and_then(|(driver, brush, refining)| {
                let finished = run_samples(s, driver, &here.samples)?;
                Ok((finished, brush, refining))
            });
            let written = (done.as_ref()).map_or_else(|_| Vec::new(), |d| d.0.report.nodes.clone());
            (done, written)
        })?;
        let done = done.in_scene(&t);
        let (mut stroked, why) = outcome(&part, brush, &done, &refining);
        stroked.recorded_preset = recorded;
        if let Some(first) = done.first.filter(|_| !done.report.nodes.is_empty()) {
            cx.views.edited(&own_view(cx.origin), first.to_array());
        }
        cx.noop_reason(why);
        Ok(stroked)
    }
}
register_command!(BrushStroke, "Stroke");

/// Runs a whole stroke through a driver on `store`. On failure the caller's step takes the
/// stroke back with everything else.
fn run_samples(
    store: &mut MeshStore,
    mut driver: Driver,
    samples: &[StrokePoint],
) -> Result<Finished, CommandError> {
    for (i, s) in samples.iter().enumerate() {
        if let Err(e) = driver.feed(store, s.input()) {
            return Err(errors::args_invalid(&format!("/samples/{i}"), e.message));
        }
    }
    Ok(driver.finish(store))
}
