//! The part list (decision 0015): every part has a stable id, handed out by the document's
//! counter when the part first enters it and never reused. The list is in rank order, then id
//! order (decision 0028): a new part gets the largest rank and so goes last, and moving a part
//! changes ranks. A part reference is `#12` (an id), `@0` (a place in the list, from 0),
//! `@active` (the active part, decision 0033) or a name. Ids and ranks travel with the part in its `Props`, so shares, checkpoints, undo and
//! jobs keep them, and only logged calls move the counter, so a replay hands out the same ids.
//!
//! The list edits the part commands build on are here too: a new order, a copy, a rename and a
//! part put in from plain mesh data. Each swaps whole parts (decision 0010), so undo needs
//! nothing more; new properties for a part (`DocTxn::set_props`) change it in place, and the
//! step keeps the old ones (decision 0029, section 8).

use std::collections::BTreeSet;

use gs_protocol::{CommandError, PlainMesh};

use super::{DocTxn, Document, Isolation, Part, Props, VertexOrder};
use crate::Unavailable;

impl Document {
    /// Every part in list order.
    pub(crate) fn list(&self) -> Vec<(&str, &Part)> {
        let mut all: Vec<(&str, &Part)> = (self.parts.iter())
            .map(|(name, part)| (name.as_str(), &**part))
            .collect();
        all.sort_by_key(|(_, part)| (part.props.rank, part.props.id));
        all
    }

    /// The name of the part `r` refers to: `#12` by id, `@0` by place in the list, `@active`
    /// the active part, otherwise by name. `None` if there is no such part.
    pub(crate) fn resolve(&self, r: &str) -> Option<&str> {
        if r == "@active" {
            return self.active();
        }
        if let Some(id) = r.strip_prefix('#').and_then(|n| n.parse::<u64>().ok()) {
            return (self.parts.iter())
                .find(|(_, part)| part.props.id == Some(id))
                .map(|(name, _)| name.as_str());
        }
        if let Some(at) = r.strip_prefix('@').and_then(|n| n.parse::<usize>().ok()) {
            return self.list().get(at).map(|(name, _)| *name);
        }
        self.parts.get_key_value(r).map(|(name, _)| name.as_str())
    }

    /// A part's stable id and place in the list.
    pub(crate) fn place(&self, name: &str) -> Option<(u64, usize)> {
        let list = self.list();
        let at = list.iter().position(|(n, _)| *n == name)?;
        Some((list[at].1.props.id?, at))
    }

    /// How many part ids the document has handed out.
    pub(crate) fn issued(&self) -> u64 {
        self.issued
    }

    /// Takes the id counter of `copy`, a job's copy of this document that landed here, the
    /// Isolate marks of the parts it made (a duplicate takes its source's) and, when it made the
    /// active part, that (decision 0033). Then keeps the active part as it is, as a call's end
    /// does.
    pub(crate) fn take_ids(&mut self, copy: &Document) {
        let new = |id: &&u64| **id > self.issued;
        let chosen: Vec<u64> = copy.isolation.chosen.iter().filter(new).copied().collect();
        let aside: Vec<u64> = copy.isolation.aside.iter().filter(new).copied().collect();
        self.isolation.chosen.extend(chosen);
        self.isolation.aside.extend(aside);
        let landed = |id: &u64| self.parts.values().any(|p| p.props.id == Some(*id));
        if let Some(made) = copy.active.filter(|id| *id > self.issued && landed(id)) {
            self.set_active(Some(made));
        }
        self.issued = self.issued.max(copy.issued);
        self.settle();
    }

    /// The isolation in effect (decision 0029, section 9): what Isolate chose while a part it
    /// chose is in the document, else none, so deleting every chosen part shows the rest by
    /// their eyes. Every reader of what shows asks [`Part::shown`] with it and the active part
    /// (see [`Document::showing`]).
    pub(crate) fn isolation(&self) -> &Isolation {
        static OFF: Isolation = Isolation {
            chosen: BTreeSet::new(),
            aside: BTreeSet::new(),
        };
        let chosen = |id: u64| self.isolation.chosen.contains(&id);
        if self
            .parts()
            .any(|(_, part)| part.props.id.is_some_and(chosen))
        {
            &self.isolation
        } else {
            &OFF
        }
    }

    /// Every part that shows, by name.
    pub(crate) fn shown(&self) -> impl Iterator<Item = (&String, &Part)> {
        let showing = self.showing();
        self.parts().filter(move |(_, part)| part.shown(showing))
    }

    /// Puts what a scene file's session section says Isolate chose (decision 0029, section 9).
    pub(crate) fn set_isolation(&mut self, isolation: Isolation) {
        self.isolation = isolation;
    }
}

impl DocTxn<'_> {
    /// Every part's name, in list order. Reads the whole list.
    pub(crate) fn list_names(&self) -> Vec<String> {
        let list = self.document().list();
        list.into_iter().map(|(name, _)| name.to_string()).collect()
    }

    /// Puts the parts in the order `names` gives (every part, once), changing the ranks of only
    /// the parts that must move (decision 0028, section 1): walking the order, a part keeps its
    /// rank while it still sorts after the part before it, else takes that part's rank (when its
    /// id is larger) or one more.
    pub(crate) fn arrange(&mut self, names: &[String]) -> Result<(), CommandError> {
        let mut last: Option<(u64, u64)> = None;
        for name in names {
            let mut props = self.get_part(name)?.props;
            let id = props.id.unwrap_or_default();
            if let Some((rank, before)) = last
                && (props.rank, id) < (rank, before)
            {
                props.rank = if id > before { rank } else { rank + 1 };
                self.set_props(name, props)?;
            }
            last = Some((props.rank, id));
        }
        Ok(())
    }

    /// Adds a copy of part `from` named `to`: a share of every level, column and property but
    /// its id, so it gets a new one and goes last, with its Isolate mark.
    pub(crate) fn copy_part(&mut self, from: &str, to: &str) -> Result<(), CommandError> {
        let source = self.get_part(from)?;
        let id = source.props.id;
        let copy = source.share_with(Props {
            id: None,
            ..source.props
        });
        self.replace_part(to, copy);
        let copied = self.get_part(to)?.props.id;
        if let Some((from, to)) = id.zip(copied) {
            self.isolate(|isolation| isolation.copy(from, to));
        }
        Ok(())
    }

    /// Gives part `from` the name `to`, keeping everything else, its id and place included.
    pub(crate) fn rename_part(&mut self, from: &str, to: &str) -> Result<(), CommandError> {
        let part = self.get_part(from)?.share();
        self.remove_part(from);
        self.replace_part(to, part);
        Ok(())
    }

    /// Puts a part made from plain mesh data under `name`, remembering `props`: with an id it
    /// keeps that part's id and rank, with none it gets a new one and goes last. Refuses meshes
    /// the store cannot hold, as [`DocTxn::insert_part`] does.
    pub(crate) fn put_mesh(
        &mut self,
        name: &str,
        mesh: PlainMesh,
        order: VertexOrder,
        props: Props,
    ) -> Result<(), CommandError> {
        let mut part = Part::new(mesh, order, name)?;
        part.props = props;
        self.replace_part(name, part);
        Ok(())
    }

    /// The name a new part gets from `base`, a name nobody typed for it (a command's default,
    /// a file's object or the file itself): `base` made fit (see [`fit_name`]), or that with
    /// -2, -3 ... added while it is taken. One of the first n + 1 is free: n names are taken.
    pub(crate) fn free_name(&self, base: &str) -> String {
        let base = fit_name(base);
        let n = self.part_names().count();
        let numbered = (2..=n + 1).map(|k| format!("{base}-{k}"));
        std::iter::once(base.clone())
            .chain(numbered)
            .find(|name| !self.part_names().any(|p| p == name))
            .unwrap_or(base)
    }
}

/// The naming rule every way a part gets a name goes through (review 0003 M1): no part is
/// named like a reference, `#2` or `@0`, which would send every command that names it to
/// another part. Says why `name` is refused, if it is. A name the caller chose is refused with
/// this; a name taken from a file is made fit instead ([`fit_name`]).
pub(crate) fn unfit(name: &str) -> Option<Unavailable> {
    reference_like(name).then(|| {
        Unavailable::new(format!(
            "'{name}' would read as a part id or a place in the part list"
        ))
        .fix("choose a name that does not start with # or @")
    })
}

/// `name`, or `part <name>` if it is unfit (see [`unfit`]).
fn fit_name(name: &str) -> String {
    if unfit(name).is_some() {
        format!("part {name}")
    } else {
        name.to_string()
    }
}

/// Whether `name` could be read as an id or a place in the list.
fn reference_like(name: &str) -> bool {
    name.starts_with('#') || name.starts_with('@')
}
