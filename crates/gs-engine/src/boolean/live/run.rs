//! The live mode's runs: each group cut on a thread of its own, one at a time, and handed back
//! to the engine thread through a channel.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use super::cache::{self, Cache, Slot};
use super::pieces::{self, Mark};
use super::{Board, Group, Key, Member, Settled};
use crate::boolean::apply::{dropped, nothing, operand};
use crate::boolean::ladder::{self, Made, Operand, Solver};
use crate::document::{Part, VertexOrder};
use crate::mesh::{FIRST_RESOLUTION, digest};

/// Wakes the engine thread when a run hands its work back (the server's `Task::Live`).
type Waker = Arc<dyn Fn() + Send + Sync>;

/// The live mode's runs, on the engine thread's side: the operand cache, how many runs still
/// owe an answer, and the channel they answer through.
pub(crate) struct Live {
    pub(super) cache: Cache,
    pub(super) owed: usize,
    to: Sender<Ran>,
    pub(super) from: Receiver<Ran>,
    wake: Option<Waker>,
    /// Held by the run that works, taken on its own thread, so one works at a time and none
    /// waits for another's import inside the pool, where a waiting thread can pick up queued
    /// work (another run) and wait on itself.
    turn: Arc<Mutex<()>>,
}

/// A run's answer: its result and its marks, or `None` when its cancel flag stopped it.
pub(super) struct Ran {
    pub(super) run: u64,
    pub(super) outcome: Option<(Result<Settled, String>, Vec<Mark>)>,
}

/// One run's work, moved to its thread.
struct Run {
    number: u64,
    members: Arc<[Member]>,
    checks: [bool; 2],
    slots: Vec<Slot>,
    cancel: Arc<AtomicBool>,
    imports: Arc<AtomicU64>,
    to: Sender<Ran>,
    wake: Option<Waker>,
    turn: Arc<Mutex<()>>,
}

impl Default for Live {
    fn default() -> Self {
        let (to, from) = mpsc::channel();
        Self {
            cache: Cache::default(),
            owed: 0,
            to,
            from,
            wake: None,
            turn: Arc::default(),
        }
    }
}

impl Live {
    /// Has the engine thread woken by `wake` whenever a run hands its work back.
    pub(crate) fn wake_with(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.wake = Some(Arc::new(wake));
    }

    /// Starts a run of `key`'s group on a thread of its own, counted on `board`, marking the
    /// checks `checks` asks for (Show Issues, Show Coplanar).
    pub(super) fn start(&mut self, board: &mut Board, key: &[Key], checks: [bool; 2]) -> Group {
        board.runs += 1;
        let members: Arc<[Member]> = (key.iter())
            .map(|&(name, part, operation)| Member {
                name: name.to_string(),
                part: part.share(),
                operation,
            })
            .collect();
        let run = Run {
            number: board.runs,
            slots: key.iter().map(|k| self.cache.slot(k.1)).collect(),
            members: Arc::clone(&members),
            checks,
            cancel: Arc::default(),
            imports: Arc::clone(&board.imports),
            to: self.to.clone(),
            wake: self.wake.clone(),
            turn: Arc::clone(&self.turn),
        };
        let group = Group {
            members,
            run: board.runs,
            cancel: Arc::clone(&run.cancel),
            outcome: None,
            checks,
            marks: Arc::new([]),
            last: None,
        };
        self.owed += 1;
        let thread = thread::Builder::new().name(format!("live boolean {}", group.run));
        if let Err(e) = thread.spawn(move || run.go()) {
            let why = format!("its run could not start a thread: {e}");
            let outcome = Some((Err(why), Vec::new()));
            let run = group.run;
            self.to.send(Ran { run, outcome }).ok();
        }
        group
    }
}

impl Run {
    /// Cuts the group in its turn and hands back what came of it, even when the cut panics.
    fn go(self) {
        let turn = self.turn.lock().unwrap_or_else(PoisonError::into_inner);
        let cut = || gs_par::background(|| self.cut());
        let outcome = catch_unwind(AssertUnwindSafe(cut)).unwrap_or_else(|_| {
            let why = "the live run stopped on an internal error";
            Some((Err(why.into()), Vec::new()))
        });
        drop(turn);
        let run = self.number;
        self.to.send(Ran { run, outcome }).ok();
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    /// The group cut as `boolean.apply` with `auto` at 128 cuts it, or the sentence Apply
    /// would refuse it with, and its operands' marks; `None` once its cancel flag is set,
    /// checked before reading the operands, before the cut and after it.
    fn cut(&self) -> Option<(Result<Settled, String>, Vec<Mark>)> {
        let stopped = || self.cancel.load(Ordering::Relaxed);
        if stopped() {
            return None;
        }
        let operands: Result<Vec<Operand>, _> = (self.members.iter())
            .map(|m| operand(&m.name, &m.part, m.operation))
            .collect();
        let operands = match operands {
            Ok(operands) => operands,
            Err(e) => return Some((Err(e.message), Vec::new())),
        };
        if stopped() {
            return None;
        }
        let import = |i: usize| cache::import(&self.slots[i], &operands[i], &self.imports);
        let made = ladder::cut(&operands, Solver::Auto, FIRST_RESOLUTION, &import);
        if stopped() {
            return None;
        }
        let marks = pieces::marks(&operands, self.checks);
        let made = made.map_err(|e| e.message);
        Some((
            made.and_then(|made| settled(&self.members, &operands, made)),
            marks,
        ))
    }
}

/// `made` as Apply would insert it, made from `members`, or Apply's refusal of a cut that
/// leaves nothing.
fn settled(members: &Arc<[Member]>, operands: &[Operand], made: Made) -> Result<Settled, String> {
    if made.mesh.faces.is_empty() {
        return Err(nothing(&operands[0].name).message);
    }
    let dropped = dropped(operands, &made.mesh);
    let walls = pieces::walls(members, &made.mesh, &made.owners);
    let part = Part::new(made.mesh, VertexOrder::Spatial, "boolean").map_err(|e| e.message)?;
    let world = part.world();
    Ok(Settled {
        from: Arc::clone(members),
        walls,
        hash: digest(&world).to_hex().to_string(),
        faces: world.faces.len(),
        points: world.positions.len(),
        part,
        kept: made.kept,
        solver: made.solver,
        repairs: made.repairs,
        warnings: made.warnings,
        dropped,
    })
}
