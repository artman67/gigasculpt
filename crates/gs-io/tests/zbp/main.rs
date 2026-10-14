//! gs-io brush file reader tests. Kinds from ARCHITECTURE 10.1: differential tests (files
//! written by our own writer from the format notes, read back field by field), oracle tests
//! (known-bad files rejected with the right error and place), and fuzz properties over mutated,
//! spliced and cut-off files. No test reads a real brush file.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap and expect in tests"
)]

mod errors;
mod fuzz;
mod reads;
mod sample;
mod writer;
