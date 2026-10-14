//! The part list's groups (decision 0057, section 5): a group runs from a part with `start`
//! (the first part always starts one) to the part before the next, and the parts that do not
//! show are dropped after, so hiding a group's start does not merge its group into the one above.

use gs_protocol::PartRef;

use crate::document::{Document, Operation};

/// One part of the list, as the groups read it.
pub(super) struct Listed {
    pub name: String,
    /// Its group's number, from 0 in list order.
    pub group: usize,
    pub operation: Operation,
    pub shown: bool,
}

/// Every part, in list order.
pub(super) fn listed(doc: &Document) -> Vec<Listed> {
    let showing = doc.showing();
    let mut out: Vec<Listed> = Vec::new();
    for (name, part) in doc.list() {
        let props = part.props();
        let group = out.last().map_or(0, |p| p.group + usize::from(props.start));
        out.push(Listed {
            name: name.to_string(),
            group,
            operation: props.operation,
            shown: part.shown(showing),
        });
    }
    out
}

/// The groups with a part that shows, each its parts that show with their operations.
pub(super) fn shown(doc: &Document) -> Vec<Vec<(PartRef, Operation)>> {
    let mut groups: Vec<Vec<(PartRef, Operation)>> = Vec::new();
    let mut last = None;
    for part in listed(doc).into_iter().filter(|p| p.shown) {
        if last != Some(part.group) {
            groups.push(Vec::new());
            last = Some(part.group);
        }
        if let Some(group) = groups.last_mut() {
            group.push((PartRef(part.name), part.operation));
        }
    }
    groups
}
