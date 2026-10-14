//! The only crate that touches SDL3: the window, its events, and pen input as `PenSample`.
//!
//! [`Platform`] opens the window and turns SDL's events into [`Input`]: pen frames merged into
//! one sample each, mouse events dropped while a pen is in range, coordinates in physical
//! pixels. [`SurfaceHandle`] lets the renderer make a GPU surface for the window without
//! `unsafe`. The pen rules are pure ([`PenTracker`], [`EventMapper`]), so they are tested
//! without a window.

mod clock;
mod input;
mod pen;
mod surface;
mod window;
#[cfg(feature = "xvfb")]
pub mod xvfb;

pub use clock::Clock;
pub use input::{Button, EventMapper, Input, Modifiers, pen_event};
pub use pen::{PenChange, PenEvent, PenTracker};
pub use surface::SurfaceHandle;
pub use window::{Platform, Pumped, WindowOptions};
