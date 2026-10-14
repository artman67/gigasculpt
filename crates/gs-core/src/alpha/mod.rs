//! Alphas: grey pictures that shape each dab (decision 0018, ARCHITECTURE 4.2).
//!
//! - [`Picture`]: a 16-bit grey picture that never changes, named by the hash of its pixels.
//! - How a brush uses one is the brush's (`crate::brush::AlphaSpec`, the `[tip.alpha]` table).
//! - [`Pyramid::build`]: one pure function from picture plus settings to the processed picture
//!   at every level; the engine caches the result by picture hash and tone hash.
//! - [`Pyramid::sample`]: the one sampling function. A stroke picks one level per dab
//!   ([`Pyramid::level_for`]) and reads 4 texels per weighted vertex; [`AlphaRead`] turns the
//!   value into the influence factor by the kernel's sign rule.
//! - [`Recipe`]: our own pictures as data (`assets/alphas/`), drawn by our own generator. Never
//!   a picture taken from another program.
//!
//! The vector kind (three signed floats per pixel, research 16, 7.2) waits for phase B6.

mod noise;
mod picture;
mod pyramid;
mod recipe;
mod tone;

pub use picture::{MOST, Picture};
pub use pyramid::{AlphaRead, Pyramid, Sign};
pub use recipe::{Layer, Op, Recipe};
