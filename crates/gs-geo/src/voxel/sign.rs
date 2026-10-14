//! Signs: which samples are inside. Applies them to the measured band, fills in the sign of
//! every uniform brick next to an allocated one, and allocates any uniform brick that a sign
//! change runs into (a hole's film, for open input), until every boundary sample is allocated.
//!
//! **The grid's outer layer is outside.** A sample with a coordinate of 0, or one less than the
//! samples along its axis, is outside whatever the winding says, so the surface never reaches
//! the grid's edge and every sign change has the four cells around it that Surface Nets joins
//! (a cell at -1 does not exist). Closed and capped input never comes near: its surface is
//! inside the bounding box, [`super::PAD`] voxels in. The generalized winding number of messy
//! input can be inside far from it: a sheet doubled over itself winds about twice as much as
//! one sheet, so a lens on its side counts as inside, reaching a good part of the sheet's width
//! away, and the edge cuts it off one sample in.
//! No repair ever moves an outer sample inside: a critical configuration needs a sample of each
//! sign on every face of its cube, and a cube touching the edge has a face all outside.
//!
//! So a brick on the grid's edge is never uniformly inside. Its sign is read one sample in from
//! the edge, and if that is inside, the brick is allocated and signed sample by sample, which
//! carries the cut along the edge to every brick the inside region reaches.
//! For mesh input the film rule in [`settle`] allocates the same bricks a round later: the
//! mesh's lowest point is in the first layer of bricks, so the band always reaches the edge,
//! and every allocated edge brick has outer samples, outside, on the faces it shares with
//! other edge bricks. This rule is what holds when no allocated brick touches the edge; only a
//! hand-made winding gets there ([`settled_inside`], for gs-geo's tests).

use super::Work;
use super::fold::{self, Run};
use super::grid::{B, BRICK, BRICK_BITS, Field, Grid, sample};
use super::gwn::Gwn;
use super::rows::Rows;
use crate::math::{morton, unmorton};
use crate::operation::Operation;

/// What the winding is read from. A sample is inside a run (`super::fold`) when it is inside
/// any of its solids (see `super::solid_ranges`), or all for a run of intersects, and inside
/// when the runs fold to inside. A remesh of parts is one run of unions.
pub(crate) struct Source {
    /// Exact row winding of the solids that are closed or capped, joined into one count for
    /// each run.
    pub(crate) rows: Option<Rows>,
    /// The generalized winding number of each other solid, run after run. Built in grid units.
    pub(crate) gwn: Vec<Gwn>,
    /// The runs, in order.
    pub(crate) runs: Vec<Run>,
}

/// Which samples are inside: the winding's, except on the grid's outer layer.
pub(crate) struct Sign {
    source: Source,
    /// Samples along each axis.
    samples: [u32; 3],
}

impl Sign {
    pub(crate) fn new(source: Source, samples: [u32; 3]) -> Self {
        Self { source, samples }
    }

    /// The runs of the fold.
    pub(crate) fn runs(&self) -> &[Run] {
        &self.source.runs
    }

    /// True if sample `p` is on the grid's outer layer.
    pub(crate) fn outer(&self, p: [u32; 3]) -> bool {
        (0..3).any(|a| p[a] == 0 || p[a] + 1 >= self.samples[a])
    }

    /// True if sample `p` is inside, and the winding terms summed to decide (none for rows or
    /// the outer layer).
    pub(crate) fn inside(&self, p: [u32; 3]) -> (bool, u64) {
        if self.source.runs.len() > 1 {
            return if self.outer(p) {
                (false, 0)
            } else {
                self.folded(p)
            };
        }
        self.unfolded(p)
    }

    /// [`Self::inside`] for one run, a union (every remesh but a fold of several): inside any
    /// solid, with no runs to go through.
    fn unfolded(&self, p: [u32; 3]) -> (bool, u64) {
        if self.outer(p) {
            return (false, 0);
        }
        if self.source.rows.as_ref().is_some_and(|r| r.winding(p) != 0) {
            return (true, 0);
        }
        any(&self.source.gwn, p)
    }

    /// [`Self::inside`] for several runs, off the outer layer: the runs folded, each only read
    /// where it can change the answer.
    fn folded(&self, p: [u32; 3]) -> (bool, u64) {
        let (mut inside, mut terms) = (false, 0);
        for (k, run) in self.source.runs.iter().enumerate() {
            if fold::matters(run.op, inside) {
                let (within, t) = self.within(p, k);
                terms += t;
                inside = fold::after(run.op, within);
            }
        }
        (inside, terms)
    }

    /// True if sample `p` is inside run `k`, whatever the outer layer, and the winding terms
    /// summed to decide.
    pub(crate) fn within(&self, p: [u32; 3], k: usize) -> (bool, u64) {
        let run = &self.source.runs[k];
        let rows =
            run.rows && (self.source.rows.as_ref()).is_some_and(|r| r.winding_of(p, k as u32) != 0);
        let gwn = &self.source.gwn[run.gwn.clone()];
        if run.op == Operation::Intersect {
            if run.never || (run.rows && !rows) {
                return (false, 0);
            }
            let mut terms = 0;
            for gwn in gwn {
                let (inside, t) = gwn.inside(p.map(f64::from));
                terms += t;
                if !inside {
                    return (false, terms);
                }
            }
            return (true, terms);
        }
        if rows {
            return (true, 0);
        }
        any(gwn, p)
    }

    /// Which samples of the brick at `at` are inside, and the winding terms summed, for one run
    /// (`super::fold` signs the bricks of several).
    fn brick(&self, at: [u32; 3]) -> ([bool; BRICK], u64) {
        let mut terms = 0;
        let inside = std::array::from_fn(|i| {
            let (inside, t) = self.unfolded(sample(at, i));
            terms += t;
            inside
        });
        (inside, terms)
    }

    /// True if the brick at `at` holds part of the grid's outer layer.
    fn on_edge(&self, at: [u32; 3]) -> bool {
        (0..3).any(|a| at[a] == 0 || (at[a] + 1) * B >= self.samples[a])
    }

    /// The sample a uniform brick's sign is read from: its first, moved one in along each axis
    /// where that is on the outer layer.
    fn probe(at: [u32; 3]) -> [u32; 3] {
        sample(at, 0).map(|c| c.max(1))
    }
}

/// True if sample `p` is inside any of `gwn`'s solids, and the winding terms summed to decide.
fn any(gwn: &[Gwn], p: [u32; 3]) -> (bool, u64) {
    let mut terms = 0;
    for gwn in gwn {
        let (inside, t) = gwn.inside(p.map(f64::from));
        terms += t;
        if inside {
            return (true, terms);
        }
    }
    (false, terms)
}

/// Signs the samples of the given bricks.
fn apply(field: &mut Field, sign: &Sign, slots: &[usize], work: &mut Work) {
    if sign.runs().len() > 1 {
        return fold::apply(field, sign, slots, work);
    }
    let signs = gs_par::map(slots.len(), |k| sign.brick(field.bricks[slots[k]].at));
    for (&slot, (inside, terms)) in slots.iter().zip(signs) {
        work.sign_queries += BRICK as u64;
        work.winding_terms += terms;
        for (value, inside) in field.bricks[slot].value.iter_mut().zip(inside) {
            *value = if inside { -value.abs() } else { value.abs() };
        }
    }
}

/// The 26 offsets to the bricks around a brick.
pub(crate) fn around() -> impl Iterator<Item = [i32; 3]> {
    (0..27)
        .map(|k| [k % 3 - 1, k / 3 % 3 - 1, k / 9 - 1])
        .filter(|d| *d != [0, 0, 0])
}

/// Signs the allocated bricks in `fresh` and settles the uniform bricks around them. The
/// measured band starts it with every brick; a repair, with edge bricks it allocated.
pub(crate) fn settle(field: &mut Field, sign: &Sign, mut fresh: Vec<usize>, work: &mut Work) {
    // Each round signs the bricks allocated last round, then allocates the uniform bricks they
    // run into. Only bricks that were uniform count as new, so there are never more rounds than
    // bricks in the grid.
    for _ in 0..field.capacity() {
        if fresh.is_empty() {
            break;
        }
        apply(field, sign, &fresh, work);
        let mut next = resolve_around(field, sign, &fresh, work);
        let found = gs_par::map(fresh.len(), |k| mismatched(field, fresh[k]));
        let mut grow: Vec<u64> = found.concat();
        gs_par::sort_keys(&mut grow);
        grow.dedup();
        next.extend(grow.iter().filter_map(|&key| {
            let at = unmorton(key, BRICK_BITS);
            let uniform = field.slot(at).is_none();
            uniform.then(|| field.materialize(at))
        }));
        work.film_bricks += next.len() as u64;
        fresh = next;
    }
}

/// Settles the sign of every unknown brick around the given allocated bricks, from the sign
/// at one sample (a uniform brick has the same sign everywhere). An edge brick found inside is
/// not uniform: it is allocated, unsigned, and its slot returned for the caller to settle.
#[must_use]
pub(crate) fn resolve_around(
    field: &mut Field,
    sign: &Sign,
    slots: &[usize],
    work: &mut Work,
) -> Vec<usize> {
    let mut unknown = Vec::new();
    for &slot in slots {
        let at = field.bricks[slot].at;
        for d in around() {
            if let Some(n) = field.neighbor(at, d)
                && !field.known(n)
            {
                unknown.push(morton(n, BRICK_BITS));
            }
        }
    }
    gs_par::sort_keys(&mut unknown);
    unknown.dedup();
    let signs = gs_par::map(unknown.len(), |k| {
        let at = unmorton(unknown[k], BRICK_BITS);
        sign.inside(Sign::probe(at)).0
    });
    let mut edge = Vec::new();
    for (&key, inside) in unknown.iter().zip(signs) {
        work.uniform_queries += 1;
        let at = unmorton(key, BRICK_BITS);
        if inside && sign.on_edge(at) {
            edge.push(field.materialize(at));
        } else {
            field.set_uniform(at, inside);
        }
    }
    edge
}

/// Uniform bricks next to the allocated brick in `slot` whose sign differs from a sample of
/// this brick right across the face: a sign change runs into them, so they must be allocated.
fn mismatched(field: &Field, slot: usize) -> Vec<u64> {
    let brick = &field.bricks[slot];
    let mut out = Vec::new();
    // Each face: the axis, the step to the neighbour, and this brick's layer of samples on it.
    for a in 0..3 {
        for (step, layer) in [(-1, 0), (1, B - 1)] {
            let mut d = [0; 3];
            d[a] = step;
            let Some(nb) = field.neighbor(brick.at, d) else {
                continue;
            };
            if field.slot(nb).is_some() {
                continue;
            }
            let beyond = field.value(sample(nb, 0)).is_sign_negative();
            let differs = (0..BRICK).any(|i| {
                sample([0; 3], i)[a] == layer && brick.value[i].is_sign_negative() != beyond
            });
            if differs {
                out.push(morton(nb, BRICK_BITS));
            }
        }
    }
    out
}

/// Which samples of `grid` are inside once the signs settle, with the winding read from `tris`
/// (in grid units) and only brick `start` allocated to begin with; x fastest, then y, then z.
/// For tests only: it reaches the edge-brick rule, which no mesh input does (review 0008, T2).
#[doc(hidden)]
pub fn settled_inside(grid: Grid, tris: Vec<[[f64; 3]; 3]>, start: [u32; 3]) -> Vec<bool> {
    let n = grid.samples();
    let source = Source {
        rows: None,
        gwn: vec![Gwn::new(tris)],
        runs: vec![Run {
            op: Operation::Union,
            rows: false,
            gwn: 0..1,
            never: false,
        }],
    };
    let sign = Sign::new(source, n);
    let mut field = Field::new(grid);
    let first = field.materialize(start);
    settle(&mut field, &sign, vec![first], &mut Work::default());
    let [nx, ny, nz] = n;
    (0..nz)
        .flat_map(|z| (0..ny).flat_map(move |y| (0..nx).map(move |x| [x, y, z])))
        .map(|p| field.value(p) < 0.0)
        .collect()
}
