//! gs-geo voxel remesh tests. Only the kinds ARCHITECTURE 10.1 admits: oracle self-tests,
//! properties of the output checked with independent oracles, differential tests against slow
//! references, determinism across thread counts, and work-counter contracts.

mod attributes;
mod common;
mod field;
mod holes;
mod oracles;
mod order;
mod parts;
mod shape;
mod winding;

mod close_holes;

mod fold;
mod operand;
