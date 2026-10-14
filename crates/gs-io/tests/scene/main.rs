//! gs-io scene file tests (decision 0016). Kinds from ARCHITECTURE 10.1: round trips (a store
//! written and read back is the same store, bit for bit), oracle tests (damaged, cut-off and
//! too-new files are refused with the right error and place), fault injection at every file
//! operation of the safe replace, and fuzz properties over mutated files.
#![expect(
    clippy::expect_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap and expect in tests"
)]

mod crafted;
mod errors;
mod fuzz;
mod hints;
mod levels;
mod reads;
mod replace;
mod sample;
