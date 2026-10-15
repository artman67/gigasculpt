//! The runner: which commands random plans (see `plan`) call and the claims they are held to,
//! how many plans run, and whether every claim was checked. Each plan is checked by
//! [`Harness::check`] (see `check`). Proptest supplies the seeds, shrinks a failure to a short
//! sequence, and reads `PROPTEST_CASES` and `PROPTEST_RNG_SEED` from the environment, as the
//! gate sets them.

mod check;

pub use check::replay;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

use gs_protocol::{Catalog, CommandInfo, ErrorCode, Kind, Step};
use proptest::collection::vec;
use proptest::prelude::any;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};
use serde_json::Value;

use super::plan::{as_given, plan};
use super::{Claim, declared};
use crate::compare::Observed;
use crate::fixtures;

/// What a run checked: per command and claim, how many steps were checked, and for claims with
/// a tolerance, the worst difference observed (to set tolerances at ten times it).
#[derive(Clone, Debug, Default)]
pub struct Tally {
    /// Checks made, by (command, claim).
    pub checks: BTreeMap<(String, String), u32>,
    /// Worst difference seen, by (command, claim).
    pub observed: BTreeMap<(String, String), Observed>,
}

impl Tally {
    /// How many times `claim` was checked on `command`.
    pub fn count(&self, command: &str, claim: &str) -> u32 {
        let key = (command.to_string(), claim.to_string());
        self.checks.get(&key).copied().unwrap_or_default()
    }

    fn note(&mut self, command: &str, claim: &str) {
        *self
            .checks
            .entry((command.into(), claim.into()))
            .or_default() += 1;
    }

    fn observe(&mut self, command: &str, claim: &str, diff: f64) {
        self.note(command, claim);
        let key = (command.to_string(), claim.to_string());
        self.observed.entry(key).or_default().note(diff);
    }

    fn merge(&mut self, other: Self) {
        for (key, n) in other.checks {
            *self.checks.entry(key).or_default() += n;
        }
        for (key, seen) in other.observed {
            self.observed.entry(key).or_default().note(seen.max);
        }
    }
}

/// A broken claim, with the whole sequence that broke it: a ready-made bug report.
#[derive(Clone, Debug)]
pub struct Violation {
    /// The claim, or `valid`, `effect` or `history` for the checks every call gets.
    pub claim: String,
    /// The command that broke it.
    pub command: String,
    /// Its arguments.
    pub args: Value,
    /// The step's number in the sequence, if it was one of the plan's steps.
    pub step: Option<usize>,
    /// What went wrong.
    pub message: String,
    /// Every step of the sequence.
    pub steps: Vec<Step>,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = self.step.map_or(String::new(), |i| format!(" at step {i}"));
        writeln!(
            f,
            "{} broken by {} {}{at}: {}",
            self.claim, self.command, self.args, self.message
        )?;
        for (i, step) in self.steps.iter().enumerate() {
            writeln!(f, "  {i}. {} {}", step.command, step.args)?;
        }
        Ok(())
    }
}

/// The generic harness: which commands to call, and the claims to hold them to.
#[derive(Clone, Debug)]
pub struct Harness {
    claims: BTreeMap<String, Vec<Claim>>,
    pool: Vec<CommandInfo>,
    crashers: Vec<String>,
    first: Vec<Step>,
    setups: BTreeMap<String, Vec<Step>>,
}

impl Harness {
    /// Every edit command in the catalog, held to the claims it declares, with the setups
    /// commands name read and checked.
    pub fn new(catalog: &Catalog) -> Result<Self, String> {
        let edits = catalog.commands.iter().filter(|c| c.kind == Kind::Edit);
        let crashes = |c: &&CommandInfo| {
            c.examples
                .iter()
                .any(|e| e.fails_with == Some(ErrorCode::Internal))
        };
        let found = declared(catalog)?;
        Ok(Self {
            claims: (found.iter().filter(|d| !d.claims.is_empty()))
                .map(|d| (d.command.clone(), d.claims.clone()))
                .collect(),
            pool: edits
                .clone()
                .filter(|c| !c.examples.is_empty())
                .cloned()
                .collect(),
            crashers: edits.filter(crashes).map(|c| c.name.clone()).collect(),
            first: Vec::new(),
            setups: (found.iter().filter(|d| !d.setup.is_empty()))
                .map(|d| (d.command.clone(), d.setup.clone()))
                .collect(),
        })
    }

    /// Only these edit commands get called.
    #[must_use]
    pub fn only(mut self, commands: &[&str]) -> Self {
        self.pool.retain(|c| commands.contains(&c.name.as_str()));
        self
    }

    /// Holds `command` to `claims` instead of what it declares.
    #[must_use]
    pub fn claiming(mut self, command: &str, claims: Vec<Claim>) -> Self {
        self.claims.insert(command.into(), claims);
        self
    }

    /// Starts every plan with the calls of `first` (arguments an object, without `part`), made
    /// once on each part the plan loads, before the random steps. For a command
    /// that changes something only in a state a random plan seldom reaches
    /// (`part.apply_transform` needs a part with a transform): the step puts every part in
    /// that state, so its claims are checked on every run and not by luck. A setup call that
    /// fails, or answers that it changed nothing, fails the run as a harness error, and so
    /// does a run in which it never ran (review 0008, T1). A command names its setup in the
    /// catalog instead, for [`Harness::check_share`].
    #[must_use]
    pub fn starting_with(mut self, first: &[Step]) -> Self {
        self.first.extend_from_slice(first);
        self
    }

    /// Share `k` of `shares` of the edit commands random plans call, dealt in catalog order as
    /// cards are: every `shares`-th command from the `k`-th. Share 0 of 1 is every one.
    pub fn share(&self, k: usize, shares: usize) -> Vec<String> {
        (self.pool.iter().skip(k).step_by(shares.max(1)))
            .map(|c| c.name.clone())
            .collect()
    }

    /// [`Harness::run_until_checked`] on share `k` of `shares` of the edit commands (see
    /// [`Harness::share`]), each held to the claims it declares. The shares together call every
    /// command and check every claim, each share in a test of its own, so nextest runs them side
    /// by side; a plan calls the commands of one share. A command that names a setup is held to
    /// its claims in its setup's run instead ([`Harness::check_setup_share`]).
    pub fn check_share(&self, k: usize, shares: usize) -> Result<Tally, String> {
        if k >= shares {
            return Err(format!("there is no share {k} of {shares}"));
        }
        let names = self.share(k, shares);
        let mine: Vec<&str> = names.iter().map(String::as_str).collect();
        self.clone().only(&mine).run_until_checked()
    }

    /// Share `k` of `shares` of the commands that name a setup in the catalog, dealt in name
    /// order as [`Harness::share`] deals the edit commands.
    pub fn setup_share(&self, k: usize, shares: usize) -> Vec<String> {
        (self.setups.keys().skip(k).step_by(shares.max(1)))
            .cloned()
            .collect()
    }

    /// [`Harness::check_setup_of`] on each command of setup share `k` of `shares` (see
    /// [`Harness::setup_share`]): each in a test of its own beside the shares, so a long
    /// setup's run does not wait for its command's share. Call it on the harness of the whole
    /// catalog ([`Harness::new`]): a setup's run takes the commands its setup calls from there.
    pub fn check_setup_share(&self, k: usize, shares: usize) -> Result<Tally, String> {
        if k >= shares {
            return Err(format!("there is no setup share {k} of {shares}"));
        }
        let mut tally = Tally::default();
        for command in self.setup_share(k, shares) {
            tally.merge(self.check_setup_of(&command)?);
        }
        Ok(tally)
    }

    /// The run of a command that names a setup in the catalog: plans of the command and the edit
    /// commands its setup calls, each starting with the setup on every part, run until every
    /// claim is checked. Only these plans start with that setup, so it never changes what other
    /// commands are checked against (review 0008). A failure names the command whose setup it
    /// is.
    pub fn check_setup_of(&self, command: &str) -> Result<Tally, String> {
        let Some(setup) = self.setups.get(command) else {
            return Err(format!("{command} names no setup in the catalog"));
        };
        let mut pool: Vec<&str> = setup.iter().map(|call| call.command.as_str()).collect();
        pool.push(command);
        let own = Self {
            first: setup.clone(),
            ..self.clone().only(&pool)
        };
        (own.run_until_checked())
            .map_err(|e| format!("{command}, on plans that start with its catalog setup: {e}"))
    }

    /// Runs random plans (`PROPTEST_CASES` of them for every five edit commands) and requires
    /// every check to pass and every claim to have been checked at least once. Whether a random
    /// call lands in the state a claim needs (the mask on the masked fixture, a mirror plane its
    /// frame is off) is luck, and any new command deals every plan again, so each command with a
    /// claim the random plans left unchecked then runs its examples as given
    /// ([`Harness::as_given`]), which no seed changes. Only while a claim is still unchecked do
    /// up to [`TOP_UPS`] more rounds of random plans run, from the next seeds, and they call only
    /// the commands still unchecked (with the setup calls, if any): the others have had their
    /// first round's calls.
    pub fn run_until_checked(&self) -> Result<Tally, String> {
        let mut tally = self.run()?;
        for command in self.unchecked(&tally) {
            tally.merge(self.as_given(&command)?);
        }
        for round in 1..=TOP_UPS {
            let unchecked = self.unchecked(&tally);
            if unchecked.is_empty() {
                break;
            }
            let names: Vec<&str> = unchecked.iter().map(String::as_str).collect();
            tally.merge(self.clone().only(&names).run_round(round)?);
        }
        self.require_checked(&tally)?;
        Ok(tally)
    }

    /// The plans of `command`'s examples as given ([`as_given`]), one on each standard
    /// fixture, each starting with the setup calls, if any.
    pub fn as_given(&self, command: &str) -> Result<Tally, String> {
        let Some(info) = self.pool.iter().find(|c| c.name == command) else {
            return Err(format!("{command} is not called by this harness"));
        };
        let mut tally = Tally::default();
        for k in 0..fixtures::all().len() {
            let plan = as_given(info, k).starting_with(&self.first);
            tally.merge(self.check(&plan).map_err(|v| v.to_string())?);
        }
        Ok(tally)
    }

    /// Runs random plans; the first failure, shrunk, as a bug report.
    pub fn run(&self) -> Result<Tally, String> {
        self.run_round(0)
    }

    /// [`Harness::run`] from the configured seed plus `round` (a random seed stays random).
    fn run_round(&self, round: u64) -> Result<Tally, String> {
        let pool: Vec<&CommandInfo> = self.pool.iter().collect();
        let tally = RefCell::new(Tally::default());
        let mut config = settings();
        if let RngSeed::Fixed(seed) = config.rng_seed {
            config.rng_seed = RngSeed::Fixed(seed.wrapping_add(round));
        }
        config.cases = config.cases.saturating_mul(per_five(pool.len()));
        let result = TestRunner::new(config).run(&vec(any::<u64>(), 2..=10), |seeds| {
            let found = self.check(&plan(&pool, &seeds).starting_with(&self.first));
            tally
                .borrow_mut()
                .merge(found.map_err(|v| TestCaseError::fail(v.to_string()))?);
            Ok(())
        });
        result
            .map(|()| tally.into_inner())
            .map_err(|e| e.to_string())
    }

    /// Fails unless every command was called, and every command with claims had each of its
    /// claims, plus exact undo and atomicity, checked at least once: a run that never
    /// exercised a claim proves nothing about it. A command that names a setup in the catalog
    /// is held to that only on plans that start with its setup; elsewhere it must only have
    /// been called, as a command without claims. Every setup call must have run, too.
    pub fn require_checked(&self, tally: &Tally) -> Result<(), String> {
        for call in &self.first {
            if tally.count(&call.command, SETUP) == 0 {
                let (command, args) = (&call.command, &call.args);
                return Err(format!("the setup call {command} {args} never ran"));
            }
        }
        for info in &self.pool {
            if let Some(missing) = self.missing(info, tally) {
                return Err(format!("{}: {missing} was never checked", info.name));
            }
        }
        Ok(())
    }

    /// The pool's commands with a claim [`Harness::require_checked`] holds them to that `tally`
    /// has not checked, in pool order.
    fn unchecked(&self, tally: &Tally) -> Vec<String> {
        (self.pool.iter())
            .filter(|info| self.missing(info, tally).is_some())
            .map(|info| info.name.clone())
            .collect()
    }

    /// The first claim [`Harness::require_checked`] holds `info` to that `tally` has not checked.
    fn missing(&self, info: &CommandInfo, tally: &Tally) -> Option<&'static str> {
        let elsewhere = (self.setups.get(&info.name)).is_some_and(|s| *s != self.first);
        let required: Vec<&'static str> = match self.claims.get(&info.name) {
            Some(claims) if !elsewhere => {
                (claims.iter().chain(&Claim::UNIVERSAL).map(Claim::name)).collect()
            }
            _ => vec![Claim::Deterministic.name()],
        };
        required
            .into_iter()
            .find(|c| tally.count(&info.name, c) == 0)
    }
}

/// What a broken setup call is reported as, and what the tally counts its runs under.
const SETUP: &str = "setup";

/// How many further rounds [`Harness::run_until_checked`] runs at most while a claim is
/// unchecked.
const TOP_UPS: u64 = 6;

/// How many times the configured case count a pool of `commands` edit commands runs: once per
/// five commands or part of five, so each command is called about 30 times at the gate's 32
/// cases however many the catalog holds. Commands that change something only in some states
/// (a mask filter needs a mask with edges) need that many to land a change.
fn per_five(commands: usize) -> u32 {
    commands.div_ceil(5).max(1) as u32
}

/// Proptest's settings for the runner: case count and seed from the environment
/// (`PROPTEST_CASES`, `PROPTEST_RNG_SEED`), and shrinking stopped after 16 tries. That is
/// enough to drop the steps a failure does not need (later tries only shrink seed values), and
/// keeps a failing run, or a failing mutant in a mutation run, to seconds.
pub fn settings() -> Config {
    Config {
        max_shrink_iters: 16,
        ..Config::default()
    }
}
