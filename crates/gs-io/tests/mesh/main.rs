//! gs-io OBJ, PLY and STL tests. Kinds from ARCHITECTURE 10.1: round trips through each writer
//! and reader (exact for what the format holds), files built by hand from the public format
//! descriptions read back value by value, damaged files rejected with the line or byte offset,
//! every cut-off length of a small file, and fuzz properties (mutated, spliced and random
//! files never panic, and what reads is a consistent mesh).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap in tests"
)]
#![expect(
    clippy::format_push_string,
    reason = "tests build small files as text; speed does not matter"
)]

mod common;
mod fuzz;
mod obj;
mod ply;
mod stl;
