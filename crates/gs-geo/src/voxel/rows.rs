//! Sign for closed input: the exact winding number along every grid row.
//!
//! Every row of samples along x starts outside at the grid's edge. Each place a triangle
//! crosses the row adds the sign of the triangle's facing (+1 or -1), so the running sum is the
//! winding number, and a sample is inside when its sum is not zero. That is a flood fill from
//! the boundary along each row, with walls found exactly: vertices are snapped to a fixed-point
//! grid of 1/65536 voxel and every crossing test is integer arithmetic. A row through a shared
//! edge or vertex is counted once, by a consistent tie rule (as if the row were nudged by an
//! infinitesimal amount in a fixed direction). So on a closed, consistently wound mesh the sum
//! is exact even where the mesh intersects itself: overlapping volumes count 2 and stay
//! inside, and a cavity wound inward counts 0 and stays hollow.
//!
//! Several solids (the parts of a `remesh_parts`, see `super::solid_ranges`) are counted apart:
//! each crossing knows its solid, and each row is then rewritten so its sum is 1 where any
//! solid's own count is not zero and 0 elsewhere. So solids never cancel: an inside-out part
//! counts -1 inside, and where it overlaps a part facing out the overlap stays inside, while a
//! cavity wound inward inside one part still counts 0 there. With several runs to fold
//! (`super::fold`), each run gets a count of its own this way, 1 where any of its solids is
//! inside, or for a run of intersects where all are, and the crossings say whose they are.

use std::ops::Range;

use super::Work;
use super::input::Soup;

/// Fraction bits of the fixed-point coordinates.
const FIX: u32 = 16;
/// Triangles per parallel task.
const CHUNK: usize = 4096;
/// Rows per parallel task when solids are joined.
const ROWS: usize = 1024;

/// Where a triangle crosses a row: the row, the crossing's x in fixed point, the facing, and
/// the solid the triangle belongs to (once rewritten for several solids, its run).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Crossing {
    row: u32,
    x: u32,
    facing: i8,
    solid: u32,
}

/// Every crossing, grouped by row and sorted along it.
pub(crate) struct Rows {
    /// Samples along y, to number the rows.
    ny: u32,
    /// `crossings[start[r]..start[r + 1]]` are the crossings of row `r`.
    start: Vec<u32>,
    crossings: Vec<Crossing>,
    /// The solids are of more than one run, so each crossing is counted for its own run only.
    tagged: bool,
}

impl Rows {
    /// Finds every crossing with the rows of a grid with `samples` per axis, of each solid's
    /// triangles: `solids[s]` are ranges of the soup's triangles, `runs[s]` the solid's run
    /// (in order), and `all[r]` says whether run `r` is a run of intersects.
    pub(crate) fn new(
        soup: &Soup,
        solids: &[&[Range<usize>]],
        (runs, all): (&[u32], &[bool]),
        samples: [u32; 3],
        work: &mut Work,
    ) -> Self {
        let fixed = gs_par::map(soup.verts.len(), |v| {
            soup.verts[v].map(|c| (f64::from(c) * f64::from(1u32 << FIX)).round() as i64)
        });
        let ny = samples[1];
        let mut tasks = Vec::new();
        for (s, ranges) in solids.iter().enumerate() {
            for r in *ranges {
                tasks.extend(
                    r.clone()
                        .step_by(CHUNK)
                        .map(|t| (s as u32, t..r.end.min(t + CHUNK))),
                );
            }
        }
        let lists = gs_par::map(tasks.len(), |k| {
            let (solid, range) = (tasks[k].0, tasks[k].1.clone());
            let mut found = Vec::new();
            let mut tests = 0;
            for t in &soup.tris[range] {
                let t = t.map(|v| fixed[v as usize]);
                tests += crossings(t, [ny, samples[2]], solid, &mut found);
            }
            (found, tests)
        });
        let mut crossings = Vec::new();
        for (found, tests) in lists {
            crossings.extend(found);
            work.crossing_tests += tests;
        }
        gs_par::sort(&mut crossings);
        let rows = (ny * samples[2]) as usize;
        let mut start = starts(&crossings, rows);
        if solids.len() > 1 {
            crossings = per_run(&crossings, &start, runs, all);
            start = starts(&crossings, rows);
        }
        Self {
            ny,
            start,
            crossings,
            tagged: runs.windows(2).any(|w| w[0] != w[1]),
        }
    }

    /// The winding number at sample `p`: the sum of the facings of every crossing at or
    /// before it along its row, every run's together.
    pub(crate) fn winding(&self, p: [u32; 3]) -> i32 {
        self.before(p).map(|c| i32::from(c.facing)).sum()
    }

    /// The winding number of run `run` at sample `p`: [`Self::winding`] of the run's crossings
    /// only (every crossing, when all are one run's). It does not call `winding`: with a second
    /// caller, `winding` stopped being inlined into the sign of one run, which cost about 0.5%
    /// of a whole voxel remesh (perf scenario `voxel-closed`).
    pub(crate) fn winding_of(&self, p: [u32; 3], run: u32) -> i32 {
        (self.before(p).filter(|c| !self.tagged || c.solid == run))
            .map(|c| i32::from(c.facing))
            .sum()
    }

    /// The crossings at or before sample `p` along its row.
    fn before(&self, p: [u32; 3]) -> impl Iterator<Item = &Crossing> {
        let row = (p[2] * self.ny + p[1]) as usize;
        let span = self.start[row] as usize..self.start[row + 1] as usize;
        let limit = p[0] << FIX;
        self.crossings[span]
            .iter()
            .take_while(move |c| c.x <= limit)
    }
}

/// Where each of `rows` rows starts in `crossings` (sorted by row), and where the last ends.
fn starts(crossings: &[Crossing], rows: usize) -> Vec<u32> {
    let mut start = vec![0u32; rows + 1];
    for c in crossings {
        start[c.row as usize + 1] += 1;
    }
    for r in 1..start.len() {
        start[r] += start[r - 1];
    }
    start
}

/// Each row's crossings rewritten run by run (`runs[s]` is solid `s`'s run): one crossing of +1
/// where the run turns inside, one of -1 where it turns back, tagged with the run. A run is
/// inside where any of its solids' own count is not zero, or for a run of intersects (`all`)
/// where all are. For one run of unions, that is their union: +1 where the first solid's count
/// leaves zero, -1 where the last one's comes back to it. Crossings at the same x are taken
/// together, as the count at a sample takes them.
fn per_run(crossings: &[Crossing], start: &[u32], runs: &[u32], all: &[bool]) -> Vec<Crossing> {
    let rows = start.len() - 1;
    let mut size = vec![0usize; all.len()];
    for &r in runs {
        size[r as usize] += 1;
    }
    // Whether a run with `on` solids inside is inside.
    let inside = |run: u32, on: usize| on > 0 && (!all[run as usize] || on == size[run as usize]);
    let lists = gs_par::map(rows.div_ceil(ROWS), |c| {
        let mut out = Vec::new();
        // Each solid met so far on the row and its count; each run met, its solids whose count
        // is not zero, and whether it was inside before this x. A row meets few.
        let mut count: Vec<(u32, i32)> = Vec::new();
        let mut met: Vec<(u32, usize, bool)> = Vec::new();
        for r in c * ROWS..rows.min((c + 1) * ROWS) {
            count.clear();
            met.clear();
            let row = &crossings[start[r] as usize..start[r + 1] as usize];
            for same in row.chunk_by(|a, b| a.x == b.x) {
                for m in &mut met {
                    m.2 = inside(m.0, m.1);
                }
                for cross in same {
                    let k = (count.iter().position(|e| e.0 == cross.solid)).unwrap_or_else(|| {
                        count.push((cross.solid, 0));
                        count.len() - 1
                    });
                    let before = count[k].1 != 0;
                    count[k].1 += i32::from(cross.facing);
                    if before == (count[k].1 != 0) {
                        continue;
                    }
                    let run = runs[cross.solid as usize];
                    let j = (met.iter().position(|m| m.0 == run)).unwrap_or_else(|| {
                        met.push((run, 0, false));
                        met.len() - 1
                    });
                    met[j].1 = if before { met[j].1 - 1 } else { met[j].1 + 1 };
                }
                for &(run, on, was) in &met {
                    if was != inside(run, on) {
                        out.push(Crossing {
                            facing: if was { -1 } else { 1 },
                            solid: run,
                            ..same[0]
                        });
                    }
                }
            }
        }
        out
    });
    lists.concat()
}

/// Twice the signed area of the 2D triangle `u v p`.
fn orient(u: [i64; 2], v: [i64; 2], p: [i64; 2]) -> i64 {
    (v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0])
}

/// Which side of the directed edge `u -> v` the point `p` is on, as +1 or -1. A point exactly
/// on the edge's line takes the side the point moves to when nudged by `(e, e * d)` for
/// infinitesimals `0 < d << e`; the reversed edge then gets the other side, so a point on an
/// edge shared by two triangles belongs to exactly one of them.
fn side(u: [i64; 2], v: [i64; 2], p: [i64; 2]) -> i64 {
    let e = orient(u, v, p);
    if e != 0 {
        return e.signum();
    }
    let along_z = u[1] - v[1];
    if along_z != 0 {
        return along_z.signum();
    }
    (v[0] - u[0]).signum()
}

/// Adds the crossings of one triangle (corners in fixed point) of `solid` with the rows.
/// Returns how many row positions it tested.
fn crossings(t: [[i64; 3]; 3], rows: [u32; 2], solid: u32, found: &mut Vec<Crossing>) -> u64 {
    // Rows run along x, so the triangle is tested in the (y, z) plane.
    let [a, b, c] = t.map(|p| [p[1], p[2]]);
    let area = orient(a, b, c);
    if area == 0 {
        return 0;
    }
    let facing = area.signum();
    let ny = rows[0];
    let first = |k: usize| -((-a[k].min(b[k]).min(c[k])) >> FIX);
    // Never past the grid, whatever the corners say.
    let last = |k: usize| (a[k].max(b[k]).max(c[k]) >> FIX).min(i64::from(rows[k]));
    let mut tests = 0;
    for z in first(1)..=last(1) {
        for y in first(0)..=last(0) {
            tests += 1;
            let p = [y << FIX, z << FIX];
            let inside =
                side(b, c, p) == facing && side(c, a, p) == facing && side(a, b, p) == facing;
            if inside {
                // Barycentric weights from the exact edge functions give the crossing's x.
                let w = [orient(b, c, p), orient(c, a, p), orient(a, b, p)];
                let x = (0..3).map(|i| w[i] as f64 * t[i][0] as f64).sum::<f64>() / area as f64;
                found.push(Crossing {
                    row: (z as u32) * ny + y as u32,
                    x: x.floor() as u32,
                    facing: facing as i8,
                    solid,
                });
            }
        }
    }
    tests
}
