//! `brush.set`: the session's brush settings, which the window strokes with when the pen goes
//! down. Commands set them, so a key in the window, the CLI and an agent set them the same way.
//! `brush.stroke` never reads them: a stroke names everything it uses, so the log replays
//! without them.
//!
//! The size and symmetry are shared by every brush. Everything else belongs to the brush: the
//! session keeps one patch per brush, the settings changed in this session in the preset's own
//! names, and the window's stroke carries it (decision 0018).

use std::collections::BTreeMap;

use gs_core::BrushPreset;
use gs_core::brush::{StrokeType, carried};
use gs_core::curve::Curve;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Length};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::colors::Colors;
use super::detail::{DetailSession, DetailState};
use super::patch::{self, Settings};
use super::pen::PenSettings;
use super::slots::Slots;
use super::steps::{self, Increments};
use super::stroke::pin_pressure;
use super::{Axis, SymmetryArg, SymmetrySettings, presets};
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// The brush size of a session that never set one: 64 screen pixels, as the original.
const DEFAULT_PX: f32 = 64.0;
/// One step of `size_step`: 8 pixels, or a tenth of a size given as a length.
const STEP_PX: f32 = 8.0;

/// The session's brush settings.
#[derive(Clone, Debug)]
pub(crate) struct BrushSettings {
    /// The brush, by name.
    brush: String,
    /// The brush before the last change of brush, which `previous` goes back to.
    previous: Option<String>,
    /// The size every brush shares: on screen, or as a length.
    size: Size,
    /// Symmetry (`symmetry.set`).
    pub symmetry: SymmetrySettings,
    /// The settings changed in this session, per brush by name.
    patches: BTreeMap<String, Settings>,
    /// The global pressure curves, where set: brushes with `pressure.global` stroke with all
    /// of them, every other brush with the size curve (decision 0036).
    global: Settings,
    /// Live Detail for the window's pen (`live_detail.set`).
    pub detail: DetailSession,
    /// The held keys' brushes (`brush.slot`).
    pub slots: Slots,
    /// The colors the window's brushes paint with (`color.set`).
    pub colors: Colors,
    /// The pen's feel the window's strokes take (`pen.set`).
    pub pen: PenSettings,
    /// The Voxel Remesh settings a voxel call leaving one out takes (`mesh.voxel_settings`).
    pub voxel: crate::mesh::VoxelSettings,
    /// The alphas used lately, for the Alpha palette (`alpha.pick`).
    pub recent: crate::alpha::Recent,
    /// The preset files `brush.load` read, per brush by name: what `brush.save` writes into.
    pub sources: BTreeMap<String, String>,
    /// How far a Strength or Falloff step moves, for every brush (decision 0052, section 10).
    pub(super) increments: Increments,
}

#[derive(Clone, Copy, Debug)]
enum Size {
    Pixels(f32),
    Length(Length),
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            brush: "Draw".into(),
            previous: None,
            size: Size::Pixels(DEFAULT_PX),
            symmetry: SymmetrySettings::default(),
            patches: BTreeMap::new(),
            global: Settings::new(),
            detail: DetailSession::default(),
            slots: Slots::default(),
            colors: Colors::default(),
            pen: PenSettings::default(),
            voxel: crate::mesh::VoxelSettings::default(),
            recent: crate::alpha::Recent::default(),
            sources: BTreeMap::new(),
            increments: Increments::default(),
        }
    }
}

/// The settings, as `brush.set` reports them and the window shows them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct BrushState {
    /// The brush the pen strokes with: the held slot's while one is held (`brush.slot`), and
    /// every value below is that brush's.
    pub brush: String,
    /// The size on screen, in pixels, when it is set that way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_px: Option<f32>,
    /// The size as a length, when it is set that way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Length>,
    /// The brush's strength.
    pub strength: f32,
    /// The brush's Falloff, -100 (widest) to 100 (narrowest).
    pub falloff: f32,
    /// The brush before the last change of brush, which `previous` goes back to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The symmetry the pen strokes with, in a stroke's form: empty while the master is off.
    pub symmetry: SymmetryArg,
    /// The symmetry as the session keeps it (`symmetry.set`).
    #[serde(default)]
    pub symmetry_settings: SymmetrySettings,
    /// The brush's Brush Modifier.
    pub modifier: f32,
    /// The brush's stroke smoothing string, in brush radii (its `stroke.smoothing.radius`).
    pub smoothing: f32,
    /// The brush's Smooth, 0 to 0.99 (its `stroke.smoothing.factor`).
    pub smoothing_factor: f32,
    /// The brush's Step, its dab spacing in brush diameters (its `stroke.spacing`); none for a
    /// stroke without one.
    pub spacing: Option<f32>,
    /// The brush's stroke type, as the stroke picker names it (`dots`, `drag_rect`, `freehand`
    /// or `drag_dot`); none for a stroke of another class (a drag, curve or gesture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    /// Which way the brush works before Alt: `add` or `subtract` (its setting `mode`).
    #[schemars(with = "String")]
    pub mode: gs_core::brush::Mode,
    /// The settings changed for the brush in this session, in its preset's own names.
    #[serde(default)]
    pub settings: Settings,
    /// The alpha the brush stamps with, by name, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<String>,
    /// The global pressure curves set in this session (brushes with `pressure.global` stroke
    /// with them, and every brush with `size` unless it names its own); a curve left out is the
    /// default.
    #[serde(default, skip_serializing_if = "Settings::is_empty")]
    pub global_pressure: Settings,
    /// Live Detail for the window's pen (`live_detail.set`).
    #[serde(default)]
    pub live_detail: DetailState,
    /// The held keys' brushes and the slot held (`brush.slot`).
    #[serde(default)]
    pub slots: Slots,
    /// The pen's feel (`pen.set`).
    #[serde(default)]
    pub pen: PenSettings,
    /// The size pressure curve the brush strokes with, the pen's Size sensitivity in it, as a
    /// stroke pins it (decision 0036, section 3): what a pressure makes of the size. Left out
    /// while no size curve is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Value>")]
    pub size_curve: Option<Curve>,
    /// How far `strength_step` and `falloff_step` move, for every brush.
    #[serde(default)]
    pub increments: Increments,
}

impl BrushState {
    /// The stroke's Smooth, Step and type by their `session.state` names (decision 0036 section
    /// 4, decision 0051 section 7).
    pub(crate) fn stroke_values(&self) -> [(String, Value); 3] {
        [
            (
                "brush.smoothing_factor".into(),
                json!(self.smoothing_factor),
            ),
            ("brush.spacing".into(), json!(self.spacing)),
            ("brush.stroke".into(), json!(self.stroke)),
        ]
    }
}

impl BrushSettings {
    /// The settings changed for `brush` (a display name) in this session.
    pub fn patch_of(&self, brush: &str) -> Settings {
        self.patches.get(brush).cloned().unwrap_or_default()
    }

    /// The global pressure curves set in this session.
    pub fn global_pressure(&self) -> &Settings {
        &self.global
    }

    /// `brush` (a display name) with its patch: what a stroke with it uses.
    pub fn resolved(&self, brush: &str) -> Result<BrushPreset, CommandError> {
        let shipped = presets::find(brush)?;
        patch::resolve(shipped, &self.patch_of(brush), &[])
    }

    /// The settings as reported.
    pub fn state(&self) -> BrushState {
        let (size_px, size) = match self.size {
            Size::Pixels(px) => (Some(px), None),
            Size::Length(l) => (None, Some(l)),
        };
        // A patch that stopped resolving (it never does: brush.set checks every change) shows
        // the preset's own values.
        let brush = self.in_effect();
        let preset = (self.resolved(brush).ok())
            .or_else(|| presets::find(brush).ok().map(|s| s.preset.clone()));
        let preset = preset.as_ref();
        BrushState {
            brush: brush.to_string(),
            size_px,
            size,
            strength: preset.map_or(0.0, |p| p.strength),
            falloff: preset.map_or(0.0, |p| p.tip.focal),
            previous: self.previous.clone(),
            symmetry: self.symmetry.stroke(),
            symmetry_settings: self.symmetry,
            modifier: preset.map_or(0.0, |p| p.modifier),
            smoothing: preset.map_or(0.0, |p| p.stroke.smoothing().radius),
            smoothing_factor: preset.map_or(0.0, |p| p.stroke.smoothing().factor),
            spacing: preset.and_then(|p| p.stroke.spacing()),
            stroke: (preset.and_then(|p| StrokeType::of(&p.stroke))).map(|t| t.name().into()),
            mode: preset.map(|p| p.mode).unwrap_or_default(),
            settings: self.patch_of(brush),
            alpha: preset.and_then(|p| p.tip.alpha.as_ref().map(|a| a.image.clone())),
            global_pressure: self.global.clone(),
            live_detail: self.detail.state(),
            slots: self.slots.clone(),
            pen: self.pen,
            size_curve: preset.and_then(|p| self.size_curve(brush, p)),
            increments: self.increments,
        }
    }

    /// The size curve `brush` (resolved as `preset`) strokes with, felt through the pen's
    /// Size sensitivity, by the rule a stroke pins it with; `None` while it is flat at full.
    fn size_curve(&self, brush: &str, preset: &BrushPreset) -> Option<Curve> {
        let patch = self.patch_of(brush);
        let shipped = presets::find(brush).ok()?;
        let pinned = pin_pressure(&patch, preset, shipped, &self.global).unwrap_or(patch);
        let given = patch::get(&pinned, &["pressure", "size"]);
        let curve = given.and_then(|v| serde_json::from_value::<Curve>(v.clone()).ok());
        let felt = (curve.as_ref().unwrap_or(&preset.pressure.size))
            .with_sensitivity(self.pen.size_sensitivity / 100.0);
        let full = 1f32.to_bits();
        (felt.points().iter())
            .any(|p| p[1].to_bits() != full)
            .then_some(felt)
    }

    /// The brush the pen strokes with and `brush.set` changes: the held slot's while one is
    /// held, else the session's brush.
    pub(super) fn in_effect(&self) -> &str {
        self.slots.held_brush().unwrap_or(&self.brush)
    }

    /// The session's own brush, whatever a held key's slot holds: what the window pins as
    /// the brush at start (decision 0052, section 9).
    pub(crate) fn own(&self) -> &str {
        &self.brush
    }
}

/// Change the session's brush settings: the brush the pen strokes with, its size, any of its
/// settings, symmetry, and the global pressure curves. Leave a setting out to keep it. The size
/// and symmetry are shared by every brush; everything else belongs to the brush, kept as its
/// patch: the settings changed in this session, in its preset's own names (`brush.describe`
/// lists them), which the window's strokes carry. `previous` goes back to the brush used before
/// the last change, so a detour to Smooth or Mask Pen costs one call (and calling it again
/// comes back). While a held key's slot is held (`brush.slot`), a brush chosen goes on the
/// slot and the brush's own settings are the slot brush's. Answers with every setting.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushSet {
    /// The brush, by name or id (`brush.list`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    /// Go back to the brush used before the last change of brush (instead of `brush`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub previous: bool,
    /// Size on screen, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 10000.0))]
    pub size_px: Option<f32>,
    /// Size as a length: scene units, or a percentage of the part's size ("5%").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Length>,
    /// Grow (positive) or shrink the size by steps of 8 pixels, or of a tenth of a length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100, max = 100))]
    pub size_step: Option<i32>,
    /// The brush's strength, 0 to 1 (its setting `strength`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub strength: Option<f32>,
    /// The brush's Falloff, -100 (widest) to 100 (narrowest); 0 is the brush tip's own curve
    /// (its setting `tip.focal`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub falloff: Option<f32>,
    /// Raise (positive) or lower the strength by steps of `strength_increment`, held to 0 to 1
    /// (instead of `strength`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100, max = 100))]
    pub strength_step: Option<i32>,
    /// Harden (positive) or soften the Falloff by steps of `falloff_increment`, held to -100
    /// to 100 (instead of `falloff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100, max = 100))]
    pub falloff_step: Option<i32>,
    /// How far one Strength step moves, 0.01 to 0.5, for every brush (0.05 at first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.01, max = 0.5))]
    pub strength_increment: Option<f32>,
    /// How far one Falloff step moves, 1 to 50, for every brush (10 at first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 50.0))]
    pub falloff_increment: Option<f32>,
    /// The brush's Brush Modifier, -100 to 100 (its setting `modifier`; `brush.describe` says
    /// what it does on the brush).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub modifier: Option<f32>,
    /// The alpha the brush stamps with, by name (`alpha.list`), "white" for none (its setting
    /// `tip.alpha.image`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<String>,
    /// The brush's stroke type, by name: `dots`, `drag_rect`, `freehand` or `drag_dot` (the
    /// stroke picker's; its setting `stroke.class`, and the one setting the type names). Picking
    /// the type the brush shows changes nothing; a type the brush's kernel cannot run is refused
    /// (Move, Pull Out and the shape brushes keep their strokes). When the class changes, the
    /// brush's stroke keeps only the settings the new class reads (decision 0051).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    /// Any of the brush's settings, in its preset's own names, merged into what this session
    /// changed for it: `{"tip": {"alpha": {"image": "ring", "mid": 0.5}}, "stroke": {"spacing":
    /// 0.2}}`. A `null` drops a setting from the session's changes, so the preset's own value
    /// applies again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "patch::settings_schema")]
    pub settings: Option<Settings>,
    /// Put the brush back as it ships: every setting this session changed for it is dropped
    /// before this call's own are merged in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
    /// The brush whose settings this call changes, by name or id, without choosing it: the pen
    /// keeps its brush. Left out, the brush the pen strokes with.
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_brush: Option<String>,
    /// Symmetry for every brush, in a stroke's form: turns the master on, each axis named on in
    /// that type and the others off (`symmetry.set` changes one setting at a time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<SymmetryArg>,
    /// Turn one axis on or off, in its type (Mirror unless `symmetry.set` changed it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_mirror: Option<Axis>,
    /// The global pressure curves, by name (`size`, `strength`, `spacing`, `modifier`,
    /// `imbed`), each a list of `[pressure, multiplier]` points with the multiplier from 0 to 1
    /// (a fraction of the slider): what brushes with `pressure.global` stroke with. `size`
    /// reaches every brush, as the size is shared: `{"size": [[0, 0.25], [1, 1]]}` makes a
    /// light touch a quarter as wide, except on brushes that name their own size curve (the
    /// Move brushes and Pull Out keep theirs flat). A `null` puts a curve back to its default,
    /// which for `size` is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "patch::curves_schema")]
    pub global_pressure: Option<Settings>,
}

impl Command for BrushSet {
    const NAME: &'static str = "brush.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = BrushState;

    fn examples() -> Vec<Example<Self>> {
        let clay = json!({ "brush": "Clay Build", "size_px": 40, "strength": 0.3 });
        let ring = json!({ "alpha": "ring", "settings": { "tip": { "alpha": { "mid": 0.5 } } } });
        vec![
            Example::raw("stroke with Clay Build, 40 pixels wide, at 30%", clay),
            Example::raw("turn X symmetry on or off", json!({ "flip_mirror": "x" })),
            Example::raw("grow the brush one step", json!({ "size_step": 1 })),
            Example::raw("a harder edge on the brush", json!({ "falloff": 40 })),
            Example::raw("one step stronger", json!({ "strength_step": 1 })),
            Example::raw(
                "a light touch makes every brush a quarter as wide",
                json!({ "global_pressure": { "size": [[0, 0.25], [1, 1]] } }),
            ),
            Example::raw("stamp a ring that raises and carves", ring),
            Example::raw(
                "stamp Clay once, sized and turned by the drag",
                json!({ "brush": "Clay", "stroke": "drag_rect" }),
            ),
            Example::raw(
                "the brush's own spacing again",
                json!({ "settings": { "stroke": { "spacing": null } } }),
            ),
            Example::raw("the brush as it ships", json!({ "reset": true })),
            Example::raw(
                "Clay's spacing set, the pen keeping its brush",
                json!({ "for": "Clay", "settings": { "stroke": { "spacing": 0.3 } } }),
            ),
            Example::raw(
                "back to the brush before Smooth",
                json!({ "previous": true }),
            )
            .after("brush.set", json!({ "brush": "Smooth" })),
        ]
    }

    /// A stroke type the brush cannot run is unavailable for it, so the window's dry run draws
    /// its button disabled (decision 0051, section 7). The call is tried on a copy of the
    /// settings, so the rule is the one `run` applies; any other refusal is left to `run`.
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        if self.stroke.is_none() {
            return Ok(());
        }
        let mut next = cx.sculpt.settings.clone();
        let tried = (self.pick(&mut next)).and_then(|()| self.change_patch(&mut next));
        match tried {
            Err(e) if e.code == ErrorCode::Precondition && e.path.as_deref() == Some("/stroke") => {
                Err(Unavailable::new(e.message))
            }
            _ => Ok(()),
        }
    }

    fn run(mut self, cx: &mut Ctx) -> Result<BrushState, CommandError> {
        let mut next = cx.sculpt.settings.clone();
        self.pick(&mut next)?;
        match (self.size_px, self.size) {
            (Some(_), Some(_)) => {
                return Err(errors::args_invalid(
                    "/size",
                    "give size or size_px, not both",
                ));
            }
            (Some(px), None) => next.size = Size::Pixels(px),
            (None, Some(l)) => next.size = Size::Length(l),
            (None, None) => {}
        }
        if let Some(steps) = self.size_step {
            next.size = step(next.size, steps);
        }
        steps::resolve(&mut self, &mut next)?;
        self.change_patch(&mut next)?;
        if let Some(curves) = &self.global_pressure {
            next.global = global(&next.global, curves, &next.brush)?;
        }
        if let Some(s) = self.symmetry {
            next.symmetry.take(s)?;
        }
        if let Some(axis) = self.flip_mirror {
            next.symmetry.flip(axis);
        }
        if let Some(alpha) = &self.alpha {
            next.recent.used(alpha);
        }
        cx.sculpt.settings = next;
        Ok(cx.sculpt.settings.state())
    }
}
register_command!(BrushSet, "Set brush");

impl BrushSet {
    /// The brush this call picks, by its name: `brush`, or the previous one.
    fn chosen(&self, settings: &BrushSettings) -> Result<Option<String>, CommandError> {
        match (&self.brush, self.previous) {
            (Some(_), true) => Err(errors::args_invalid(
                "/previous",
                "give brush or previous, not both",
            )),
            (Some(brush), false) => Ok(Some(presets::find(brush)?.preset.name.clone())),
            (None, true) => settings.previous.clone().map(Some).ok_or_else(|| {
                let reason = "the brush has not changed yet, so there is no previous brush";
                errors::error(ErrorCode::Precondition, reason)
            }),
            (None, false) => Ok(None),
        }
    }

    /// Chooses this call's brush in `next`: on the held slot while one is held (`brush.slot`).
    fn pick(&self, next: &mut BrushSettings) -> Result<(), CommandError> {
        if let Some(brush) = self.chosen(next)? {
            match next.slots.held.filter(|_| self.brush.is_some()) {
                Some(slot) => next.slots.put(slot, brush),
                None => next.choose(brush),
            }
        }
        Ok(())
    }

    /// The shortcut arguments this call gave, by name.
    fn shortcuts(&self) -> Vec<(&'static str, Value)> {
        let numbers = [
            ("strength", self.strength),
            ("falloff", self.falloff),
            ("modifier", self.modifier),
        ];
        let mut given: Vec<_> = (numbers.into_iter())
            .filter_map(|(name, x)| Some((name, patch::number(x?))))
            .collect();
        if let Some(a) = &self.alpha {
            given.push(("alpha", json!(a)));
        }
        given
    }

    /// The stroke type this call picks, by its name.
    fn stroke_type(&self) -> Result<Option<StrokeType>, CommandError> {
        let Some(name) = &self.stroke else {
            return Ok(None);
        };
        StrokeType::named(name).map(Some).ok_or_else(|| {
            let names: Vec<&str> = StrokeType::ALL.iter().map(|t| t.name()).collect();
            let message = format!("{name} is not a stroke type: give {}", names.join(", "));
            patch::refused("/stroke", message)
        })
    }

    /// The brush this call's settings change, by its name: `for`, or the one the pen strokes
    /// with.
    pub(super) fn target(&self, next: &BrushSettings) -> Result<String, CommandError> {
        let Some(name) = &self.for_brush else {
            return Ok(next.in_effect().to_string());
        };
        let found = presets::find(name).map_err(|e| CommandError {
            path: Some("/for".into()),
            ..e
        })?;
        Ok(found.preset.name.clone())
    }

    /// Merges this call's settings into the brush's patch in `next` (an empty one on `reset`),
    /// checked by resolving the brush with it; refused, nothing changes.
    fn change_patch(&self, next: &mut BrushSettings) -> Result<(), CommandError> {
        let mut given = self.shortcuts();
        let picked = self.stroke_type()?;
        if let Some(t) = picked {
            given.push(("stroke", json!(t.class())));
        }
        let brush = self.target(next)?;
        if given.is_empty() && self.settings.is_none() && !self.reset {
            return Ok(());
        }
        let mut incoming = patch::with_shortcuts(self.settings.as_ref(), &given)?;
        let shipped = presets::find(&brush)?;
        let mut patch = if self.reset {
            Settings::new()
        } else {
            next.patch_of(&brush)
        };
        let now = patch::resolve(shipped, &patch, &[])?.stroke;
        // A type other than the one shown writes the setting it names (decision 0051).
        if let Some(t) = picked.filter(|&t| StrokeType::of(&now) != Some(t)) {
            let (key, value) = t.setting();
            if patch::get(&incoming, &["stroke", key]).is_none() {
                patch::put(&mut incoming, &["stroke", key], json!(value));
            }
        }
        restroke(&mut patch, &incoming, now.class());
        patch::merge(&mut patch, &incoming);
        unclass(&mut patch, shipped.preset.stroke.class());
        let names: Vec<&str> = given.iter().map(|g| g.0).collect();
        let resolved = patch::resolve(shipped, &patch, &names).map_err(unavailable)?;
        // The alpha must exist; processing it now spares the pen's first dab the wait.
        if let Some(spec) = &resolved.tip.alpha {
            crate::alpha::pyramid(spec)?;
        }
        if patch.is_empty() {
            next.patches.remove(&brush);
        } else {
            next.patches.insert(brush, patch);
        }
        Ok(())
    }
}

/// The global pressure curves `stored` with `curves` merged in: only curve names, each a valid
/// curve (`null` puts it back to its default). Checked as `brush`'s own pressure curves, so a
/// bad curve is refused with its name.
fn global(stored: &Settings, curves: &Settings, brush: &str) -> Result<Settings, CommandError> {
    if let Some(name) = curves.keys().find(|k| !patch::CURVES.contains(&k.as_str())) {
        let names = patch::CURVES.join(", ");
        let message = format!("{name} is not a pressure curve: give {names}");
        return Err(patch::refused(&format!("/global_pressure/{name}"), message));
    }
    let mut out = stored.clone();
    patch::merge(&mut out, curves);
    let probe = json!({ "pressure": out });
    let probe = probe.as_object().cloned().unwrap_or_default();
    patch::resolve(presets::find(brush)?, &probe, &[])
        .map_err(|e| patch::refused("/global_pressure", e.message))?;
    Ok(out)
}

/// The session's stroke changes for a brush whose class `incoming` changes from `was`: they keep
/// only the settings the new class reads (gs-core `carried`, decision 0051 section 6), so a Step
/// changed for Dots does not stop the brush taking Drag Rect.
fn restroke(patch: &mut Settings, incoming: &Settings, was: &str) {
    let Some(Value::String(class)) = patch::get(incoming, &["stroke", "class"]) else {
        return;
    };
    let Some(Value::Object(stroke)) = patch.get("stroke").filter(|_| class != was) else {
        return;
    };
    let Ok(toml::Value::Table(stroke)) = toml::Value::try_from(stroke) else {
        return;
    };
    match serde_json::to_value(carried(&stroke, class)) {
        Ok(Value::Object(kept)) if !kept.is_empty() => patch.insert("stroke".into(), kept.into()),
        _ => patch.remove("stroke"),
    };
}

/// A refusal of the `stroke` argument once it named a real type: the brush cannot run that type,
/// which is unavailable for it rather than a wrong name, so the window's dry run draws its
/// button disabled (decision 0051, section 7).
fn unavailable(e: CommandError) -> CommandError {
    if e.path.as_deref() != Some("/stroke") {
        return e;
    }
    CommandError {
        path: e.path,
        ..errors::error(ErrorCode::Precondition, e.message)
    }
}

/// `patch` without a `stroke.class` that names the brush's own class `shipped`, which changes
/// nothing: picking the brush's own type again leaves no change behind.
fn unclass(patch: &mut Settings, shipped: &str) {
    let Some(Value::Object(stroke)) = patch.get_mut("stroke") else {
        return;
    };
    if stroke.get("class").and_then(Value::as_str) == Some(shipped) {
        stroke.remove("class");
    }
    if stroke.is_empty() {
        patch.remove("stroke");
    }
}

impl BrushSettings {
    /// Makes `brush` the brush; the one it replaces becomes the previous one.
    fn choose(&mut self, brush: String) {
        if brush != self.brush {
            self.previous = Some(std::mem::replace(&mut self.brush, brush));
        }
    }

    /// `brush` as `brush.load` read it from the preset file `text`: `patch` replaces its
    /// settings, the file is what `brush.save` writes into, and the brush is chosen as
    /// `brush.set` chooses one (on the held slot while one is held).
    pub(super) fn load(&mut self, brush: String, patch: Settings, text: String) {
        if patch.is_empty() {
            self.patches.remove(&brush);
        } else {
            self.patches.insert(brush.clone(), patch);
        }
        self.sources.insert(brush.clone(), text);
        match self.slots.held {
            Some(slot) => self.slots.put(slot, brush),
            None => self.choose(brush),
        }
    }
}

/// `size` grown by `steps` steps (shrunk when negative), never below one step.
fn step(size: Size, steps: i32) -> Size {
    let k = steps as f32;
    match size {
        Size::Pixels(px) => Size::Pixels((px + k * STEP_PX).clamp(STEP_PX, 10000.0)),
        Size::Length(Length::Units(u)) => Size::Length(Length::Units(grow(u, k))),
        Size::Length(Length::Percent(p)) => Size::Length(Length::Percent(grow(p, k))),
    }
}

/// `x` grown by `k` tenths of itself, kept positive.
fn grow(x: f32, k: f32) -> f32 {
    (x * (1.0 + 0.1 * k)).max(x * 0.1)
}
