//! The mask commands (decisions 0015 and 0026): `mask.set`, `mask.invert`, `mask.filter` and
//! `mask.make`. Each is one undo step and runs through gs-core's `mask` module, the one home of
//! the mask's rules, which the Mask Pen reads too.

mod filter;
mod invert;
mod make;
mod set;

use gs_core::mask::Scope;
use gs_core::{MeshStore, NodeId, PosRead};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which points a mask command writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScopeArg {
    /// Visible points only: hidden ones keep their mask.
    #[default]
    Visible,
    /// Every point, hidden ones too.
    All,
}

impl ScopeArg {
    fn core(self) -> Scope {
        match self {
            ScopeArg::Visible => Scope::Visible,
            ScopeArg::All => Scope::All,
        }
    }

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_visible(&self) -> bool {
        *self == ScopeArg::Visible
    }
}

/// The part's mask after the command.
#[derive(Serialize, JsonSchema)]
pub struct MaskReport {
    /// The part.
    pub part: String,
    /// Points fully masked (1).
    pub masked: u64,
    /// Points partly masked (between 0 and 1).
    pub partial: u64,
    /// Points free (0).
    pub free: u64,
}

impl MaskReport {
    pub(super) fn of(part: &str, store: &MeshStore) -> MaskReport {
        let mut out = MaskReport {
            part: part.to_string(),
            masked: 0,
            partial: 0,
            free: 0,
        };
        let mask = store
            .attr_id(gs_protocol::MASK)
            .map(|m| store.attr_reader(m));
        let pos = store.pos();
        for n in 0..store.node_count() as NodeId {
            let node = store.node(n);
            for v in (node.base..).take(node.pos.len()) {
                if pos.get(v).is_dead() {
                    continue;
                }
                match mask.map_or(0.0, |r| r.scalar(v)) {
                    m if m >= 1.0 => out.masked += 1,
                    m if m <= 0.0 => out.free += 1,
                    _ => out.partial += 1,
                }
            }
        }
        out
    }
}
