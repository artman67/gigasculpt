//! Session commands: facts about the session, its state as named values, and the switches a
//! human uses to keep agents out of the way.

mod control;
mod info;
mod state;

pub use state::{ReferenceState, SessionState, StateValues};
pub(crate) use state::{StateSources, reference_state, state_of};
