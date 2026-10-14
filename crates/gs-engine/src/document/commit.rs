//! The commit: what a transaction touched, turned into the history's changes and what the
//! effect reports, one [`PartChange`] per part that really changed. `DocTxn::finish` calls it
//! for each part the call touched, after the checks pass.

use gs_core::PosRead;

use super::changed::Changed;
use super::{Document, Order, Part, Props};
use crate::effect::{self, Names, Touched};
use crate::history::Change;
use crate::mesh::digest;

/// One part's change, ready to commit: what the history keeps, and what the effect reports.
pub(crate) struct PartChange {
    pub change: Change,
    /// The vertices it touched.
    pub touched: Touched,
    pub topology_changed: bool,
    /// What changed, by name (see `effect`).
    pub changed: Names,
}

/// Keeps a part's new properties, or nothing when they ended up the same. A part whose
/// transform changed touches every vertex whose scene point moved.
pub(super) fn commit_props(doc: &mut Document, name: &str, before: Props) -> Option<PartChange> {
    let part = doc.get(name)?;
    if part.props == before {
        return None;
    }
    let touched = if part.transform() == before.transform {
        Touched::default()
    } else {
        part.touched_moved(&before.transform)
    };
    Some(PartChange {
        change: Change::Props {
            part: name.to_string(),
            other: before,
        },
        touched,
        topology_changed: false,
        changed: effect::props(&before, &part.props),
    })
}

/// Closes an in-place edit's step, or cancels it if nothing changed.
pub(super) fn commit_edit(
    doc: &mut Document,
    name: &str,
    changed: Option<Changed>,
) -> Option<PartChange> {
    let part = doc.get_mut(name)?;
    let Some(changed) = changed.filter(Changed::any) else {
        part.store_mut().cancel_step();
        return None;
    };
    let summary = part.store_mut().end_step();
    // A slot numbering no longer holds once topology changed: the part takes the store's own
    // order, and the history keeps the one it had (decision 0021, item 11.4). No UV corner
    // turns here (`MeshStore::store_corners`): the one edit that changes topology in place,
    // Live Detail, refuses a part with UVs (item 11), whose corners would first have to be
    // carried through each split and collapse.
    let order = (summary.topology && matches!(part.order(), Order::Given(_))).then(|| {
        let mut spatial = Order::Spatial;
        part.swap_order(&mut spatial);
        spatial
    });
    let touched = if changed.reshaped {
        part.touched_whole()
    } else {
        let (numbering, store) = (part.numbering(), part.store());
        let (cfg, pos, t) = (store.config(), store.pos(), part.transform());
        let mut points: Vec<(u32, [f32; 3])> = (changed.touched.iter())
            .map(|&v| {
                let at = numbering.index(cfg.node_of(v), cfg.local_of(v));
                (at, t.point(pos.get(v)).to_array())
            })
            .collect();
        points.sort_unstable_by_key(|t| t.0);
        Touched::of(points.iter().map(|t| t.1))
    };
    let names = if summary.topology {
        effect::whole(part)
    } else {
        let moved = (!changed.moved.is_empty()).then(|| effect::POSITIONS.to_string());
        changed.columns.into_iter().chain(moved).collect()
    };
    Some(PartChange {
        change: Change::Edited {
            part: name.to_string(),
            step: summary.id,
            order,
        },
        touched,
        topology_changed: summary.topology,
        changed: names,
    })
}

/// Keeps a part that was added, removed or replaced, or puts the old one back if it ended up
/// the same. A part added or removed counts every vertex as touched, read from its store; a
/// part replaced is compared with the old one as plain mesh data in the scene. When either has
/// detail levels, frozen or not, the old part is kept even if what `part.get` shows is the same
/// (the levels below can differ), a change of the active level's point or face count is a
/// topology change read without exporting either, and two active levels cut and numbered alike
/// under one transform are compared node by node without exporting either. The same body
/// under another transform touches the vertices whose scene point moved.
pub(super) fn commit_swap(
    doc: &mut Document,
    name: &str,
    before: Option<Box<Part>>,
) -> Option<PartChange> {
    let (touched, topology_changed, changed) = match (before.as_deref(), doc.get(name)) {
        (None, None) => return None,
        (Some(old), Some(new)) if new.same_as(old) => {
            doc.swap(name, before);
            return None;
        }
        (Some(old), Some(new))
            if [old, new]
                .iter()
                .any(|p| p.levels().is_some() || p.frozen().is_some()) =>
        {
            let counts = |p: &Part| (p.store().vertex_count(), p.store().face_count());
            let mut names = effect::props(&old.props, &new.props);
            names.extend(effect::levels(old, new));
            if new.store().same_columns(old.store()) && new.order().same(&old.order()) {
                (new.touched_moved(&old.transform()), false, names)
            } else if counts(new) != counts(old) {
                names.extend(effect::whole(old));
                names.extend(effect::whole(new));
                (new.touched_whole(), true, names)
            } else {
                // New columns over the same faces: a whole-part command run on the top and
                // the active level gathered again, or a restore that only drops a column.
                // Compared node by node when both are cut and numbered alike and at the
                // identity, where part points are scene points; exporting both costs far more
                // on a big part (review 0006, S1).
                let order = new.order();
                let identity = old.transform().is_identity() && new.transform().is_identity();
                let alike = (identity && order.same(&old.order()))
                    .then(|| order.numbering(new.store()))
                    .and_then(|n| effect::swapped_alike(old.store(), new.store(), &n));
                if let Some((touched, found)) = alike {
                    names.extend(found);
                    (touched, false, names)
                } else {
                    let (was, now) = (old.world(), new.world());
                    let (touched, topology, found) = effect::swapped(old, new, &was, &now);
                    names.extend(found);
                    (touched, topology, names)
                }
            }
        }
        // The same mesh with other properties (a list edit, decision 0028): nothing to export.
        (Some(old), Some(new))
            if new.store().same_columns(old.store()) && new.order().same(&old.order()) =>
        {
            let names = effect::props(&old.props, &new.props);
            (new.touched_moved(&old.transform()), false, names)
        }
        (Some(old), Some(new)) => {
            let (was, now) = (old.world(), new.world());
            if digest(&was) == digest(&now) && new.props == old.props {
                doc.swap(name, before);
                return None;
            }
            let (touched, topology, mut names) = effect::swapped(old, new, &was, &now);
            names.extend(effect::props(&old.props, &new.props));
            (touched, topology, names)
        }
        (Some(part), None) | (None, Some(part)) => {
            (part.touched_whole(), true, effect::whole(part))
        }
    };
    Some(PartChange {
        change: Change::Swapped {
            part: name.to_string(),
            other: before,
        },
        touched,
        topology_changed,
        changed,
    })
}
