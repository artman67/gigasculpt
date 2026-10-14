//! Live Boolean's operand cache (decision 0057, section 8, as amended for step LB6): one import
//! per body (gs-geo's steps 1 to 4 of the part's own mesh), keyed by `Part::same_body` alone, so
//! a cutter moved, turned or re-marked imports nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use gs_geo::boolean::{Prepared, Unfit, prepare};

use super::Key;
use crate::boolean::ladder::Operand;
use crate::document::Part;

/// The most triangles the cache keeps imports of: about 230 bytes a triangle (the library's
/// arrays and collision tree beside gs-geo's copies), so about 1.8 GB.
const CACHE_TRIANGLES: usize = 8_000_000;

/// One body's import, made by the first run that needs it and read by the rest.
pub(super) type Slot = Arc<OnceLock<Result<Arc<Prepared>, Unfit>>>;

/// The imports the live mode keeps, on the engine thread's side.
#[derive(Default)]
pub(super) struct Cache(Vec<Import>);

/// A body in the cache.
struct Import {
    /// A share of a part with the body, for `Part::same_body`.
    part: Part,
    triangles: usize,
    slot: Slot,
}

impl Cache {
    /// The slot of `part`'s body: the cache's, or a new one the cache keeps while it has room.
    pub(super) fn slot(&mut self, part: &Part) -> Slot {
        if let Some(held) = self.0.iter().find(|i| i.part.same_body(part)) {
            return Arc::clone(&held.slot);
        }
        let slot = Slot::default();
        let triangles = part.store().face_count();
        let cached: usize = self.0.iter().map(|i| i.triangles).sum();
        if cached + triangles <= CACHE_TRIANGLES {
            self.0.push(Import {
                part: part.share(),
                triangles,
                slot: Arc::clone(&slot),
            });
        }
        slot
    }

    /// Drops every import whose body no operand of `wanted` has (every one, given none).
    pub(super) fn prune(&mut self, wanted: &[Vec<Key>]) {
        let used = |i: &Import| wanted.iter().flatten().any(|k| i.part.same_body(k.1));
        self.0.retain(used);
    }
}

/// `operand`'s import from `slot`, made there by the first run that needs it and counted on
/// `count`.
pub(super) fn import(
    slot: &Slot,
    operand: &Operand,
    count: &AtomicU64,
) -> Result<Arc<Prepared>, Unfit> {
    let made = || {
        count.fetch_add(1, Ordering::Relaxed);
        prepare(&operand.local).map(Arc::new)
    };
    slot.get_or_init(made).clone()
}
