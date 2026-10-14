//! One plan, checked: its calls run through a recording session, and after every call the
//! scene is checked against the history model, the validator and the engine's own report of
//! the edit; after every successful edit undo and redo are probed and the command's declared
//! claims checked. Beside it, a fresh session on a thread of its own replays the recording call
//! by call and requires bit-identical responses.

mod again;

use std::panic;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;

use gs_protocol::{CmdResponse, Outcome, Step, Success};
use serde_json::{Value, json};

use super::{Harness, SETUP, Tally, Violation};
use crate::claims::model::{Call, Model};
use crate::claims::plan::Plan;
use crate::claims::{Claim, props, shape};
use crate::compare::scene_bytes;
use crate::session::{Exchange, Scene, Session};

impl Harness {
    /// Runs one plan and checks everything. Its determinism replay runs beside it in a twin
    /// session on a thread of its own, handed each step's call as the plan makes it and the
    /// recording a step at a time, so every call is still made twice but both at once, and no
    /// recording is ever held whole (decision 0008, amended 2026-10-12). A broken check of the
    /// plan's own is reported before a replay difference, as when the replay came after. A
    /// difference is blamed on the step during which it showed, naming the call that answered
    /// differently (often a `part.get` after the step).
    pub fn check(&self, plan: &Plan) -> Result<Tally, Violation> {
        thread::scope(|scope| {
            let (feed, recording) = sync_channel(2);
            let twin = scope.spawn(move || replayed(plan.seed, recording));
            let run = Run {
                harness: self,
                plan,
                s: Session::headless_seeded(plan.seed).recording(),
                tally: Tally::default(),
                feed,
            };
            let checked = run.all();
            let differs = twin.join();
            let tally = checked?;
            let Some((k, call, step)) = differs.unwrap_or_else(|crash| panic::resume_unwind(crash))
            else {
                return Ok(tally);
            };
            let (command, args) = match step.and_then(|i| plan.steps.get(i)) {
                Some(during) => (&during.command, &during.args),
                None => (&call.command, &call.args),
            };
            let message = format!(
                "call {k} of the recording, {} {}, answered differently on replay",
                call.command, call.args
            );
            Err(Violation {
                claim: "deterministic".into(),
                command: command.clone(),
                args: args.clone(),
                step,
                message,
                steps: plan.steps.clone(),
            })
        })
    }
}

/// A setup call ([`Harness::starting_with`]) is a harness error, not a dropped step, when it
/// fails or answers that it changed nothing: the state it was meant to make is missing, and
/// the claims it was there for would be checked by luck again. Says which call, on which part.
fn set_up(step: &Step, outcome: &Outcome) -> Result<(), String> {
    let part = &step.args["part"];
    let why = match outcome {
        Outcome::Error(e) => format!("failed: {}", e.message),
        Outcome::Ok(done) => match done.effect.as_ref().and_then(|e| e.noop_reason.as_ref()) {
            Some(reason) => format!("changed nothing: {reason}"),
            None => return Ok(()),
        },
    };
    Err(format!(
        "the setup call {} on part {part} {why}; the setup is wrong, not the command",
        step.command
    ))
}

/// The first call whose response differs when `transcript` is replayed into a fresh session.
pub fn replay(seed: u64, transcript: &[Exchange]) -> Option<usize> {
    replayed(seed, [Feed::Made(transcript.to_vec())]).map(|(k, _, _)| k)
}

/// What a plan hands its twin, in the order it calls.
enum Feed {
    /// The call the plan is about to make, so the twin makes it at the same time.
    Next(Step),
    /// The calls the plan made since it last handed any on, with their answers.
    Made(Vec<Exchange>),
}

/// The twin's replay: makes every call the plan made, in order, in a fresh session with the
/// plan's seed, keeping none once compared, and returns the first whose answer differs, with
/// its number and the number of the step it came in (none before the first). A call handed on
/// as [`Feed::Next`] is made as it comes, before the plan's answer.
fn replayed(
    seed: u64,
    feed: impl IntoIterator<Item = Feed>,
) -> Option<(usize, Exchange, Option<usize>)> {
    let mut s = Session::headless_seeded(seed).recording();
    let (mut k, mut step) = (0, None);
    for handed in feed {
        let calls = match handed {
            Feed::Next(call) => {
                step = Some(step.map_or(0, |i| i + 1));
                s.call(&call.command, call.args);
                continue;
            }
            Feed::Made(calls) => calls,
        };
        for exchange in calls {
            if s.transcript().is_empty() {
                s.call(&exchange.command, exchange.args.clone());
            }
            if s.take_transcript().first() != Some(&exchange) {
                return Some((k, exchange, step));
            }
            k += 1;
        }
    }
    None
}

/// The scene as the harness sees it: every part as `part.get` shows it, and the engine's scene
/// hash, which also covers what `part.get` cannot show (the levels of a part with detail
/// levels, decision 0014), and the active part, which a command naming no part works on.
struct Seen {
    scene: Scene,
    hash: String,
    active: Option<String>,
}

impl Seen {
    fn of(s: &mut Session) -> Self {
        let state = s.query("session.state", json!({}));
        Self {
            scene: s.scene(),
            hash: s.hash(),
            active: state["parts.active"].as_str().map(str::to_string),
        }
    }

    /// Both, as bytes to compare.
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = scene_bytes(&self.scene);
        bytes.extend_from_slice(self.hash.as_bytes());
        bytes
    }
}

/// `args` with the active part written in at the pointer `part` when they name none, as the
/// engine pins it (decision 0033).
fn with_active(args: &Value, part: &str, active: Option<&str>) -> Value {
    let mut args = args.clone();
    let key = part.strip_prefix('/').filter(|k| !k.contains('/'));
    if let (None, Some(key), Some(active)) = (args.pointer(part), key, active) {
        args[key] = json!(active);
    }
    args
}

/// One plan in progress.
struct Run<'a> {
    harness: &'a Harness,
    plan: &'a Plan,
    s: Session,
    tally: Tally,
    /// Where each step's call and the recording go: the twin session that replays them.
    feed: SyncSender<Feed>,
}

type Broken = (&'static str, String);

/// A successful step as its claims see it: the call, its success, and the scene before and
/// after.
struct Made<'a> {
    step: &'a Step,
    done: &'a Success,
    before: &'a Seen,
    after: &'a Seen,
}

/// What checking one claim on one step came to.
enum Checked {
    /// Nothing to check on this step.
    Not,
    /// It held.
    Held,
    /// It held, with this worst difference.
    Within(f64),
}

impl Run<'_> {
    /// Loads the plan's parts and runs its steps, checking each, and hands the recording on as
    /// it goes; the first broken check, as a bug report.
    fn all(mut self) -> Result<Tally, Violation> {
        let plan = self.plan;
        let mut model = Model::new(Seen::of(&mut self.s).bytes());
        for fixture in &plan.fixtures {
            self.s.add_part(fixture.name, &fixture.mesh);
            let loaded = Seen::of(&mut self.s).bytes();
            let args = json!({ "name": fixture.name });
            model
                .step(Call::Edit, true, true, loaded)
                .map_err(|m| self.blame("history", "part.create", &args, None, m))?;
        }
        let mut before = Seen::of(&mut self.s);
        for (i, step) in plan.steps.iter().enumerate() {
            self.hand_on(Some(step));
            let response = self.s.call(&step.command, step.args.clone());
            if i < plan.setup {
                set_up(step, &response.outcome)
                    .map_err(|m| self.blame(SETUP, &step.command, &step.args, Some(i), m))?;
                self.tally.note(&step.command, SETUP);
            }
            let after = Seen::of(&mut self.s);
            self.step(i, step, &response, &before, &after, &mut model)
                .map_err(|(claim, m)| self.blame(claim, &step.command, &step.args, Some(i), m))?;
            before = after;
        }
        self.hand_on(None);
        Ok(self.tally)
    }

    /// Hands the twin the calls recorded since the last time and then `next`, the call the plan
    /// is about to make, if any. A twin that has stopped (it found a difference, or panicked)
    /// takes no more, and its join says why.
    fn hand_on(&mut self, next: Option<&Step>) {
        let _stopped = self.feed.send(Feed::Made(self.s.take_transcript()));
        if let Some(step) = next {
            let _stopped = self.feed.send(Feed::Next(step.clone()));
        }
    }

    /// A broken check as a bug report: the call that broke it, and every step of the plan.
    fn blame(
        &self,
        claim: &str,
        command: &str,
        args: &Value,
        step: Option<usize>,
        message: String,
    ) -> Violation {
        Violation {
            claim: claim.into(),
            command: command.into(),
            args: args.clone(),
            step,
            message,
            steps: self.plan.steps.clone(),
        }
    }

    fn step(
        &mut self,
        i: usize,
        step: &Step,
        response: &CmdResponse,
        seen_before: &Seen,
        seen_after: &Seen,
        model: &mut Model,
    ) -> Result<(), Broken> {
        let (before, after) = (&seen_before.scene, &seen_after.scene);
        let command = step.command.as_str();
        let crashes = self.harness.crashers.iter().any(|c| c == command);
        props::no_crash(&response.outcome, crashes).map_err(|m| ("atomic", m))?;
        let done = match &response.outcome {
            Outcome::Ok(done) => Some(done),
            Outcome::Error(_) => None,
        };
        props::sound(before, after).map_err(|m| ("valid", m))?;
        let call = Call::of(command, &step.args);
        let made_step = done.is_some_and(|d| d.history_id.is_some());
        let label = if done.is_some() { "history" } else { "atomic" };
        model
            .step(call, done.is_some(), made_step, seen_after.bytes())
            .map_err(|m| (label, m))?;
        self.history_matches(command, model)?;
        if !self.harness.pool.iter().any(|c| c.name == command) {
            return Ok(());
        }
        self.tally.note(command, "deterministic");
        let Some(done) = done else {
            self.tally.note(command, "atomic");
            return Ok(());
        };
        let changed = seen_before.hash != seen_after.hash;
        props::effect_matches(done.effect.as_ref(), before, after, changed)
            .map_err(|m| ("effect", m))?;
        self.tally.note(command, "effect");
        if let Some(id) = done.history_id {
            self.undo_exact(command, id, seen_before, seen_after)?;
        }
        let made = Made {
            step,
            done,
            before: seen_before,
            after: seen_after,
        };
        self.claims(i, &made)
    }

    /// `history.list` agrees with the model about what can be undone and redone.
    fn history_matches(&mut self, command: &str, model: &Model) -> Result<(), Broken> {
        let history = self.s.query("history.list", json!({ "limit": 1 }));
        props::counts_match(&history, model.undoable(), model.redoable())
            .map_err(|m| ("history", m))?;
        self.tally.note(command, "history");
        Ok(())
    }

    /// Undo puts `before` back exactly, and redo `after`.
    fn undo_exact(
        &mut self,
        command: &str,
        id: u64,
        before: &Seen,
        after: &Seen,
    ) -> Result<(), Broken> {
        for (probe, want) in [("history.undo", before), ("history.redo", after)] {
            let ok = self.s.run(probe, json!({ "expect": id })).is_ok();
            props::restored(ok, &Seen::of(&mut self.s).bytes(), &want.bytes())
                .map_err(|m| ("undo_exact", format!("{probe} of step {id}: {m}")))?;
        }
        self.tally.note(command, "undo_exact");
        Ok(())
    }

    /// The command's declared claims, each checked on the step and counted.
    fn claims(&mut self, i: usize, made: &Made) -> Result<(), Broken> {
        let harness = self.harness;
        let command = made.step.command.as_str();
        for claim in harness.claims.get(command).into_iter().flatten() {
            let name = claim.name();
            match self.claim(claim, i, made).map_err(|m| (name, m))? {
                Checked::Not => {}
                Checked::Held => self.tally.note(command, name),
                Checked::Within(seen) => self.tally.observe(command, name, seen),
            }
        }
        Ok(())
    }

    /// One claim, checked on the step.
    fn claim(&mut self, claim: &Claim, i: usize, made: &Made) -> Result<Checked, String> {
        let (before, after) = (&made.before.scene, &made.after.scene);
        let (step, args) = (made.step, &made.step.args);
        let held = |checked: bool| if checked { Checked::Held } else { Checked::Not };
        let within = |seen: Option<f64>| seen.map_or(Checked::Not, Checked::Within);
        Ok(match claim {
            Claim::PreservesTopology => {
                props::preserves_topology(before, after)?;
                Checked::Held
            }
            Claim::Local {
                part,
                center,
                radius,
            } => {
                let support = center.as_deref().zip(radius.as_deref());
                let args = with_active(args, part, made.before.active.as_deref());
                props::local(before, after, &args, part, support)?;
                Checked::Held
            }
            Claim::VolumeNonDecreasing { tolerance } => {
                let (checked, shrink) = props::volume_non_decreasing(before, after, *tolerance)?;
                within((checked >= 1).then_some(shrink))
            }
            Claim::MirrorEquivariant { vectors, tolerance } => {
                let seen = self.mirror(i, step, made.before, after, vectors, *tolerance)?;
                Checked::Within(seen)
            }
            Claim::MaskHolds { switch } => {
                held(shape::mask_holds(before, after, args, switch.as_deref())?)
            }
            Claim::HiddenHolds => held(shape::hidden_holds(before, after)?),
            Claim::PositionsOnly => {
                shape::positions_only(before, after)?;
                Checked::Held
            }
            Claim::IdentityAtZero { set } => held(self.identity(made, set)?),
            Claim::PlacementInvariant {
                pinned,
                unless,
                tolerance,
            } => within(self.placement(made, pinned, unless.as_deref(), *tolerance)?),
            Claim::KeepsSymmetry {
                planes,
                origin,
                pinned,
                tolerance,
            } => within(self.symmetry(made, (planes, origin), pinned, *tolerance)?),
            Claim::Closed => held(crate::claims::closed::closed(before, after)?),
            Claim::Deterministic | Claim::UndoExact | Claim::Atomic => Checked::Not,
        })
    }

    /// Runs the step again on a mirrored copy of the scene, with the same part active, in a
    /// fresh session, and compares with the mirrored result.
    fn mirror(
        &self,
        i: usize,
        step: &Step,
        before: &Seen,
        after: &Scene,
        vectors: &[String],
        tolerance: f64,
    ) -> Result<f64, String> {
        let g = self.plan.mirrors[i];
        let mut s = Session::headless_seeded(self.plan.seed);
        for (name, mesh) in &before.scene {
            s.add_part(name, &g.mesh(mesh));
        }
        if let Some(active) = &before.active {
            s.ok("part.select", json!({ "part": active }));
        }
        if let Err(e) = s.run(&step.command, g.args(&step.args, vectors)) {
            return Err(format!("{g:?}: the mirrored call failed: {}", e.message));
        }
        let want: Scene = after.iter().map(|(n, m)| (n.clone(), g.mesh(m))).collect();
        props::matches(&want, &s.scene(), tolerance).map_err(|m| format!("{g:?}: {m}"))
    }
}
