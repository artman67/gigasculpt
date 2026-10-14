//! The open deformer (decision 0041, sections 4 and 5, amended 2026-10-12 for DF4): a deformer
//! kept open across handle drags, with Accept, Cancel and Reset. `gizmo.deformer` checks each
//! call and queues it, as `gizmo.drag` does; after the call the engine puts the part back as it
//! was at the open and applies the whole deformer again, keeping each result where
//! `transform.apply` would, so handles never compound rounding. Accept puts the part back and
//! runs the `transform.apply` calls the deformer is (a `cmd.batch` of them for several), so it
//! is one undo step and one log line that replays it, by construction. Cancel leaves every bit,
//! and Reset goes back to no change and stays open.
//!
//! What accepts it first: an edit or a history call (`end_gizmo`), a pen press that starts a
//! stroke, a gizmo drag, another deformer, picking another tool, and a command that writes the
//! document out while no handle is held. While one is open, autosave waits, and while a handle
//! is held a write refuses for a moment (`Ctx::settled`), as it does for a drag.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Outcome, PartRef, Step};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Gizmos;
use super::live::Event;
use crate::command::CommandSpec;
use crate::document::Props;
use crate::engine::{Call, Engine};
use crate::mesh::smooth::MeshSmooth;
use crate::transform::{self, OpArg, Route, TransformApply};
use crate::{Command, Ctx, Example, Origin, errors, register_command};

/// The commands that write the document out: an open deformer with no handle held is accepted
/// before each, so the file holds it whole.
const WRITERS: [&str; 5] = [
    "file.save",
    "file.quick_save",
    "file.save_next",
    "file.export",
    "exchange.write",
];

/// Open a deformer on a part and keep it open across handle drags, as one undo step.
///
/// For the window: the gizmo's deformers and the Deform palette's sliders. `{"phase": "open",
/// "transforms": [...]}` opens one: one or more `transform.apply` calls on one part (a slider
/// sends one per lit axis), whose frames and box are pinned now. Each `"update"` carries the
/// deformer's whole arguments, the open's operations in the same order, and applies them again
/// from the part as it was at the open; `"lift"` says the pen let go of the handle. `"accept"`
/// keeps it as one undo step, logged as the `transform.apply` call that replays it (a
/// `cmd.batch` of them for several); `"cancel"` puts every bit back; `"reset"` goes back to no
/// change and stays open. An edit, undo, a pen stroke, another tool or a save accepts it first;
/// while a handle is held, saves and agents' edits wait. With none open, every phase but
/// "open" changes nothing.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoDeformer {
    /// "open", "update", "lift", "accept", "cancel" or "reset".
    pub phase: DeformerPhase,
    /// With "open" and "update": the deformer's whole arguments, as `transform.apply` calls
    /// on one part (the active part when none is named). An update's part, frames, box and
    /// spherize radius are the open's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transforms: Vec<TransformApply>,
    /// With "open" and "update", in place of `transforms`: a `mesh.smooth` call, for the
    /// Gizmo's smooth deformers and the palette's Smooth group. An update's part, frame and
    /// box are the open's; one that only raises the amount runs only the rounds it adds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<MeshSmooth>,
}

/// Where an open deformer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeformerPhase {
    /// Open a deformer, accepting any open one first.
    Open,
    /// A handle moved: here are the arguments now.
    Update,
    /// The pen let go of the handle; the deformer stays open.
    Lift,
    /// Keep it: one undo step and one log line.
    Accept,
    /// Put the part back, every bit.
    Cancel,
    /// Back to no change, still open.
    Reset,
}

/// What a deformer call did.
#[derive(Serialize, JsonSchema)]
pub struct Deformed {
    /// A deformer is open after the call.
    pub open: bool,
    /// The part it deforms, while one is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

impl Command for GizmoDeformer {
    const NAME: &'static str = "gizmo.deformer";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const HUMAN_ONLY: bool = true;
    const LOGGED: bool = false;
    type Output = Deformed;

    fn examples() -> Vec<Example<Self>> {
        let sphere = json!({ "subdivisions": 3 });
        let bend = json!([{ "op": { "bend": { "axis": "y", "toward": "x", "degrees": 0.0 } } }]);
        let twist = |axis| json!({ "op": { "twist": { "axis": axis, "degrees": 0.0 } } });
        vec![
            Example::raw(
                "open a bend on the active part, about its gizmo",
                json!({ "phase": "open", "transforms": bend }),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "a slider with X and Z lit: a twist about each, one deformer",
                json!({ "phase": "open", "transforms": [twist("x"), twist("z")] }),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "with no deformer open, accept keeps nothing",
                json!({ "phase": "accept" }),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw("an open needs the transforms", json!({ "phase": "open" }))
                .after("primitive.sphere", sphere)
                .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Deformed, CommandError> {
        use DeformerPhase::{Accept, Cancel, Lift, Open, Reset, Update};
        let given = matches!(self.phase, Open | Update);
        if u8::from(given)
            != u8::from(!self.transforms.is_empty()) + u8::from(self.smooth.is_some())
        {
            let message = if given {
                "open and update need the transforms, or smooth in their place"
            } else {
                "only open and update take transforms or smooth"
            };
            return Err(errors::args_invalid("/transforms", message));
        }
        if self.phase == Open {
            if let Some(job) = cx.control.held {
                return Err(crate::job::busy(job));
            }
            let opened = opened(cx, self.transforms, self.smooth)?;
            let part = Some(opened.part.clone());
            let change = Change::Open(Box::new(opened));
            cx.gizmos.queued.push(Event::Deformer(change));
            return Ok(Deformed { open: true, part });
        }
        let Some(open) = &cx.gizmos.deformer else {
            return Ok(Deformed {
                open: false,
                part: None,
            });
        };
        let part = open.part.clone();
        let change = match self.phase {
            Update => {
                let (transforms, smooth) = updated(cx, open, self.transforms, self.smooth)?;
                Change::Update(transforms, smooth)
            }
            Lift => Change::Lift,
            Accept => Change::Accept,
            Cancel => Change::Cancel,
            Open | Reset => Change::Reset,
        };
        if self.phase != Reset {
            cx.control.gesture = self.phase == Update;
        }
        let open = !matches!(self.phase, Accept | Cancel);
        cx.gizmos.queued.push(Event::Deformer(change));
        Ok(Deformed {
            open,
            part: open.then_some(part),
        })
    }
}
register_command!(GizmoDeformer, "Deformer");

/// Pins an open's transforms to one part (the one they name, else the active part) and each
/// frame (the part's gizmo when left out), and checks each as `transform.apply` would.
fn opened(
    cx: &Ctx,
    mut transforms: Vec<TransformApply>,
    mut smooth: Option<MeshSmooth>,
) -> Result<Opened, CommandError> {
    let mut map = Map::new();
    let named = (transforms.iter().map(|t| &t.part)).chain(smooth.iter().map(|m| &m.part));
    if let Some(part) = named.flatten().next() {
        map.insert("part".into(), part.0.clone().into());
    }
    crate::pinning::pin_existing(&cx.doc, &mut map)?;
    let part = map["part"].as_str().unwrap_or_default().to_string();
    // Measured only for a reach in percent: on a turned part it reads every point.
    let found = cx.doc.get_part(&part)?;
    for (i, args) in transforms.iter_mut().enumerate() {
        if args.part.as_ref().is_some_and(|p| p.0 != part) {
            let path = format!("/transforms/{i}/part");
            return Err(errors::args_invalid(&path, "a deformer works on one part"));
        }
        args.part = Some(PartRef(part.clone()));
        args.pin_frame(cx, &part)?;
        args.resolve(|| found.world_diagonal())?;
    }
    if let Some(m) = &mut smooth {
        m.part = Some(PartRef(part.clone()));
        m.pin_frame(cx, &part)?.resolve(|| found.world_diagonal())?;
    }
    Ok(Opened {
        origin: cx.origin.clone(),
        part,
        transforms,
        smooth,
    })
}

/// An update's transforms, checked against the open deformer's: the same operations in the
/// same order, on its part, about its frames, with its box and spherize radius where left out;
/// or its smoothing, on its part, about its frame, with its box where left out.
fn updated(
    cx: &Ctx,
    open: &Deformer,
    mut transforms: Vec<TransformApply>,
    smooth: Option<MeshSmooth>,
) -> Result<(Vec<TransformApply>, Option<MeshSmooth>), CommandError> {
    if transforms.len() != open.transforms.len() {
        let message = "an update gives as many transforms as the open did";
        return Err(errors::args_invalid("/transforms", message));
    }
    // Measured only for a reach in percent: on a turned part it reads every point.
    let found = cx.doc.get_part(&open.part)?;
    for (i, (args, was)) in transforms.iter_mut().zip(&open.transforms).enumerate() {
        if std::mem::discriminant(&args.op) != std::mem::discriminant(&was.op) {
            let path = format!("/transforms/{i}/op");
            return Err(errors::args_invalid(
                &path,
                "an update keeps the open's operations",
            ));
        }
        (args.part, args.frame) = (was.part.clone(), was.frame);
        args.bounds = args.bounds.or(was.bounds);
        if let (OpArg::Spherize(s), OpArg::Spherize(w)) = (&mut args.op, &was.op) {
            s.radius = s.radius.or(w.radius);
        }
        args.resolve(|| found.world_diagonal())?;
    }
    let smooth = match (smooth, &open.smooth) {
        (Some(mut m), Some(was)) => {
            (m.part, m.frame, m.bounds) = (was.part.clone(), was.frame, m.bounds.or(was.bounds));
            m.rounds()?;
            m.weighing().resolve(|| found.world_diagonal())?;
            Some(m)
        }
        (None, None) => None,
        _ => {
            return Err(errors::args_invalid(
                "/smooth",
                "an update keeps the open's kind",
            ));
        }
    };
    Ok((transforms, smooth))
}

/// `args` at no change: each value the command claims leaves every point where it was (its
/// `identity_at_zero` claim) put in, every other argument kept.
fn identity<C: Command + Clone>(args: &C) -> C {
    let claims: Value = serde_json::from_str(C::CLAIMS).unwrap_or_default();
    let zeros = (claims.as_array().into_iter().flatten())
        .find_map(|c| c["identity_at_zero"]["set"].as_object().cloned())
        .unwrap_or_default();
    let mut value = serde_json::to_value(args).unwrap_or_default();
    for (pointer, zero) in zeros {
        if let Some(v) = value.pointer_mut(&pointer) {
            *v = zero;
        }
    }
    serde_json::from_value(value).unwrap_or_else(|_| args.clone())
}

/// `now` is the smoothing `was` run further: the same arguments at as many rounds or more, `was`
/// above 0.
fn further(was: &MeshSmooth, now: &MeshSmooth) -> bool {
    let rest = |m: &MeshSmooth| {
        serde_json::to_value(MeshSmooth {
            amount: 0.0,
            ..m.clone()
        })
        .ok()
    };
    was.amount > 0.0 && now.rounds().ok() >= was.rounds().ok() && rest(was) == rest(now)
}

/// A deformer call, for the engine to apply after the call that queued it.
#[derive(Clone)]
pub(crate) enum Change {
    Open(Box<Opened>),
    Update(Vec<TransformApply>, Option<MeshSmooth>),
    Lift,
    Accept,
    Cancel,
    Reset,
}

/// An open's transforms, pinned and checked.
#[derive(Clone)]
pub(crate) struct Opened {
    origin: Origin,
    part: String,
    transforms: Vec<TransformApply>,
    smooth: Option<MeshSmooth>,
}

/// The open deformer: its part as it was at the open, and its arguments now.
#[derive(Clone)]
pub(crate) struct Deformer {
    origin: Origin,
    pub(super) part: String,
    /// The arguments now, as the `transform.apply` calls that replay them: part, frames, box
    /// and spherize radius pinned at the open.
    pub(super) transforms: Vec<TransformApply>,
    /// In place of the transforms, the `mesh.smooth` call that replays it; and the smoothing
    /// in the part's points now.
    smooth: Option<MeshSmooth>,
    ran: Option<MeshSmooth>,
    /// The part's properties at the open.
    pub(super) start: Props,
    /// The part's undo step is open: an update edited its points.
    pub(super) points: bool,
    /// The pen holds a handle.
    held: bool,
    /// The arguments were applied since the open or the last reset.
    pub(super) changed: bool,
    /// The tool at the open: the session's brush, and whether the window's gizmo tool is on.
    tool: (String, Option<bool>),
    /// On the graphics card: the points stay as at the open and the window moves them.
    pub(super) card: Option<super::preview::Card>,
}

/// The open deformer, as `session.state` names it.
#[derive(Clone, Debug)]
pub struct OpenDeformer {
    /// What it does: its first operation, `"bend"`.
    pub kind: String,
    /// The part it deforms.
    pub part: String,
    /// The pen holds one of its handles.
    pub held: bool,
    /// Its arguments now, as the `transform.apply` calls Accept would run.
    pub transforms: Vec<TransformApply>,
    /// Or as the `mesh.smooth` call Accept would run.
    pub smooth: Option<MeshSmooth>,
}

impl Gizmos {
    /// The open deformer, as `session.state` names it; `None` while none is open.
    pub fn open_deformer(&self) -> Option<OpenDeformer> {
        let d = self.deformer.as_ref()?;
        let op = d.transforms.first().map(|t| serde_json::to_value(&t.op));
        let kind = op.and_then(|op| Some(op.ok()?.as_object()?.keys().next()?.clone()));
        let smooth = d.smooth.as_ref().map(|_| "smooth".to_string());
        Some(OpenDeformer {
            kind: kind.or(smooth).unwrap_or_default(),
            part: d.part.clone(),
            held: d.held,
            transforms: d.transforms.clone(),
            smooth: d.smooth.clone(),
        })
    }
}

impl Engine {
    /// Applies a deformer call the last call queued.
    pub(crate) fn deformer_change(&mut self, change: Change) {
        match change {
            Change::Open(opened) => self.open_deformer(*opened),
            Change::Update(transforms, smooth) => {
                if let Some(mut d) = self.state.gizmos.deformer.take() {
                    (d.transforms, d.smooth, d.held) = (transforms, smooth, true);
                    self.reapply(&mut d);
                    self.state.gizmos.deformer = Some(d);
                }
            }
            Change::Lift => {
                if let Some(d) = &mut self.state.gizmos.deformer {
                    d.held = false;
                }
            }
            Change::Accept => self.accept_deformer(),
            Change::Cancel => {
                if let Some(mut d) = self.state.gizmos.deformer.take() {
                    self.put_back(&mut d);
                }
            }
            Change::Reset => {
                if let Some(mut d) = self.state.gizmos.deformer.take() {
                    self.put_back(&mut d);
                    d.transforms = d.transforms.iter().map(identity).collect();
                    d.smooth = d.smooth.as_ref().map(identity);
                    d.changed = false;
                    d.rest();
                    self.state.gizmos.deformer = Some(d);
                }
            }
        }
    }

    /// Opens a deformer: ends a stroke and a drag, accepts any open deformer, then measures
    /// each transform's box and radius on the part as it is now and applies the transforms
    /// unless they change nothing.
    fn open_deformer(&mut self, opened: Opened) {
        self.end_live();
        self.end_drag();
        self.accept_deformer();
        let tool = self.tool();
        let Some(found) = self.state.doc.get(&opened.part) else {
            return;
        };
        let start = found.props();
        let (mut transforms, mut smooth) = (opened.transforms, opened.smooth);
        for args in &mut transforms {
            // An open was checked; a settle that fails leaves the call as it was.
            args.settle(found.store(), start.transform, || found.world_diagonal())
                .ok();
        }
        if let Some(m) = &mut smooth {
            m.settle(found).ok();
        }
        let moves = (transforms.iter())
            .any(|t| serde_json::to_value(identity(t)).ok() != serde_json::to_value(t).ok())
            || smooth.as_ref().is_some_and(|m| m.amount != 0.0);
        let mut d = Deformer {
            origin: opened.origin,
            part: opened.part,
            transforms,
            smooth,
            ran: None,
            start,
            points: false,
            held: false,
            changed: false,
            tool,
            card: None,
        };
        if moves {
            self.reapply(&mut d);
        } else {
            self.preview(&mut d);
        }
        self.state.gizmos.deformer = Some(d);
    }

    /// Applies `d`'s arguments again from its part as it was at the open, each transform's
    /// result kept where `transform.apply` keeps it (decision 0029, section 6): in the part's
    /// transform, changed in place, or in its points, inside the part's open undo step.
    pub(super) fn reapply(&mut self, d: &mut Deformer) {
        if self.preview(d) {
            d.changed = true;
            return;
        }
        if d.smooth.is_some() {
            self.resmooth(d);
            return;
        }
        let Some(found) = self.state.doc.get_mut(&d.part) else {
            return;
        };
        if d.points {
            found.store_mut().revert_step();
        }
        let mut props = d.start;
        found.swap_props(&mut props);
        for args in &mut d.transforms {
            let from = found.props().transform;
            let Ok(t) = args.resolve(|| found.world_diagonal()) else {
                continue;
            };
            match transform::route(found, from, &t) {
                Route::Placed(transform) => {
                    let mut props = Props {
                        transform,
                        ..found.props()
                    };
                    found.swap_props(&mut props);
                }
                Route::Points(_) => {
                    if !d.points {
                        found.store_mut().begin_step();
                        d.points = true;
                    }
                    if let Ok(t) = args.settle(found.store(), from, || found.world_diagonal()) {
                        transform::apply(found.store_mut(), &t);
                    }
                }
            }
        }
        d.changed = true;
    }

    /// Applies `d`'s smoothing inside the part's open undo step: when it is the smoothing in
    /// the points run further (the same arguments, more rounds), only the rounds it adds; else
    /// all of it again from the part as it was at the open. A round is the same in either, so
    /// the points end as one `mesh.smooth` call at the amount leaves them.
    fn resmooth(&mut self, d: &mut Deformer) {
        let (Some(found), Some(now)) = (self.state.doc.get_mut(&d.part), &mut d.smooth) else {
            return;
        };
        let from = match &d.ran {
            Some(was) if further(was, now) => was.rounds().unwrap_or_default(),
            _ => 0,
        };
        if d.points && from == 0 {
            found.store_mut().revert_step();
        }
        if !d.points {
            found.store_mut().begin_step();
            d.points = true;
        }
        if let Ok(t) = now.settle(found) {
            now.apply(found.store_mut(), &t, from);
        }
        (d.ran, d.changed) = (Some(now.clone()), true);
    }

    /// Puts `d`'s part back as it was at the open, every bit, and closes its undo step.
    fn put_back(&mut self, d: &mut Deformer) {
        self.state.control.gesture = false;
        if let Some(part) = self.state.doc.get_mut(&d.part) {
            if d.points {
                part.store_mut().cancel_step();
            }
            let mut props = d.start;
            part.swap_props(&mut props);
        }
        (d.points, d.ran) = (false, None);
    }

    /// Accepts the open deformer, if there is one: puts its part back and runs the
    /// `transform.apply` calls it is, as its client, so it is one undo step and one log line
    /// (a `cmd.batch` of them for several) that replay it.
    pub(crate) fn accept_deformer(&mut self) {
        let Some(mut d) = self.state.gizmos.deformer.take() else {
            return;
        };
        self.put_back(&mut d);
        if !d.changed {
            return;
        }
        let smooth = (d.smooth.iter()).map(|m| ("mesh.smooth", serde_json::to_value(m)));
        let steps: Vec<Step> = (d.transforms.iter())
            .map(|t| ("transform.apply", serde_json::to_value(t)))
            .chain(smooth)
            .map(|(command, args)| Step {
                command: command.into(),
                args: args.unwrap_or_default(),
            })
            .collect();
        let call = Call {
            batch: steps.len() > 1,
            steps,
            if_rev: None,
            dry_run: false,
        };
        if let Outcome::Error(e) = self.run_now(&d.origin, &call).outcome {
            self.state.sculpt.problem = Some(e.message);
        }
    }

    /// Accepts an open deformer with no handle held before a call of `specs` that writes the
    /// document out, so the file holds it whole; a held one makes the write refuse for a
    /// moment instead (`Ctx::settled`).
    pub(crate) fn accept_before_writing(&mut self, specs: &[&CommandSpec]) {
        let idle = (self.state.gizmos.deformer.as_ref()).is_some_and(|d| !d.held);
        if idle && specs.iter().any(|s| WRITERS.contains(&s.name)) {
            self.accept_deformer();
        }
        self.catch_up(specs);
    }

    /// Accepts the open deformer when the tool changed since it opened: another brush picked,
    /// or the window's gizmo tool turned off.
    pub(crate) fn accept_on_another_tool(&mut self) {
        let moved = (self.state.gizmos.deformer.as_ref()).is_some_and(|d| d.tool != self.tool());
        if moved {
            self.accept_deformer();
        }
    }

    /// The tool now: the session's own brush, and whether the window's gizmo tool is on.
    fn tool(&self) -> (String, Option<bool>) {
        let gizmo = self.state.window.with(|w| w.gizmo(None)).map(|g| g.tool);
        (self.state.sculpt.settings.own().to_string(), gizmo)
    }
}
