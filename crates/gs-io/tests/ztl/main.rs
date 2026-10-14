//! gs-io tool and project file reader tests. Kinds from ARCHITECTURE 10.1: differential tests
//! (files written by our own writer from the format notes, read back field by field, and the
//! subdivision numbering against faces worked out by hand), oracle tests (known-bad files
//! rejected with the right error and place), and fuzz properties over mutated, spliced and
//! cut-off files. No test reads a real tool or project file.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap and expect in tests"
)]

/// The brush writer's container pieces (objects, planar arrays, names, settings).
#[expect(
    dead_code,
    reason = "only the container half of the brush writer is needed here"
)]
#[path = "../zbp/writer.rs"]
mod brush;
mod errors;
mod fuzz;
mod reads;
mod sample;
mod writer;
