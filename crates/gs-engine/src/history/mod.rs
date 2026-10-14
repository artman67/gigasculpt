//! Undo history: storage, memory accounting, and the `history.*` commands.

mod checkpoint;
mod held;
mod list;
mod store;
mod undo;

use gs_protocol::CommandError;

pub(crate) use store::{Change, Checkpoint, Ends, Entry, History, Listing, Moved, Snapshot};

use crate::document::DocTxn;
use crate::errors;

/// The history as a running command sees it: the session's own, or, in a job, the snapshot
/// taken when the job started (decision 0010). Only session commands change the history, and
/// those never run in a job.
pub(crate) enum HistoryRef<'a> {
    Session(&'a mut History),
    Job(&'a Snapshot),
}

impl HistoryRef<'_> {
    /// The same history, borrowed again for one command.
    pub fn reborrow(&mut self) -> HistoryRef<'_> {
        match self {
            Self::Session(h) => HistoryRef::Session(h),
            Self::Job(s) => HistoryRef::Job(s),
        }
    }

    /// The listing `history.list` and `scene.describe` show: the history as it was when the
    /// call began, in place as in a job.
    pub fn listing(&self, doc: &DocTxn) -> Listing {
        match self {
            Self::Session(h) => h.listing(doc.committed().into_iter()),
            Self::Job(s) => s.listing.clone(),
        }
    }

    /// The history's two ends, for `session.state`.
    pub fn ends(&self) -> Ends {
        match self {
            Self::Session(h) => h.ends(),
            Self::Job(s) => s.ends.clone(),
        }
    }

    pub fn checkpoint_named(&self, name: &str) -> Result<&Checkpoint, CommandError> {
        match self {
            Self::Session(h) => h.checkpoint_named(name),
            Self::Job(s) => s.checkpoint_named(name),
        }
    }

    pub fn budget_bytes(&self) -> u64 {
        match self {
            Self::Session(h) => h.budget_bytes,
            Self::Job(s) => s.listing.budget_bytes,
        }
    }

    /// The session's history, to change it. Session commands only, which never run in a job.
    pub fn session(&mut self) -> Result<&mut History, CommandError> {
        match self {
            Self::Session(h) => Ok(h),
            Self::Job(_) => Err(errors::internal(
                "a job reached a command that changes the history",
            )),
        }
    }
}
