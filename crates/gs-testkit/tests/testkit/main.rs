//! gs-testkit self-tests (ARCHITECTURE 10.1, item 4): every oracle accepts known-good input and
//! rejects the known-bad input it exists to catch, and every fixture really is the shape it
//! claims to be. One test binary, so the suite links once.

mod bench_shapes;
mod claims;
mod closed;
mod compare;
mod data;
mod fixtures;
mod hashcheck;
mod measure;
mod model;
mod plan;
mod props;
mod reference;
mod rng;
mod session;
mod stroke_ref;
mod symmetry;
mod validate;
