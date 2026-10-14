//! Live Detail in the engine (decision 0021, sections 3, 4 and 8): the session's settings and
//! `live_detail.set`, the `brush.stroke` argument and how pinning fills it, which strokes
//! refine, and what a stroke reports.
//!
//! A stroke refines only when its call carries `live_detail`; the window's pen adds it while the
//! session's switch is on. Pinning fills every field the call left out, from the brush's own
//! `[live_detail]` table, else the session's, so the logged call replays without either.

pub use gs_core::live::Blocked;
use gs_core::live::{self, LiveReport, Target};
use gs_core::{BrushPreset, LiveMode, MeshStore};
use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::document::Document;
use crate::{Command, Ctx, Example, errors, register_command};

/// The target in brush mode when none is given: a quarter of the brush radius.
const BRUSH_SIZE: f32 = 0.25;
/// The target in screen mode when none is given, in pixels.
const SCREEN_SIZE: f32 = 30.0;

/// How the target edge length follows the brush.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetailMode {
    /// A fraction of the brush radius: a small brush adds fine detail, a big one coarse.
    Brush,
    /// A length in pixels on screen, so detail follows the zoom. Needs a stroke through a
    /// camera with its size in pixels, as the window strokes.
    Screen,
    /// A length in scene units, the same everywhere.
    Constant,
}

impl From<LiveMode> for DetailMode {
    fn from(m: LiveMode) -> Self {
        match m {
            LiveMode::Brush => Self::Brush,
            LiveMode::Screen => Self::Screen,
            LiveMode::Constant => Self::Constant,
        }
    }
}

impl DetailMode {
    /// The size a mode takes when none is given; constant mode has none.
    fn default_size(self) -> Option<f32> {
        match self {
            Self::Brush => Some(BRUSH_SIZE),
            Self::Screen => Some(SCREEN_SIZE),
            Self::Constant => None,
        }
    }
}

/// Live Detail settings: `brush.stroke`'s `live_detail`, and the session's. A field left out of
/// a stroke is filled when the call is pinned, from the brush's own settings, else the
/// session's (`live_detail.set`). The ranges are checked by the command rather than the
/// schema, so a refusal names the field at fault.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailSettings {
    /// How the target edge length follows the brush: `brush`, `screen` or `constant`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DetailMode>,
    /// The target edge length in the mode's terms: a fraction of the brush radius (0.25 when
    /// left out, at least 0.005), pixels (30), or scene units (no default). A size counts only
    /// with its own mode, so a session's brush-mode size never becomes pixels. In every mode a
    /// dab's target is at least 0.005 of its radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Edges shorter than this times the target merge, 0 to 1; at most 0.45 is used, so the
    /// halves of a split edge never merge again. Left out, 0.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<f32>,
    /// Merge short edges; off only adds triangles. Left out, on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combined: Option<bool>,
    /// Topology operations per dab, at least 1: the rest waits for the next dab. Left out,
    /// 4096.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u32>,
    /// Put a new point on the smooth curve through its edge's ends; off, on the straight edge.
    /// Left out, on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth_split: Option<bool>,
    /// How far changed points slide toward the middle of their neighbours, 0 to 1. Left out,
    /// 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relax: Option<f32>,
}

impl DetailSettings {
    /// Every field at its default, in brush mode.
    pub fn defaults() -> Self {
        let core = gs_core::LiveDetail::default();
        Self {
            mode: Some(DetailMode::Brush),
            size: Some(BRUSH_SIZE),
            merge: Some(core.merge),
            combined: Some(core.combined),
            budget: Some(core.budget),
            smooth_split: Some(core.smooth_split),
            relax: Some(core.relax),
        }
    }

    /// These settings, each field left out taken from `under`. The size goes with the mode:
    /// one given with another mode than the result's is not taken, and if no source gives one
    /// for that mode, the mode's default is.
    fn over(&self, under: &DetailSettings) -> DetailSettings {
        let mode = self.mode.or(under.mode);
        let sized = |s: &DetailSettings| s.size.filter(|_| s.mode.is_none() || s.mode == mode);
        DetailSettings {
            mode,
            size: sized(self)
                .or_else(|| sized(under))
                .or_else(|| mode.and_then(DetailMode::default_size)),
            merge: self.merge.or(under.merge),
            combined: self.combined.or(under.combined),
            budget: self.budget.or(under.budget),
            smooth_split: self.smooth_split.or(under.smooth_split),
            relax: self.relax.or(under.relax),
        }
    }

    /// Refuses a value no stroke can use, at its place under `at`.
    fn check(&self, at: &str) -> Result<(), CommandError> {
        let bad =
            |field: &str, message: String| errors::args_invalid(&format!("{at}/{field}"), message);
        if self.mode == Some(DetailMode::Constant) && self.size.is_none() {
            let message = "constant mode needs a size, the target edge length in scene units";
            return Err(bad("size", message.into()));
        }
        if let Some(size) = self.size
            && !(size > 0.0 && size.is_finite())
        {
            return Err(bad(
                "size",
                format!("the size must be a positive number, not {size}"),
            ));
        }
        if self.mode == Some(DetailMode::Brush)
            && let Some(size) = self.size
            && size < live::LEAST_SIZE
        {
            let message = format!(
                "in brush mode the size is a fraction of the brush radius, at least {} (at \
                 which one brush already holds 290,000 triangles), not {size}",
                live::LEAST_SIZE
            );
            return Err(bad("size", message));
        }
        for (field, x) in [("merge", self.merge), ("relax", self.relax)] {
            if let Some(x) = x
                && !(0.0..=1.0).contains(&x)
            {
                return Err(bad(field, format!("{field} must be from 0 to 1, not {x}")));
            }
        }
        if self.budget == Some(0) {
            return Err(bad(
                "budget",
                "the budget must be at least one operation".into(),
            ));
        }
        Ok(())
    }

    /// The core's settings for a stroke whose brush is `pixels` wide on screen, if it is sized
    /// that way. Every field is filled (see [`pin`]). Screen mode is brush mode with the factor
    /// `size / pixels`, so the core never sees pixels; without pixels it is refused.
    pub(crate) fn resolve(&self, pixels: Option<f32>) -> Result<gs_core::LiveDetail, CommandError> {
        let core = gs_core::LiveDetail::default();
        let size = self.size.unwrap_or(BRUSH_SIZE);
        let target = match self.mode.unwrap_or(DetailMode::Brush) {
            DetailMode::Brush => Target::Relative(size),
            DetailMode::Constant => Target::Constant(size),
            DetailMode::Screen => {
                let Some(px) = pixels else {
                    let message = "screen mode needs a brush sized in pixels: a stroke through a \
                                   camera with size_px, or with no size at all; give this \
                                   stroke brush or constant mode instead";
                    return Err(errors::args_invalid("/live_detail/mode", message));
                };
                Target::Relative(size / px)
            }
        };
        let ld = gs_core::LiveDetail {
            target,
            merge: self.merge.unwrap_or(core.merge),
            combined: self.combined.unwrap_or(core.combined),
            budget: self.budget.unwrap_or(core.budget),
            smooth_split: self.smooth_split.unwrap_or(core.smooth_split),
            relax: self.relax.unwrap_or(core.relax),
        };
        ld.check()
            .map_err(|m| errors::args_invalid("/live_detail", m))?;
        Ok(ld)
    }
}

/// The brush's own `[live_detail]` settings, in the call's terms.
fn of_preset(preset: &BrushPreset) -> DetailSettings {
    let spec = &preset.live_detail;
    DetailSettings {
        mode: spec.mode.map(DetailMode::from),
        size: spec.size,
        merge: spec.merge,
        combined: spec.combined,
        ..DetailSettings::default()
    }
}

/// A stroke's `live_detail` with every field filled: from the call, then `preset`, then the
/// session (decision 0021, section 4). `None` stays `None`: the stroke does not refine.
pub(crate) fn pin(
    call: Option<DetailSettings>,
    preset: &BrushPreset,
    session: &DetailSession,
) -> Result<Option<DetailSettings>, CommandError> {
    let Some(call) = call else {
        return Ok(None);
    };
    call.check("/live_detail")?;
    let filled = call.over(&of_preset(preset).over(&session.settings));
    filled.check("/live_detail")?;
    Ok(Some(filled))
}

/// Why Live Detail does not refine a part under a brush: its kind, from gs-core's one list, as
/// `session.state` names it in `live_detail.blocked` for the window's cursor, and its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub blocked: Blocked,
    /// The reason in a sentence, with what to do about it.
    pub why: String,
}

/// Why a stroke that asked for Live Detail does not refine, with what to do about it; `None`
/// if it refines.
pub(crate) fn refusal(preset: &BrushPreset, levels: bool, store: &MeshStore) -> Option<String> {
    blocked(preset, levels, store).map(|r| r.why)
}

/// Why Live Detail cannot refine `doc`'s active part under `preset`, as the window's cursor
/// shows it before a stroke; `None` when it can, and in an empty scene.
pub(crate) fn blocked_on_active(preset: &BrushPreset, doc: &Document) -> Option<Refusal> {
    let part = doc.get(doc.active()?)?;
    let levels = part.levels().is_some() || part.frozen().is_some();
    blocked(preset, levels, part.store())
}

/// What keeps Live Detail from refining a part with or without `levels` and `store`'s columns
/// under `preset`, in the order decision 0021 (section 8) lists the reasons.
fn blocked(preset: &BrushPreset, levels: bool, store: &MeshStore) -> Option<Refusal> {
    let name = &preset.name;
    let refusal = |blocked, why: String| Some(Refusal { blocked, why });
    if levels {
        let why = "the part has detail levels, which keep their topology: keep one level \
                   (level.delete_higher or level.delete_lower) to refine it, after \
                   level.unfreeze if they are frozen";
        return refusal(Blocked::Levels, why.into());
    }
    if let Some(why) = torn_uvs(store) {
        return refusal(Blocked::Uvs, format!("the part {why}"));
    }
    // gs-core's list words the preset's own reasons (`NotRefined::why`) and gives their kind.
    let Some(reason) = live::refuses(preset) else {
        if preset.live_detail.enable {
            return None;
        }
        let why = format!(
            "{name} opts out of Live Detail; settings {{\"live_detail\": {{\"enable\": \
             true}}}} turns it on"
        );
        return refusal(Blocked::OptOut, why);
    };
    refusal(reason.blocked(), reason.why(preset))
}

/// Why a part's UVs keep it from refining, with the remedy, if it has any: refining would tear
/// them (decision 0021, section 12, item 11). Worded to follow "the part" or "part 'name'".
pub(crate) fn torn_uvs(store: &MeshStore) -> Option<String> {
    let uv = (store.attrs().iter()).find(|d| crate::level::is_uv_corner(&d.name))?;
    Some(format!(
        "has UV columns ({} ...), which refining would tear: import it with uvs off to refine \
         it",
        uv.name
    ))
}

/// Whether a stroke refines: it did not ask, it asked and cannot, or it refines with these.
#[derive(Clone, Debug)]
pub(crate) enum Refining {
    NotAsked,
    Refused(String),
    Runs,
}

/// What Live Detail did in a stroke that asked for it.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct DetailDone {
    /// A refinement pass changed the topology: split, collapsed or turned an edge.
    pub refined: bool,
    /// Why it did not, and what to do about it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
    /// Edges split in two.
    pub splits: u32,
    /// Short edges merged.
    pub collapses: u32,
    /// Edges turned to even out the mesh.
    pub flips: u32,
    /// Quads split into two triangles.
    pub quad_splits: u32,
    pub points_added: u32,
    pub points_removed: u32,
    /// Dabs that stopped at the operation budget, the rest waiting for the next dab.
    pub budget_hits: u32,
    /// The stroke changed the topology without the mirror the part's mirror map is for, and
    /// released the map (`symmetry.map`).
    pub mirror_cleared: bool,
    /// What the stroke did to the part's symmetry, and what restores it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symmetry: Option<String>,
}

/// The note of a stroke that released a mirror map.
const CLEARED: &str = "topology is no longer symmetric; mesh.symmetrize restores it";
/// The note of a mirrored stroke that changed the topology of a part without a map.
const LOOSE: &str = "the topology is symmetric only as far as floating point allows; \
                     symmetry.map finds the mirror map of a symmetric part, and \
                     mesh.symmetrize makes a part symmetric with its map";

impl DetailDone {
    /// The report of a stroke that `refining` and that the core reported `live` for.
    pub(crate) fn of(refining: &Refining, live: Option<&LiveReport>) -> Option<Self> {
        match refining {
            Refining::NotAsked => None,
            Refining::Refused(why) => Some(Self {
                why_not: Some(why.clone()),
                ..Self::default()
            }),
            Refining::Runs => {
                let r = live.copied().unwrap_or_default();
                let refined = r.ops() > 0;
                Some(Self {
                    refined,
                    why_not: (!refined).then(|| unchanged(&r)),
                    splits: r.splits,
                    collapses: r.collapses,
                    flips: r.flips,
                    quad_splits: r.quad_splits,
                    points_added: r.added,
                    points_removed: r.removed,
                    budget_hits: r.budget_hits,
                    mirror_cleared: r.mirror_cleared,
                    symmetry: symmetry_note(&r),
                })
            }
        }
    }
}

/// Why a stroke whose passes ran, as reported by `r`, changed nothing.
fn unchanged(r: &LiveReport) -> String {
    if r.budget_hits > 0 {
        "every dab ran out of its live detail budget before its first operation: a larger \
         budget refines"
            .into()
    } else if r.waits > 0 {
        "the edges to split waited, for room in their nodes or for a longer edge beside them \
         that could not split: the next stroke tries again"
            .into()
    } else {
        "no edge the brush touched was longer than the live detail size or shorter than its \
         merge length, but for masked and hidden ones: a smaller size adds triangles"
            .into()
    }
}

/// What a stroke reported by `r` did to its part's symmetry, if anything worth a note.
fn symmetry_note(r: &LiveReport) -> Option<String> {
    let notes: Vec<&str> = [(r.mirror_cleared, CLEARED), (r.mirror_loose, LOOSE)]
        .into_iter()
        .filter_map(|(on, note)| on.then_some(note))
        .collect();
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// The session's Live Detail: whether the window's pen refines, and with what.
#[derive(Clone, Debug)]
pub(crate) struct DetailSession {
    pub on: bool,
    /// Every field given.
    pub settings: DetailSettings,
}

impl Default for DetailSession {
    fn default() -> Self {
        Self {
            on: false,
            settings: DetailSettings::defaults(),
        }
    }
}

/// The session's Live Detail, as `live_detail.set` answers and `session.state` names it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DetailState {
    /// The window's pen refines.
    pub on: bool,
    /// What it refines with.
    pub settings: DetailSettings,
}

impl DetailSession {
    pub fn state(&self) -> DetailState {
        DetailState {
            on: self.on,
            settings: self.settings,
        }
    }

    /// What the window's pen strokes with: nothing while the switch is off, else every field
    /// left to pinning (the brush's own table, then the session).
    pub fn pen(&self) -> Option<DetailSettings> {
        self.on.then(DetailSettings::default)
    }
}

/// Change the session's Live Detail: whether the window's pen adds and removes triangles under
/// the brush as it sculpts (`on`), and the settings it does it with. Leave a setting out to keep
/// it. A change of `mode` without a `size` takes that mode's default size. `brush.stroke`
/// never reads these: a stroke carries its own `live_detail`, which the window fills from here,
/// so the log replays without them. Answers with every setting.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveDetailSet {
    /// Turn Live Detail on or off for the window's pen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Turn it the other way (instead of `on`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub toggle: bool,
    /// The settings to change.
    #[serde(flatten)]
    pub settings: DetailSettings,
}

impl Command for LiveDetailSet {
    const NAME: &'static str = "live_detail.set";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = DetailState;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("sculpt with Live Detail", json!({ "on": true })),
            Example::raw("turn Live Detail on or off", json!({ "toggle": true })),
            Example::raw(
                "detail a tenth of the brush radius",
                json!({ "mode": "brush", "size": 0.1 }),
            ),
            Example::raw(
                "detail 12 pixels on screen, whatever the zoom",
                json!({ "mode": "screen", "size": 12 }),
            ),
            Example::raw(
                "edges 0.02 long everywhere, only added",
                json!({ "mode": "constant", "size": 0.02, "combined": false }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<DetailState, CommandError> {
        if self.toggle && self.on.is_some() {
            return Err(errors::args_invalid(
                "/toggle",
                "give on or toggle, not both",
            ));
        }
        let session = &mut cx.sculpt.settings.detail;
        self.settings.check("")?;
        // A new mode without a size takes its own default, never the old mode's size.
        let mut given = self.settings;
        if given.mode.is_some() && given.size.is_none() {
            given.size = given.mode.and_then(DetailMode::default_size);
        }
        let next = given.over(&session.settings);
        next.check("")?;
        session.settings = next;
        session.on = match (self.on, self.toggle) {
            (Some(on), _) => on,
            (None, true) => !session.on,
            (None, false) => session.on,
        };
        Ok(session.state())
    }
}
register_command!(LiveDetailSet, "Set Live Detail");
