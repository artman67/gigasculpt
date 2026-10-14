//! Protected: build agents in the loop cannot change this crate.
//!
//! The oracles every GigaSculpt test leans on. Every one has a negative test in `tests/`
//! proving it rejects the known-bad input it exists to catch (ARCHITECTURE 10.1, item 4), and
//! `tests/canary/` registers deliberately buggy commands that the claims harness must catch.
//!
//! | Module | What it is |
//! |---|---|
//! | [`session`] | `Session`: one engine driven in-process by command name and JSON, as clients drive it; can record every call for replay |
//! | [`compare`] | The one float and mesh comparer: NaN counts as infinitely far, tolerances are ten times the worst observed, canonical bytes for bit-identity |
//! | [`validate`](mod@validate) | The mesh validator: always finite, in range, no repeated corner, no unused vertex, attributes the right length; manifold, closed and Euler characteristic where declared |
//! | [`hashcheck`] | The hasher check: any hash must change for every one-bit change and every rewiring (run on the engine's `mesh.hash`) |
//! | [`measure`] | Counts, area, volume, bounding box and two-sided Hausdorff distance, in `f64` |
//! | [`reference`](mod@reference) | Slow references for what the engine reports (the effect of an edit) and for one dab of each brush kernel |
//! | [`rng`] | `SplitMix64`, the seeded generator for test data |
//! | [`fixtures`] | Tetrahedron, icosphere, cube, torus, open patch and a quad-and-triangle cylinder, each checked to be the shape it claims |
//! | [`bench_shapes`] | Research 28's sphere, head, ring and sleeve at a size given in faces, the inputs Auto UV is measured on |
//! | [`symmetry`] | Metamorphic helpers: the 48 cube symmetries for meshes and arguments, and vertex and face renumbering |
//! | [`claims`] | The claims vocabulary, the history model and the generic runner over random command sequences |
//! | [`golden`] | Golden images: scenes as command steps rendered on lavapipe, the 10.3 comparison rule, the pending queue, automated approval rules, the maintainer's digest, and the `golden` tool |
//!
//! # Wiring
//!
//! `cargo mutants` runs only the mutated package's tests, so a crate whose code commands
//! reach runs the claims harness from its own `tests/` folder too (see [`claims`]).

#![expect(
    clippy::result_large_err,
    reason = "CommandError is the engine's wire error type; tests return it as is"
)]

pub mod bench_shapes;
pub mod claims;
pub mod compare;
pub mod fixtures;
pub mod golden;
pub mod hashcheck;
pub mod measure;
pub mod reference;
pub mod rng;
pub mod session;
pub mod symmetry;
pub mod validate;

pub use rng::SplitMix64;
pub use session::{Exchange, Scene, Session, code, failure, success, text};
pub use validate::{Defect, Expect, validate};
