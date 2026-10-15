//! The claims harness: commands declare what they promise as data, and one generic runner
//! checks every promise on random command sequences (ARCHITECTURE 10.1).
//!
//! # Where claims are declared
//!
//! Next to the command, as a JSON array in its `CLAIMS` constant (beside `examples()`), for
//! example in `crates/gs-engine/src/mesh/translate.rs`:
//!
//! ```text
//! const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}}]"#;
//! ```
//!
//! The engine copies them into the command catalog (`CommandInfo::claims`), so agents and the
//! `gs` client can read them, and the harness reads them from there. The engine never
//! interprets them. This module, which build agents cannot edit, owns the vocabulary
//! ([`Claim`]) and what each claim means: a declaration the harness does not understand fails
//! the run. A new brush adds one line of data and no test code; weakening a claim is a
//! one-line change anyone can read in a diff.
//!
//! Every edit command is held to [`Claim::Deterministic`], [`Claim::UndoExact`] and
//! [`Claim::Atomic`] whether it declares them or not (ARCHITECTURE 10.1, item 1), and every
//! state must pass the validator. Declaring them only documents intent.
//!
//! # Claims random calls seldom reach
//!
//! Random plans change every number of an example and pick parts at random, so a claim checked
//! only on some calls (`mask_holds` with the mask on, on the one masked fixture;
//! `keeps_symmetry` with a mirror plane its frame is off) is reached now and then, and any
//! command added deals every plan again. So when a run's random plans leave a claim unchecked,
//! the command's own examples run as given ([`run::Harness::as_given`]): unchanged, on each
//! fixture, the same on every run. Give a command examples that reach each of its claims that
//! way; a claim they miss is left to more rounds of random plans, which is luck.
//!
//! # Commands that need a state first
//!
//! A command that changes something only after another call has set a state up (a part must
//! have a transform before `part.apply_transform` bakes it) is seldom checked by random plans
//! over the whole catalog, and whether it is changes with every command added. Such a command
//! names the setting-up calls beside its claims, in its `SETUP` constant, written as an
//! example's setup is, without the part (the harness calls each on every part):
//!
//! ```text
//! const SETUP: &'static str = r#"[{"command": "gizmo.place", "args": {"origin": [0, 0, 0], "normal": [1, 1, 0]}}]"#;
//! ```
//!
//! [`run::Harness::check_setup_share`] then checks it in a run of its own
//! ([`run::Harness::check_setup_of`]): plans of the command and the edit commands its setup
//! calls, each starting with the setup. The setup heads no other plan, so it never changes
//! what other commands are checked against, and outside its own run such a command is only
//! required to be called, as a command without claims is (review 0008). A setup call that fails
//! or changes nothing fails the run, naming the call, the part and the command, as one that
//! never ran does. Name a setup only for such a command; a single case that once broke stays a
//! test.
//!
//! # Wiring
//!
//! `cargo mutants` runs only the tests of the package it mutates, so each crate whose code
//! commands reach calls [`run::Harness::check_share`] on every share of the catalog's edit
//! commands and [`run::Harness::check_setup_share`] on every share of the commands that name a
//! setup, one test per share, in its own `tests/` folder (gs-engine does, in
//! `tests/engine/claims.rs`, with tests that the shares cover every edit command and the setup
//! tests every command that names a setup).

pub mod closed;
pub mod model;
pub mod plan;
pub mod props;
pub mod run;
pub mod shape;

use std::collections::BTreeMap;

use gs_protocol::{Catalog, Kind, Step};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One promise a command makes. Pointers are JSON pointers into the command's arguments, where
/// a `*` step means every element of an array (`/samples/*/position`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Claim {
    /// Never adds or removes parts, and never changes a part's faces, vertex count or the
    /// names, domains and lengths of its attributes. Positions and attribute values may change.
    PreservesTopology,
    /// Changes only the part named at `part` (or the only part, when the argument is left
    /// out). With `center` and `radius`, also leaves every vertex of that part farther than
    /// `radius` from every center bit-identical. `radius` is a length: scene units, or a
    /// percentage of the part's bounding-box diagonal (`"5%"`).
    Local {
        /// Pointer to the part argument.
        part: String,
        /// Pointer to the support centers (one or more `[x, y, z]`).
        #[serde(default)]
        center: Option<String>,
        /// Pointer to the support radius.
        #[serde(default)]
        radius: Option<String>,
    },
    /// Never shrinks a closed part's volume by more than `tolerance` times the volume before,
    /// past what rounding its points to `f32` can change it by (a flat part's volume is noise).
    VolumeNonDecreasing {
        /// Relative tolerance: ten times the worst shrink observed on correct results.
        tolerance: f64,
    },
    /// Commutes with every mirror and axis swap (the 48 cube symmetries): transforming the
    /// scene and the vectors at `vectors`, then running the command, gives the transformed
    /// result, vertex by vertex, within `tolerance` times the largest coordinate (at least 1).
    MirrorEquivariant {
        /// Pointers to the arguments that are points or directions.
        vectors: Vec<String>,
        /// Tolerance: ten times the worst difference observed on correct results.
        tolerance: f64,
    },
    /// The same session seed and the same calls give bit-identical responses. Every edit.
    Deterministic,
    /// Undo puts the scene back bit for bit, and redo restores the result. Every edit.
    UndoExact,
    /// A failed call changes nothing: not the scene, not the history. Every edit.
    Atomic,
    /// Leaves every vertex masked 1 or more (or not a finite number) bit-identical, unless the
    /// argument at `switch` is `false`.
    MaskHolds {
        /// Pointer to the argument that turns the mask off when `false`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        switch: Option<String>,
    },
    /// Leaves every vertex all of whose faces are hidden bit-identical.
    HiddenHolds,
    /// Changes only positions: faces and every attribute stay bit-identical.
    PositionsOnly,
    /// The call made again from the scene before, with the arguments at `set`'s pointers that
    /// it has given those values, succeeds, reports that it changed nothing and changes no bit
    /// (undo, the call, redo, in the same session).
    IdentityAtZero {
        /// The identity: a value for each pointer, such as `/op/bend/degrees` at 0.
        set: BTreeMap<String, Value>,
    },
    /// Lands within `tolerance` times the largest coordinate (at least 1) of the call made
    /// again in a fresh session on the scene before as parts without a transform (as
    /// `part.get` shows them), with what the result reports at each of `pinned`'s values
    /// written into the arguments at its key. Skipped when the call gives the argument at
    /// `unless`.
    PlacementInvariant {
        /// Argument pointer to result pointer: what the call worked out, used as given.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pinned: BTreeMap<String, String>,
        /// Pointer to an argument under which the claim does not hold (planes of the part's own).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unless: Option<String>,
        /// Tolerance: ten times the worst difference observed on correct results.
        tolerance: f64,
    },
    /// Across each plane switched on at `planes` (`x`, `y`, `z`) that the result's point at
    /// `origin` is off, the call made as [`Claim::PlacementInvariant`] makes it, on the mirror
    /// image of the scene before (faces reversed, masks and hidden faces kept), is the mirror
    /// image of that call on the scene itself, within `tolerance` times its largest coordinate
    /// (at least 1). So a part symmetric across the plane stays symmetric, and the claim is
    /// checked on any part, symmetric or not.
    KeepsSymmetry {
        /// Pointer to the argument switching the planes on.
        planes: String,
        /// Pointer to the result's frame origin.
        origin: String,
        /// As [`Claim::PlacementInvariant`]'s.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pinned: BTreeMap<String, String>,
        /// Tolerance: ten times the worst difference observed on correct results.
        tolerance: f64,
    },
    /// Every part the call adds is closed and manifold, and no two of its faces that share no
    /// point cross ([`closed`]). Checked when the call adds a part.
    Closed,
}

impl Claim {
    /// The claims every edit command is held to.
    pub const UNIVERSAL: [Self; 3] = [Self::Deterministic, Self::UndoExact, Self::Atomic];

    /// The claim's name, as written in declarations.
    pub fn name(&self) -> &'static str {
        match self {
            Self::PreservesTopology => "preserves_topology",
            Self::Local { .. } => "local",
            Self::VolumeNonDecreasing { .. } => "volume_non_decreasing",
            Self::MirrorEquivariant { .. } => "mirror_equivariant",
            Self::Deterministic => "deterministic",
            Self::UndoExact => "undo_exact",
            Self::Atomic => "atomic",
            Self::MaskHolds { .. } => "mask_holds",
            Self::HiddenHolds => "hidden_holds",
            Self::PositionsOnly => "positions_only",
            Self::IdentityAtZero { .. } => "identity_at_zero",
            Self::PlacementInvariant { .. } => "placement_invariant",
            Self::KeepsSymmetry { .. } => "keeps_symmetry",
            Self::Closed => "closed",
        }
    }
}

/// What one command declares in the catalog: its claims, and the setup its own run starts
/// every plan with (empty when it names none).
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// The command's name.
    pub command: String,
    /// The claims it declares.
    pub claims: Vec<Claim>,
    /// The calls that set a state up before it changes anything.
    pub setup: Vec<Step>,
}

/// What each command that declares claims or names a setup declares, in catalog order. Fails
/// on either from a command that is not an edit, on a claim the vocabulary does not have, on a
/// setup call to a command the catalog does not have, and on setup arguments that are not an
/// object (left out means none, as in an example's setup).
pub fn declared(catalog: &Catalog) -> Result<Vec<Declaration>, String> {
    let mut out = Vec::new();
    for info in &catalog.commands {
        let name = &info.name;
        if info.claims.is_empty() && info.setup.is_empty() {
            continue;
        }
        if info.kind != Kind::Edit {
            let what = if info.claims.is_empty() {
                "names a setup"
            } else {
                "declares claims"
            };
            return Err(format!("{name} {what} but is not an edit command"));
        }
        let claims = info
            .claims
            .iter()
            .map(|c| serde_json::from_value(c.clone()))
            .collect::<Result<Vec<Claim>, _>>()
            .map_err(|e| format!("{name} declares a claim the harness does not know: {e}"))?;
        for call in &info.setup {
            let command = &call.command;
            if !catalog.commands.iter().any(|c| c.name == *command) {
                return Err(format!(
                    "{name}'s setup calls {command}, which is not in the catalog"
                ));
            }
            if !(call.args.is_object() || call.args.is_null()) {
                return Err(format!(
                    "{name}'s setup calls {command} with arguments that are not an object: {}",
                    call.args
                ));
            }
        }
        out.push(Declaration {
            command: name.clone(),
            claims,
            setup: info.setup.clone(),
        });
    }
    Ok(out)
}
