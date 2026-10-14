//! Reading the input before repair: split it into triangles, find its solids (each part's own,
//! capped or left to the generalized winding number), measure the band and sign every sample.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::ops::Range;

use gs_protocol::PlainMesh;

use super::fold::{self, Run};
use super::grid::{self, Field, Grid};
use super::gwn::Gwn;
use super::input::{self, Soup};
use super::rows::Rows;
use super::sign::{self, Sign, Source};
use super::{CellSize, Kept, RemeshError, SignMethod, Work, band, cap};
use crate::operation::Operation;

/// The input, measured and signed, before repair.
pub(super) struct Prepared {
    pub(super) soup: Soup,
    /// How many of the soup's triangles cap holes (they come last).
    pub(super) caps: usize,
    pub(super) field: Field,
    pub(super) kept: Kept,
    pub(super) sign: Sign,
    pub(super) method: SignMethod,
    pub(super) work: Work,
    /// The input was turned right way out ([`super::repair_operand`]).
    pub(super) turned: bool,
}

/// One solid, signed on its own: the ranges of the soup's triangles that make it (its parts'
/// own, then the caps over its holes), how its sign is read, and its run of the fold.
pub(super) struct Solid {
    pub(super) tris: Vec<Range<usize>>,
    pub(super) method: SignMethod,
    pub(super) run: usize,
}

/// Each part's range of the triangles: `parts` counts each part's faces, in order, and `face`
/// is each triangle's face.
fn part_ranges(face: &[u32], parts: &[(usize, Operation)]) -> Vec<Range<usize>> {
    let cut = |faces: usize| face.partition_point(|&f| (f as usize) < faces);
    let mut end = 0;
    (parts.iter())
        .map(|&(n, _)| {
            let start = cut(end);
            end += n;
            start..cut(end)
        })
        .collect()
}

/// The solids to sign one by one, as ranges of `tris`, each with its run: each part's
/// triangles (`parts`, from [`part_ranges`]; `run`, each part's run of the fold), with parts of
/// the same run that share a point taken together, in the order of their first part. Parts with
/// no triangles are left out. `weld` (`crate::weld::exact`) is asked for only with more than
/// one part.
fn solid_ranges<'a>(
    tris: &[[u32; 3]],
    parts: Vec<Range<usize>>,
    run: &[usize],
    weld: impl FnOnce() -> &'a [u32],
) -> Vec<(usize, Vec<Range<usize>>)> {
    if parts.len() < 2 {
        return vec![(0, parts)];
    }
    let weld = weld();
    // Each part's set, named by its first part. There are fewer joins than parts, and parts are
    // few, so a join renames the later set everywhere.
    let mut set: Vec<usize> = (0..parts.len()).collect();
    // The first part of the latest run seen using each welded point.
    let mut owner = vec![usize::MAX; weld.len()];
    for (p, range) in parts.iter().enumerate() {
        for &v in tris[range.clone()].iter().flatten() {
            let w = weld[v as usize] as usize;
            if owner[w] == usize::MAX || run[owner[w]] != run[p] {
                owner[w] = p;
                continue;
            }
            let (a, b) = (set[owner[w]], set[p]);
            if a != b {
                let (keep, gone) = (a.min(b), a.max(b));
                for s in set.iter_mut().filter(|s| **s == gone) {
                    *s = keep;
                }
            }
        }
    }
    let mut index = vec![usize::MAX; parts.len()];
    let mut out: Vec<(usize, Vec<Range<usize>>)> = Vec::new();
    for (p, (range, &s)) in parts.into_iter().zip(&set).enumerate() {
        if range.is_empty() {
            continue;
        }
        if index[s] == usize::MAX {
            index[s] = out.len();
            out.push((run[p], Vec::new()));
        }
        out[index[s]].1.push(range);
    }
    out
}

/// The solids (see [`solid_ranges`]), each capped or left to the generalized winding number,
/// and the caps over their holes, which come after every input triangle.
fn solids(
    mesh: &PlainMesh,
    tris: &[[u32; 3]],
    parts: Vec<Range<usize>>,
    run: &[usize],
    work: &mut Work,
) -> (Vec<Solid>, Vec<[u32; 3]>) {
    // Only a mesh not closed as numbered, or one of several parts, needs welding.
    let welded = OnceCell::new();
    let weld = || {
        welded
            .get_or_init(|| crate::weld::exact(&mesh.positions))
            .as_slice()
    };
    let mut solids = Vec::new();
    let mut caps = Vec::new();
    for (run, mut ranges) in solid_ranges(tris, parts, run, weld) {
        let own: Cow<[[u32; 3]]> = match ranges.as_slice() {
            [one] => Cow::Borrowed(&tris[one.clone()]),
            _ => ranges
                .iter()
                .flat_map(|r| tris[r.clone()].to_vec())
                .collect(),
        };
        let method = match cap::classify(&own, weld) {
            cap::Edges::Closed => SignMethod::Winding,
            cap::Edges::Holes { caps: more, loops } => {
                work.holes += loops;
                let at = tris.len() + caps.len();
                ranges.push(at..at + more.len());
                caps.extend(more);
                SignMethod::CappedWinding
            }
            cap::Edges::Messy => SignMethod::GeneralizedWinding,
        };
        solids.push(Solid {
            tris: ranges,
            method,
            run,
        });
    }
    (solids, caps)
}

/// What the sign is read from: the rows of the closed and capped solids, the generalized
/// winding number of the others (each triangle turned over if `turned`), and the runs (`ops`,
/// each run's operation; `never`, each run's [`fold::never`]).
fn source(
    soup: &Soup,
    solids: &[Solid],
    (ops, never): (&[Operation], Vec<bool>),
    turned: bool,
    samples: [u32; 3],
    work: &mut Work,
) -> Source {
    let messy = |s: &&Solid| s.method == SignMethod::GeneralizedWinding;
    let corners = |t: &[u32; 3]| {
        let t = if turned { [t[0], t[2], t[1]] } else { *t };
        t.map(|v| soup.verts[v as usize].map(f64::from))
    };
    let gwn = (solids.iter().filter(messy))
        .map(|s| {
            let ranges = s.tris.iter().cloned();
            Gwn::new(
                ranges
                    .flat_map(|r| soup.tris[r].iter().map(corners))
                    .collect(),
            )
        })
        .collect();
    let counted: Vec<&Solid> = solids.iter().filter(|s| !messy(s)).collect();
    let ranges: Vec<&[Range<usize>]> = counted.iter().map(|s| s.tris.as_slice()).collect();
    let tags: Vec<u32> = counted.iter().map(|s| s.run as u32).collect();
    let all: Vec<bool> = ops.iter().map(|&op| op == Operation::Intersect).collect();
    let rows =
        (!counted.is_empty()).then(|| Rows::new(soup, &ranges, (&tags, &all), samples, work));
    let mut next = 0;
    let runs = (ops.iter().zip(never).enumerate())
        .map(|(k, (&op, never))| {
            let start = next;
            next += solids.iter().filter(|s| s.run == k && messy(s)).count();
            let rows = counted.iter().any(|s| s.run == k);
            Run {
                op,
                rows,
                gwn: start..next,
                never,
            }
        })
        .collect();
    Source { rows, gwn, runs }
}

/// Reads, measures and signs the input: `operands` counts each operand's faces, in order, with
/// its operation. With `turn`, a solid whose capped volume is negative is turned right way out.
pub(super) fn prepare(
    mesh: &PlainMesh,
    operands: &[(usize, Operation)],
    cell: CellSize,
    turn: bool,
) -> Result<Prepared, RemeshError> {
    let (mut tris, mut face) = input::triangles(mesh)?;
    let sum = operands
        .iter()
        .try_fold(0usize, |sum, o| sum.checked_add(o.0));
    if sum != Some(mesh.faces.len()) {
        return Err(RemeshError::BadParts);
    }
    let (lo, hi) = input::bounds(mesh);
    let (grid, kept) = Grid::new(lo, hi, cell)?;
    let mut work = Work::default();
    let (run, ops) = fold::runs(operands);
    let parts = part_ranges(&face, operands);
    let never = fold::never(&parts, &run, &ops);
    let (solids, caps) = solids(mesh, &tris, parts, &run, &mut work);
    let method = (solids.iter().map(|s| s.method).max()).unwrap_or(SignMethod::Winding);
    let capped = caps.len();
    // Caps come from no input face.
    face.extend(std::iter::repeat_n(grid::NONE, capped));
    tris.extend(caps);
    let centre = std::array::from_fn(|a| f64::midpoint(lo[a], hi[a]));
    let turned = turn && cap::volume(&mesh.positions, &tris, centre) < 0.0;
    let soup = Soup::new(mesh, tris, face, &grid);
    let mut field = Field::new(grid);
    let of = if ops.len() > 1 {
        fold::triangle_runs(&solids, soup.tris.len())
    } else {
        Vec::new()
    };
    band::measure(&soup, &of, ops.len(), &mut field, &mut work);
    let source = source(
        &soup,
        &solids,
        (&ops, never),
        turned,
        grid.samples(),
        &mut work,
    );
    let sign = Sign::new(source, grid.samples());
    let band = (0..field.bricks.len()).collect();
    sign::settle(&mut field, &sign, band, &mut work);
    Ok(Prepared {
        soup,
        caps: capped,
        field,
        kept,
        sign,
        method,
        work,
        turned,
    })
}
