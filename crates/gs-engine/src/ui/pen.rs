//! `pen.input`.

use gs_core::Vec3;
use gs_core::gizmo::Pose;
use gs_protocol::{CommandError, Cost, Kind, PenSample, Role};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Surface;
use crate::sculpt::SymmetryArg;
use crate::{Command, Ctx, Example, register_command};

/// Inject pen samples through the same path the tablet's samples take in the window.
///
/// Each sample is one merged tablet report in physical window pixels (`ui.describe` gives the
/// window's size). In the window, samples move the cursor and act on the interface under it,
/// exactly as the real pen does; without a window they only move the session's cursor. While
/// the human's pen touches, agent edits wait (`E_BUSY`). A call is written to the command log,
/// so recorded pen input replays, unless its samples sculpted or drove the gizmo: those reach
/// the log as the stroke, drag or placement they made, so a replay does not make it twice.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PenInput {
    /// The samples, oldest first: 1 to 10,000.
    #[schemars(length(min = 1, max = 10000))]
    pub samples: Vec<PenSample>,
}

/// Where the pen ended up.
#[derive(Serialize, JsonSchema)]
pub struct PenOutput {
    /// Samples taken.
    pub accepted: u64,
    /// The pen after the last sample, or nothing if it left range.
    pub cursor: Option<PenSample>,
    /// True if a window took the samples.
    pub window: bool,
}

impl Command for PenInput {
    const NAME: &'static str = "pen.input";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = PenOutput;

    fn examples() -> Vec<Example<Self>> {
        // Raw, as agents write them: everything left out is a pen hovering in range.
        let touch = json!({ "samples": [
            { "x": 400.0, "y": 300.0 },
            { "t_ns": 4_000_000, "x": 400.0, "y": 300.0, "pressure": 0.5, "contact": true }
        ] });
        let away = json!({ "samples": [
            { "x": 400.0, "y": 300.0 },
            { "t_ns": 8_000_000, "x": 400.0, "y": 300.0, "in_proximity": false }
        ] });
        vec![
            Example::raw("hover, then touch with half pressure", touch),
            Example::raw("the pen leaves range: no cursor", away),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<PenOutput, CommandError> {
        let pose = |part: &str| crate::gizmo::pose_of(cx, part).ok().map(|(pose, _)| pose);
        let under = |o, d| super::gizmo::under(cx, o, d);
        let active = || cx.doc.active();
        let masked = || {
            let part = active().and_then(|name| Some((cx.doc.get_part(&name).ok()?, name)));
            part.is_some_and(|(p, name)| {
                let report = crate::mask::MaskReport::of(&name, p.store());
                report.masked + report.partial > 0
            })
        };
        let dabs = |o, d, s: &SymmetryArg| dabs(cx, o, d, s);
        let reads = PenReads {
            pose: &pose,
            under: &under,
            masked: &masked,
            dabs: &dabs,
        };
        let taken = cx.window.with(|w| w.pen_input(&self.samples, &reads));
        let cursor = taken.cursor;
        cx.in_gesture = taken.gizmo;
        if cx.origin.role == Role::Human {
            cx.control.gesture = cursor.is_some_and(|c| c.contact);
        }
        Ok(PenOutput {
            accepted: self.samples.len() as u64,
            cursor,
            window: cx.window.with(|w| w.has_window()),
        })
    }
}
register_command!(PenInput, "Pen input");

/// What the pen reads from the session while the window takes its samples: the gizmo tool's
/// poses, and the surfaces and part state the pen's gestures on the 3D view need.
pub struct PenReads<'a> {
    /// The pose of a part's gizmo, by the part's name; `None` for a part that does not exist.
    pub pose: &'a dyn Fn(&str) -> Option<Pose>,
    /// The first surface of a part that shows under a ray from the scene point `o` along the
    /// unit vector `d`.
    pub under: &'a dyn Fn(Vec3, Vec3) -> Option<Surface>,
    /// Some point of the active part is masked, hidden points too, as a voxel remesh refuses.
    pub masked: &'a dyn Fn() -> bool,
    /// Where a dab under a ray from the scene point `o` along the unit vector `d` lands, and
    /// where symmetry `s` repeats it (see [`dabs`]): the cursor's mirror rings.
    pub dabs: &'a dyn Fn(Vec3, Vec3, &SymmetryArg) -> Vec<Vec3>,
}

/// Where a dab under the scene ray from `o` along unit `d` lands on the active part, the only
/// part a stroke sculpts (decision 0033, section 4), and where symmetry `s`'s other passes put
/// it: scene points, the dab first, then the passes in gs-core's `Symmetry::passes` order.
/// The passes mirror and turn in the part's own space about its origin, as a stroke's do, so
/// on a symmetric part each lands on the surface. Empty when the ray misses the active part or
/// it does not show. One ray cast through the part's BVH.
pub(crate) fn dabs(cx: &Ctx, o: Vec3, d: Vec3, s: &SymmetryArg) -> Vec<Vec3> {
    let active = cx.doc.active();
    let mut shown = cx.doc.document().shown();
    let Some((_, part)) = shown.find(|(name, _)| active.as_ref() == Some(*name)) else {
        return Vec::new();
    };
    let Some((_, hit)) = part.hit(o, d) else {
        return Vec::new();
    };
    let t = part.transform();
    (s.core().passes().into_iter())
        .map(|xf| t.point(xf.apply(hit.pos)))
        .collect()
}
