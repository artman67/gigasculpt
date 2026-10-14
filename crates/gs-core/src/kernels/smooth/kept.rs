//! What a Smooth dab's rounds keep from round to round (decision 0036, section 2, amended): the
//! neighbour lists of a part's points, gathered once a dab, and the stronger mode's means in a
//! later round, found in one pass. [`super::smooth`] reads the lists in the first round;
//! [`uniform_kept`] and [`stronger_kept`] read them in the later ones.

use super::{Around, Scratch, chosen, mean};
use crate::config::VertId;
use crate::kernel::SculptSurface;
use crate::kernels::Frame;
use crate::math::Vec3;
use crate::view::{NodeView, PosRead, edge_neighbours};

use std::ops::Range;

impl Scratch {
    /// A scratch for a dab's rounds (decision 0036, section 2, amended) over the slots `slots`
    /// of `node`: it keeps their neighbour lists from round to round.
    pub(in crate::kernels) fn keeping(node: &NodeView<'_>, slots: Range<u32>) -> Self {
        let mut s = Self::new(node, slots);
        let around = &mut s.around;
        around.kept = Some(Kept {
            at: vec![None; around.slots],
            ids: Vec::new(),
            outer: Vec::new(),
        });
        s
    }

    /// The stronger mode's means for a later round of a dab over the part of `node` this scratch
    /// keeps, into `flat`: the mean around every point whose list the first round gathered,
    /// read from this round's positions in one pass. [`stronger_kept`] reads them.
    pub(in crate::kernels) fn find_means(
        &self,
        s: SculptSurface<'_>,
        node: &NodeView<'_>,
        flat: &mut Flat,
    ) {
        let Around {
            base, slots, kept, ..
        } = &self.around;
        let Some(Kept { at, ids, outer }) = kept else {
            return;
        };
        if flat.at.is_empty() {
            // Once a dab, after the first round: the lists stay, and no later round adds any.
            flat.at = (ids.iter())
                .map(|&u| match u.wrapping_sub(*base) as usize {
                    l if l < *slots => l as u32,
                    _ => outer
                        .binary_search_by_key(&u, |o| o.0)
                        .map_or(u32::MAX, |i| (*slots + i) as u32),
                })
                .collect();
        }
        let (pos, own) = (s.pos(), node.pos);
        // A point of the same node is read straight from its positions (ids below wrap far
        // above), the same values as through the reader.
        let read = |u: VertId| match own.get(u.wrapping_sub(node.base) as usize) {
            Some(x) => *x,
            None => pos.get(u),
        };
        let lists = (at.iter().enumerate())
            .map(|(l, r)| (*base + l as u32, *r))
            .chain(outer.iter().map(|o| (o.0, Some(o.1))));
        flat.means.clear();
        flat.means.extend(lists.map(|(u, r)| {
            r.and_then(|(a, b)| mean(ids[a as usize..b as usize].iter().copied(), read))
                .unwrap_or(read(u))
        }));
    }

    /// Starts a round: the stronger mode's means read this round's positions.
    pub(in crate::kernels) fn round(&mut self) {
        self.around.means.clear();
        for o in self.around.kept.iter_mut().flat_map(|k| &mut k.outer) {
            o.2 = None;
        }
    }
}

/// The chosen neighbours of a part's own vertices, and for the stronger mode of the other parts'
/// vertices whose means it reads, each gathered the first time it is needed and kept for the
/// rounds of one dab, within which the topology and the visible faces stay.
pub(super) struct Kept {
    /// Per slot, its list's range in `ids` once gathered.
    pub(super) at: Vec<Option<(u32, u32)>>,
    pub(super) ids: Vec<VertId>,
    /// The stronger mode: the vertices of other parts whose means the part reads, by id, with
    /// their lists' ranges in `ids` and their means in the round once found.
    pub(super) outer: Vec<(VertId, (u32, u32), Option<Vec3>)>,
}

/// The stronger mode's means in a later round of a dab, the part's own slots first, then the
/// other parts' points of `outer` in order. A rounds' part keeps it beside its scratch, which
/// every kernel's slot loop builds, so that loop stays as it was.
#[derive(Default)]
pub(in crate::kernels) struct Flat {
    /// Per entry of the kept `ids`, its point's place in `means`: `u32::MAX` for a point whose
    /// mean the part does not read.
    at: Vec<u32>,
    means: Vec<Vec3>,
}

impl Kept {
    /// The range in `ids` of the chosen neighbours of `v`, the part's slot `l`, gathered into
    /// `ids` (with `nb` as scratch) the first time.
    pub(super) fn list(
        &mut self,
        s: SculptSurface<'_>,
        v: VertId,
        l: usize,
        nb: &mut Vec<(VertId, u8)>,
    ) -> (usize, usize) {
        let Kept { at, ids, .. } = self;
        let (a, b) = *at[l].get_or_insert_with(|| {
            let (topo, visible) = (s.topo(), s.visible());
            edge_neighbours(&topo, v, |f| visible.face(f), nb);
            let a = ids.len() as u32;
            ids.extend(chosen(nb));
            (a, ids.len() as u32)
        });
        (a as usize, b as usize)
    }

    /// The index in `outer` of `u`, a vertex of another part, its chosen neighbours gathered
    /// into `ids` (with `nb` as scratch) the first time.
    pub(super) fn outer_list(
        &mut self,
        s: SculptSurface<'_>,
        u: VertId,
        nb: &mut Vec<(VertId, u8)>,
    ) -> usize {
        let Kept { ids, outer, .. } = self;
        match outer.binary_search_by_key(&u, |o| o.0) {
            Ok(i) => i,
            Err(i) => {
                let (topo, visible) = (s.topo(), s.visible());
                edge_neighbours(&topo, u, |f| visible.face(f), nb);
                let a = ids.len() as u32;
                ids.extend(chosen(nb));
                outer.insert(i, (u, (a, ids.len() as u32), None));
                i
            }
        }
    }
}

/// [`super::smooth`] for Uniform in a later round of a dab, reading `v`'s neighbours from the
/// lists `nb` kept since the first round, which gathered the list of every slot a later round
/// visits. `node` is `v`'s node, whose positions it reads straight. `gain` is the spec's.
pub(in crate::kernels) fn uniform_kept(
    gain: f32,
    s: SculptSurface<'_>,
    f: &Frame,
    nb: &Scratch,
    node: &NodeView<'_>,
    (v, p, w): (VertId, Vec3, f32),
) -> Vec3 {
    let around = &nb.around;
    let l = v.wrapping_sub(around.base) as usize;
    let list = (around.kept.as_ref())
        .and_then(|kept| kept.at[l].map(|(a, b)| &kept.ids[a as usize..b as usize]))
        .unwrap_or_default();
    let (pos, own) = (s.pos(), node.pos);
    // A neighbour of another node lands past the node's slots (ids below wrap far above).
    let at = |u: VertId| match own.get(u.wrapping_sub(node.base) as usize) {
        Some(x) => *x,
        None => pos.get(u),
    };
    let raw = mean(list.iter().copied(), at).map(|t| t - p);
    raw.unwrap_or(Vec3::ZERO) * (1.0 - libm::expf(-gain * w * f.amount))
}

/// [`super::smooth`] for Stronger in a later round of a dab: the mean of the means around `v`'s
/// kept neighbours, which [`Scratch::find_means`] found for the round into `flat`, in the
/// list's order. `gain` is the spec's.
pub(in crate::kernels) fn stronger_kept(
    gain: f32,
    f: &Frame,
    (nb, flat): (&Scratch, &Flat),
    (v, p, w): (VertId, Vec3, f32),
) -> Vec3 {
    let around = &nb.around;
    let l = v.wrapping_sub(around.base) as usize;
    let raw = around.kept.as_ref().and_then(|kept| {
        let (a, b) = kept.at[l]?;
        let (at, means) = (&flat.at, &flat.means);
        mean(at[a as usize..b as usize].iter().copied(), |i| {
            means[i as usize]
        })
    });
    raw.map_or(Vec3::ZERO, |t| t - p) * (1.0 - libm::expf(-gain * w * f.amount))
}
