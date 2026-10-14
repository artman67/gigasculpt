//! Comparing pictures: `image.diff` (ARCHITECTURE 7.5), for example two `view.snapshot`
//! results before and after a change; and the 8-bit RGBA PNGs it and the looks read and write.

mod diff;
mod picture;

pub(crate) use picture::{Picture, decode, encode};
