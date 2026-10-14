//! The live mode as `session.state` reports it (`boolean.live`).

use std::sync::atomic::Ordering;

use serde::Serialize;

use super::{Board, Group};
use crate::boolean::ladder::Solver;

/// The live mode as `session.state` reports it (`boolean.live`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct LiveState {
    /// Each group of two parts or more that show, in list order; none while the setting is off.
    pub groups: Vec<LiveGroup>,
    /// Runs started since the session began.
    pub runs: u64,
    /// Operands imported (gs-geo's steps 1 to 4) since the session began.
    pub imports: u64,
    /// Runs a later change cancelled before they landed.
    pub cancelled: u64,
    /// Runs whose result landed.
    pub landed: u64,
}

/// One group as `session.state` reports it.
#[derive(Clone, Debug, Serialize)]
pub struct LiveGroup {
    /// Its parts, base first.
    pub operands: Vec<String>,
    /// `running`, `settled` or `refused`.
    pub state: &'static str,
    /// A settled result's hash, as `mesh.hash` hashes a part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faces: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
    /// The path that cut it: `manifold` or `voxel`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver: Option<Solver>,
    /// Why a refused group has no result: the sentence `boolean.apply` would refuse it with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Board {
    /// What `session.state` reports.
    pub(crate) fn state(&self) -> LiveState {
        LiveState {
            groups: self.groups.iter().map(Group::state).collect(),
            runs: self.runs,
            imports: self.imports.load(Ordering::Relaxed),
            cancelled: self.cancelled,
            landed: self.landed,
        }
    }
}

impl Group {
    fn state(&self) -> LiveGroup {
        let (state, settled, why) = match &self.outcome {
            None => ("running", None, None),
            Some(Ok(settled)) => ("settled", Some(settled), None),
            Some(Err(why)) => ("refused", None, Some(why.clone())),
        };
        LiveGroup {
            operands: self.members.iter().map(|m| m.name.clone()).collect(),
            state,
            hash: settled.map(|s| s.hash.clone()),
            faces: settled.map(|s| s.faces),
            points: settled.map(|s| s.points),
            solver: settled.map(|s| s.solver),
            why,
        }
    }
}
