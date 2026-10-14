//! Golden images (ARCHITECTURE 10.3, `docs/spikes/golden.md`).
//!
//! A **scene** is a TOML file in `goldens/scenes/`: what it checks, a session seed, a camera
//! preset, its looks (`zebra`, `clay`, `matcap:<name>`, `lit:<name>`), whether every part
//! must be a closed manifold, and a list of command steps:
//!
//! ```toml
//! about = "Voxel Remesh rounds a cube's edges into an even quad surface."
//! view = "iso"
//! looks = ["zebra", "clay"]
//! closed = true
//!
//! [[step]]
//! command = "primitive.cube"
//!
//! [[step]]
//! command = "mesh.voxel_remesh"
//! args = { resolution = 48 }
//! ```
//!
//! An optional `[show]` table says what the parts' surfaces show (decision 0039, section 9):
//! `colors = false` draws every part's material only, `face_groups = true` the view's face
//! group colors.
//!
//! Each scene runs in a fresh in-process session and every look is rendered offscreen at
//! 512x512 through gs-render ([`render`]). The picture `<scene>.<look>.png` sits next to its
//! manifest `<scene>.<look>.toml`, which names the adapter, driver and wgpu version.
//!
//! [`compare`] applies the rule exactly as 10.3 states it: byte equality on the same renderer;
//! otherwise zebra images pass at FLIP worst pixel 0.30, look images within 2 sRGB levels per
//! channel or else with at most 50 pixels over FLIP 0.06 and none over 0.15; never the mean.
//!
//! # Folders
//!
//! - `goldens/reference/`: the approved pictures (protected).
//! - `goldens/previous/`: references an approval replaced, until the maintainer keeps or
//!   reverts them; the digest shows exactly these.
//! - `target/golden/pending/`: the queue. A check writes a picture here, never over a
//!   reference, when it does not pass or has no reference yet.
//! - `target/golden/failures/<name>/`: `expected.png`, `actual.png` and `diff.png`.
//!
//! # Commands
//!
//! `cargo run -p gs-testkit --bin golden -- <command>` ([`cli`]):
//!
//! - `check` renders every scene on lavapipe and queues what does not pass. The test kit's
//!   own golden test does the same on every `cargo test`.
//! - `review` reports the queue against the references, with a sheet of before, after and
//!   difference map (`target/golden/review.png`), and the approvals awaiting the maintainer.
//! - `decide <request.toml>` applies the automated approval rules ([`decide`]) for the catalog
//!   item being built: approve, reject (a regression, failed mesh checks, no stated
//!   expectation, the expected change missing) or escalate (an artistic change).
//! - `approve` and `reject` empty the queue; `approve` keeps each replaced reference.
//! - `digest` writes the maintainer's contact sheet of every changed reference, old next to
//!   new; `keep` confirms approvals and `revert` undoes one.

pub mod cli;
mod compare;
mod queue;
mod rules;
mod scene;

pub use compare::{
    LOOK_LEVELS, LOOK_MAX, LOOK_PIXEL, LOOK_PIXELS, Rule, Verdict, ZEBRA_MAX, compare, diff_map,
};
pub use queue::{Goldens, Manifest, Outcome, Status, Stored, names, read, write};
pub use rules::{Decision, Request, Ruling, decide};
pub use scene::{GoldenScene, Look, SIZE, Show, render};
