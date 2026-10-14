//! The golden-image pipeline (ARCHITECTURE 10.3): the repository's scenes against their
//! references, the comparison rule's negative tests, the pending queue, the approval rules and
//! the `golden` tool. One binary, so lavapipe opens once per run.
#![expect(
    clippy::expect_used,
    reason = "test helpers fail loudly; clippy.toml allows expect in tests"
)]

mod cli;
mod common;
mod compare;
mod queue;
mod references;
mod rules;
