//! The session's bookkeeping around the step pipeline: which revision each client last saw,
//! greeting and forgetting clients, the window and the offscreen renderer the engine works
//! with, telling the window what each call did, the window's pen strokes, which commit as the
//! `brush.stroke` calls that replay them, and view gestures, which commit as the one `view.set`
//! call that replays each (decision 0017).

use std::collections::BTreeMap;

use gs_core::NodeId;
use gs_core::rng::CommandRng;
use gs_protocol::{
    Catalog, CommandError, ErrorCode, HelloResult, Kind, PROTOCOL_VERSION, Role, Step,
};
use serde_json::{Value, json};

use super::{Call, Done, Engine, Env, Ran, run_steps};
use crate::command::CommandSpec;
use crate::ctx::Origin;
use crate::document::{DocTxn, Document, Part, Props, Showing};
use crate::sculpt::{Live, PenStroke};
use crate::session::{StateSources, reference_state, state_of};
use crate::ui::{CallReport, MainView, PartReport, RenderHost, ShownReference, WindowHost};
use crate::view::{Ended, ExactCamera, MAIN, Scene, ViewInfo, Views, own_view};

impl Engine {
    /// Greets a client: starts counting other clients' changes for it from now, and tells it
    /// about the session.
    pub fn hello(&mut self, origin: &Origin) -> HelloResult {
        self.seen.insert(origin.client_id, self.rev);
        HelloResult {
            engine_version: self.registry.catalog.engine_version.clone(),
            protocol: PROTOCOL_VERSION,
            session: self.config.session.clone(),
            client_id: origin.client_id,
            role: origin.role,
            catalog_hash: self.registry.catalog.hash.clone(),
        }
    }

    /// Forgets a client that disconnected.
    pub fn goodbye(&mut self, client_id: u64) {
        self.seen.remove(&client_id);
    }

    /// Changes other clients made since this client's last call. A new client starts counting
    /// from now.
    pub(crate) fn concurrent_changes(&mut self, origin: &Origin) -> u64 {
        let changes = (self.seen.get(&origin.client_id)).map_or(0, |seen| self.rev - seen);
        self.seen.entry(origin.client_id).or_insert(self.rev);
        changes
    }

    /// Puts the engine in a window: window commands reach it, and it hears about every call.
    pub fn attach_window(&mut self, window: Box<dyn WindowHost>) {
        self.state.window.replace(window);
        self.redraw();
    }

    /// Tells the window of the session as it is, as after a call nobody made: so it draws what
    /// changed without a call (a Live Boolean run that landed).
    pub(crate) fn redraw(&mut self) {
        let nobody = Origin {
            client_id: 0,
            client: String::new(),
            role: Role::Human,
        };
        self.report(&nobody, &[], true);
    }

    /// Gives the engine its offscreen renderer, for `view.snapshot` and `view.contact_sheet`.
    /// A renderer starts with the shipped palette, so a palette changed before is handed over.
    pub fn attach_renderer(&mut self, render: Box<dyn RenderHost>) {
        self.state.render.replace(render);
        let palette = &self.state.palette.now;
        if *palette != crate::MaterialPalette::shipped() {
            self.state.render.with(|r| r.materials(palette));
        }
    }

    /// Every view in use, as `view.get` reports each: what a saved scene could keep.
    pub fn views(&self) -> BTreeMap<String, ViewInfo> {
        self.state.views.report(&self.state.doc)
    }

    /// `main` is seen at the size of the window's 3D view, when there is one, and views keep
    /// their framing once they are seen with parts in the scene (`main` in a window only once
    /// the window knows its size). Done at the start of every call, so it happens at the same
    /// point of a replay.
    pub(crate) fn sync_views(&mut self) {
        let (window, size) = self.state.window.with(|w| (w.has_window(), w.viewport()));
        if let Some(size) = size {
            self.state.views.seen_at(MAIN, size);
        }
        self.reference_seen();
        let main_seen = !window || size.is_some();
        let views = &mut self.state.views;
        views.keep_framing(&self.state.doc, window, |name| name != MAIN || main_seen);
    }

    /// Each Reference View the window shows is seen at the size of its picture (decisions
    /// 0020, 0024), as `main` is at the 3D view's.
    fn reference_seen(&mut self) {
        let sizes = self.state.window.with(WindowHost::reference_sizes);
        for (view, size) in sizes {
            if self.state.views.secondary(&view).is_some() {
                self.state.views.seen_at(&view, size);
            }
        }
    }

    /// Tells the window what a call did.
    pub(crate) fn report(&mut self, origin: &Origin, commands: &[String], ok: bool) {
        if let Some(size) = self.state.window.with(WindowHost::viewport) {
            self.state.views.seen_at(MAIN, size);
        }
        self.reference_seen();
        let controls = self.state.window.with(WindowHost::controls);
        let refused = self.refused(controls);
        let state = &mut self.state;
        let client = if origin.client.is_empty() {
            String::new()
        } else {
            origin.label()
        };
        let (session, rev) = (&self.config.session, self.rev);
        let main = state.views.info(MAIN, &state.doc);
        let now = state_of(StateSources::of(state, &self.config, rev));
        state.window.with(|window| {
            let references = if window.has_window() {
                shown_references(&state.views, &state.doc)
            } else {
                Vec::new()
            };
            // Only a real window draws the parts, so only a window takes their changes. Dynamic
            // isolate hides the other parts while a gesture moves the main camera.
            let dynamic = now.settings.as_ref().is_some_and(|s| s.dynamic_isolate);
            let hiding = dynamic && main.gesture.is_some();
            let d = &state.display;
            let live = (window.has_window()).then(|| {
                state
                    .live
                    .draw(&state.doc, [d.show_issues, d.show_coplanar])
            });
            let parts = if window.has_window() {
                part_reports(&mut state.doc, hiding)
            } else {
                Vec::new()
            };
            // The window draws the pen's brush's curves and smoothing string (decision 0025).
            let preset = (window.has_window())
                .then(|| state.sculpt.settings.resolved(&now.brush.brush).ok())
                .flatten();
            let problem = state.sculpt.problem.take();
            let stroke = state.sculpt.refused.take();
            let saved = state.file.wrote.take();
            let canvas = std::mem::take(&mut state.sculpt.on_canvas);
            window.after_call(&CallReport {
                session,
                client: &client,
                role: origin.role,
                commands,
                ok,
                rev,
                parts,
                state: &now,
                preset,
                problem: problem.as_deref(),
                stroke: stroke.as_ref(),
                saved: saved.as_deref(),
                main: MainView::of(&main),
                references,
                on_canvas: canvas,
                refused,
                preview: state.gizmos.preview(),
                live: live.unwrap_or_default(),
            });
        });
    }

    /// Which of the window's controls' `steps` the human cannot run now, and why: each is a
    /// dry run (policy, arguments, pinning and the command's check; nothing runs). Only a
    /// refusal as unavailable or naming nothing that exists counts; any other error is the
    /// controls file's mistake, which shows when the control is pressed. A refusal that asks
    /// for an argument the step does not give leaves the control on: pressed, the window asks
    /// (decision 0016, section 17). So does one about an argument it lacks, which the step is
    /// dry-run with as [`needed`] fills it in.
    fn refused(&mut self, steps: Vec<Step>) -> Vec<(Step, String)> {
        let human = Origin {
            client_id: 0,
            client: String::new(),
            role: Role::Human,
        };
        let mut out = Vec::new();
        for step in steps {
            let (runs, filled) = needed(&self.registry.catalog, &step);
            let call = Call {
                steps: vec![runs],
                if_rev: None,
                dry_run: true,
                batch: false,
            };
            let Ok(specs) = self.admit(&human, &call) else {
                continue;
            };
            let env = Env {
                registry: &self.registry,
                config: &self.config,
                origin: &human,
                log: self.log.recent(),
                log_at: self.log.position(),
                rev: self.rev,
                clients: self.seen.len() as u64,
                job: None,
            };
            let mut seq = self.next_seq;
            let ran = run_steps(self.state.scope(), &env, &mut seq, &call, &specs);
            let filled_at = |at: &str| {
                filled
                    .iter()
                    .any(|f| at.split('/').nth(1) == Some(f.as_str()))
            };
            if let Err((_, e)) = ran
                && matches!(e.code, ErrorCode::Precondition | ErrorCode::NotFound)
                && crate::errors::asks(&e, &step.args).is_none()
                && !e.path.as_deref().is_some_and(filled_at)
            {
                out.push((step, e.message));
            }
        }
        out
    }

    /// Ends the pen stroke and the gizmo drag in progress, and accepts an open deformer, before
    /// a call that edits or moves through history, so the call acts on the finished stroke,
    /// drag or deformer; accepts an idle deformer before a write, too.
    pub(crate) fn end_live_before(&mut self, specs: &[&CommandSpec]) {
        if specs
            .iter()
            .any(|s| matches!(s.kind, Kind::Edit | Kind::Session))
        {
            self.end_live();
            self.end_gizmo();
        }
        self.accept_before_writing(specs);
    }

    /// The window's pen strokes since the last call: a touch starts a stroke, samples sculpt
    /// as they come, and lifting the pen commits the stroke (see `sculpt::live`).
    pub(crate) fn pen_strokes(&mut self, origin: &Origin, strokes: Vec<PenStroke>) {
        for event in strokes {
            match event {
                PenStroke::Down(down) => {
                    self.end_live();
                    self.end_gizmo();
                    let [_, _, w, h] = down.viewport;
                    self.state.views.seen_at(MAIN, [w, h]);
                    let camera = self.state.views.camera_of(MAIN, &self.state.doc);
                    let seed = CommandRng::for_command(self.config.seed, self.next_seq).seed();
                    let held = self.state.control.held;
                    let sculpt = &mut self.state.sculpt;
                    let begun =
                        camera.and_then(|c| Live::begin(origin, down, &sculpt.settings, c, seed));
                    match begun {
                        Ok(mut live) => {
                            live.held = held;
                            sculpt.live = Some(live);
                        }
                        Err(e) => sculpt.refused = Some(e),
                    }
                }
                PenStroke::Sample(sample) => {
                    let sculpt = &mut self.state.sculpt;
                    if let Some(live) = &mut sculpt.live {
                        live.feed(&mut self.state.doc, &sample);
                        if live.on_canvas {
                            sculpt.live = None;
                            sculpt.on_canvas = true;
                        }
                    }
                }
                key @ (PenStroke::Alt(_)
                | PenStroke::Point
                | PenStroke::Corner
                | PenStroke::Space(_)) => {
                    if let Some(live) = &mut self.state.sculpt.live {
                        live.key(&key);
                    }
                }
                PenStroke::Up => self.end_live(),
            }
        }
    }

    /// Ends the pen stroke in progress, if there is one, and commits it as the `brush.stroke`
    /// call that replays it: one log line and one undo step, as the call would make.
    pub(crate) fn end_live(&mut self) {
        let Some(live) = self.state.sculpt.live.take() else {
            return;
        };
        let problem = live.problem.clone();
        let Some(ended) = live.end(&mut self.state.doc) else {
            self.state.sculpt.refused = problem;
            return;
        };
        if let Some(first) = ended.first {
            self.state.views.edited(&own_view(&ended.origin), first);
        }
        let adopted = Adopted {
            command: "brush.stroke",
            args: serde_json::to_value(&ended.args).unwrap_or_default(),
            output: serde_json::to_value(&ended.output).unwrap_or_default(),
            why: ended.why,
            part: ended.part,
            written: ended.written,
            props: None,
        };
        if let Err(e) = self.commit_in_place(&ended.origin, adopted) {
            self.state.sculpt.refused = Some(e);
        }
    }

    /// Commits an edit made in place between calls (a pen stroke, a gizmo drag) as the call
    /// that replays it: the part's open step becomes one undo step and one log line, as the
    /// call would make.
    pub(crate) fn commit_in_place(
        &mut self,
        origin: &Origin,
        adopted: Adopted,
    ) -> Result<(), CommandError> {
        let spec = self.registry.find(adopted.command)?;
        let call = Call {
            steps: vec![Step {
                command: spec.name.into(),
                args: adopted.args.clone(),
            }],
            if_rev: None,
            dry_run: false,
            batch: false,
        };
        let first = self.next_seq;
        self.next_seq += 1;
        let active = self.state.doc.active_id();
        let txn = DocTxn::adopt(
            &mut self.state.doc,
            &adopted.part,
            adopted.written,
            adopted.props,
        );
        let (changes, reads) = txn.finish()?;
        let ran = Ran {
            spec,
            args: adopted.args,
            output: adopted.output,
            noop_reason: Some(adopted.why),
            warnings: Vec::new(),
            landing: None,
            logged: true,
        };
        let done = Done {
            ran: vec![ran],
            changes,
            doc_replaced: false,
            reads,
            log_as: None,
            active,
        };
        self.commit(origin, &call, first, done, true);
        Ok(())
    }
}

/// An edit made in place between calls, as the call that replays it.
pub(crate) struct Adopted {
    /// The command it is logged as.
    pub command: &'static str,
    /// Its arguments, fully resolved.
    pub args: Value,
    pub output: Value,
    /// Its no-op reason, should it change nothing.
    pub why: String,
    /// The part, whose step is open.
    pub part: String,
    /// The nodes written, ascending.
    pub written: Vec<NodeId>,
    /// When only the part's properties changed (a gizmo drag that moved the whole part,
    /// decision 0029): what they were. Its store then has no open step.
    pub props: Option<Props>,
}

/// The view a step names in its `view` argument (left out: the caller's own), for a command
/// that takes one.
fn view_of(origin: &Origin, spec: &CommandSpec, step: &Step) -> Option<String> {
    (spec.view != crate::ViewArg::None).then(|| {
        (step.args.get("view").and_then(Value::as_str))
            .map_or_else(|| own_view(origin), str::to_string)
    })
}

/// `step` with each argument its command needs and it lacks filled in from the command's
/// examples in `catalog` (the first that gives it, one that runs before one that fails, never
/// one with arguments refused), and the names filled. A control that opens a form lacks what
/// the form asks for until it is typed (Export…'s path), and a dry run checks the arguments
/// before the command's check: filled in, the step reaches its check, so a row is gray with the
/// check's reason before the form asks anything (decision 0025, section 6, amended 2026-10-14).
/// A refusal about a filled argument is the form's to say once it is typed, not the row's; a
/// command whose check judges an argument names it in its refusal, as the others do.
fn needed(catalog: &Catalog, step: &Step) -> (Step, Vec<String>) {
    let info = catalog.commands.iter().find(|c| c.name == step.command);
    let required = info.and_then(|i| i.args_schema.get("required")?.as_array());
    let mut args = step.args.as_object().cloned().unwrap_or_default();
    let lacks: Vec<&str> = (required.into_iter().flatten())
        .filter_map(Value::as_str)
        .filter(|name| !args.contains_key(*name))
        .collect();
    // Whether each fails, and its arguments: one that runs sorts first.
    let mut examples: Vec<(bool, &Value)> = (info.into_iter())
        .flat_map(|i| &i.examples)
        .filter(|x| x.fails_with != Some(ErrorCode::ArgsInvalid))
        .map(|x| (x.fails_with.is_some(), &x.args))
        .collect();
    examples.sort_by_key(|(fails, _)| *fails);
    let mut filled = Vec::new();
    for name in lacks {
        if let Some(value) = examples.iter().find_map(|(_, x)| x.get(name)) {
            args.insert(name.into(), value.clone());
            filled.push(name.to_string());
        }
    }
    if filled.is_empty() {
        return (step.clone(), filled);
    }
    let runs = Step {
        command: step.command.clone(),
        args: Value::Object(args),
    };
    (runs, filled)
}

impl Engine {
    /// Makes the views a call names before it runs (see `Views::make`): its client's own, and
    /// each view a step names as its `view` or its camera. A dry run makes none.
    pub(crate) fn make_views(&mut self, origin: &Origin, call: &Call, specs: &[&CommandSpec]) {
        let views = &mut self.state.views;
        views.make(&own_view(origin), origin);
        for (spec, step) in specs.iter().zip(&call.steps) {
            let camera = (spec.camera.and_then(|arg| step.args.get(arg))).and_then(Value::as_str);
            for name in view_of(origin, spec, step)
                .into_iter()
                .chain(camera.map(str::to_string))
            {
                views.make(&name, origin);
            }
        }
    }

    /// Closes the gestures a call's camera commands do not continue, before it runs, and gives
    /// back their lines, for the call to write before its own once it succeeded.
    pub(crate) fn end_gestures_before(
        &mut self,
        origin: &Origin,
        call: &Call,
        specs: &[&CommandSpec],
    ) -> Vec<Ended> {
        let views = &mut self.state.views;
        for (spec, step) in specs.iter().zip(&call.steps) {
            let Some(view) = view_of(origin, spec, step).filter(|_| spec.kind == Kind::View) else {
                continue;
            };
            let id = step.args.get("gesture").and_then(Value::as_u64);
            if views.gesture(&view).is_some_and(|open| Some(open) != id) {
                views.close(&view);
            }
        }
        views.take_ended()
    }

    /// Writes the lines of the gestures that ended and the views kept framed or made since the
    /// last time (see [`Engine::log_view_lines`]).
    pub(crate) fn log_ended_gestures(&mut self) {
        let ended = self.state.views.take_ended();
        let seq = self.next_seq;
        self.next_seq += ended.len() as u64;
        self.log_view_lines(ended, seq);
    }

    /// Writes one `view.set` line for each of `ended`, numbered from `seq`: the camera a gesture
    /// ended on, a view kept framed or a view made, as the client that made it. Nothing runs;
    /// the view already has that camera.
    pub(crate) fn log_view_lines(&mut self, ended: Vec<Ended>, mut seq: u64) {
        let Ok(spec) = self.registry.find("view.set") else {
            return;
        };
        for Ended {
            view,
            origin,
            camera,
            viewport,
            remember,
        } in ended
        {
            let mut args = json!({
                "view": view,
                "viewport": viewport,
                "camera": ExactCamera::from(camera),
            });
            if !remember {
                args["remember"] = false.into();
            }
            let call = Call {
                steps: vec![Step {
                    command: spec.name.into(),
                    args: args.clone(),
                }],
                if_rev: None,
                dry_run: false,
                batch: false,
            };
            let first = seq;
            seq += 1;
            let ran = Ran {
                spec,
                args,
                output: Value::Null,
                noop_reason: None,
                warnings: Vec::new(),
                logged: true,
                landing: None,
            };
            let done = Done {
                ran: vec![ran],
                changes: Vec::new(),
                doc_replaced: false,
                reads: crate::document::Reads::default(),
                log_as: None,
                active: self.state.doc.active_id(),
            };
            self.commit(&origin, &call, first, done, true);
        }
    }
}

/// Every part as the window draws it, taking its changes, in list order (decision 0028), which
/// the window lists them in; `hiding` while Dynamic isolate hides the other parts.
fn part_reports(doc: &mut Document, hiding: bool) -> Vec<PartReport<'_>> {
    let order: BTreeMap<String, usize> = (doc.list().into_iter())
        .enumerate()
        .map(|(i, (name, _))| (name.to_string(), i))
        .collect();
    let isolation = doc.isolation().clone();
    let active = doc.active_id();
    let mut parts: Vec<PartReport> = (doc.parts_mut())
        .map(|(name, part)| {
            let dirty = part.take_dirty();
            let part: &Part = part;
            let showing = Showing {
                isolation: &isolation,
                active,
            };
            PartReport {
                name,
                id: part.id(),
                store: part.store(),
                dirty,
                shown: part.shown(showing),
                drawn: crate::part::drawn(part, showing, hiding),
                active: active.is_some() && part.props().id == active,
                visible: !part.props().hidden,
                painted: !part.props().paint_hidden,
                transform: part.transform(),
            }
        })
        .collect();
    parts.sort_by_key(|p| order.get(p.name).copied());
    parts
}

/// Every open Reference View, by number, as the window draws it: whether it is the active one,
/// its camera, and its own scene's parts (sent whole: they never change), or `None` for the
/// session's parts, live.
fn shown_references<'a>(views: &'a Views, doc: &Document) -> Vec<ShownReference<'a>> {
    let active = views.active_reference();
    let shown = |view: String| {
        let parts = match &views.secondary(&view)?.scene {
            Scene::Live => None,
            Scene::Own(own) => Some(
                (own.parts())
                    .map(|(name, part)| PartReport {
                        name,
                        id: part.id(),
                        store: part.store(),
                        dirty: Vec::new(),
                        shown: part.shown(own.showing()),
                        // A reference's own scene is still while main moves.
                        drawn: part.shown(own.showing()),
                        visible: !part.props().hidden,
                        painted: !part.props().paint_hidden,
                        // A reference's own scene: nothing there is the session's active part.
                        active: false,
                        transform: part.transform(),
                    })
                    .collect(),
            ),
        };
        Some(ShownReference {
            active: active.as_ref() == Some(&view),
            shown: MainView::of(&views.info(&view, doc)),
            setup: reference_state(views, view)?,
            parts,
        })
    };
    views.references().into_iter().filter_map(shown).collect()
}
