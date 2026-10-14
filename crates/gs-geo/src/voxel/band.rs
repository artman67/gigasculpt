//! The narrow band: the exact distance from every sample within [`REACH`] voxels of a triangle
//! to the nearest triangle, and which triangle that is.
//!
//! Triangles are binned into the bricks their reach overlaps (a sort of brick and triangle
//! pairs), then every brick works through its own list in parallel. A sample is only measured
//! against triangles whose reach box holds it. A big triangle is binned as pieces of at most
//! [`PIECE`] voxels a side, each with its own small reach box, so a large tilted triangle costs
//! its area rather than its bounding box's volume. The work grows with the surface area, never
//! with the volume. With several runs to fold (`super::fold`), each brick also keeps the
//! nearest triangle of each run, from the same distances.

use super::Work;
use super::grid::{B, BRICK, BRICK_BITS, Brick, FAR, Field, local};
use super::input::Soup;
use crate::math::{V3, at, closest, dist2, morton, unmorton};

/// How far, in voxels, a triangle's distance is measured: one voxel (every sample next to a
/// sign change is within one voxel of the surface) plus a margin for rounding.
pub const REACH: f32 = 1.000_976_6;

/// Longest edge, in voxels, of the pieces a big triangle is binned as.
pub const PIECE: f32 = 4.0;

/// Pieces per parallel binning task.
const CHUNK: usize = 4096;

/// The inclusive range of samples within [`REACH`] of triangle `t` along each axis.
fn reach(t: &[V3; 3]) -> [[u32; 2]; 3] {
    std::array::from_fn(|a| {
        let lo = t[0][a].min(t[1][a]).min(t[2][a]);
        let hi = t[0][a].max(t[1][a]).max(t[2][a]);
        [(lo - REACH).ceil() as u32, (hi + REACH).floor() as u32]
    })
}

/// How many cuts along each edge make a triangle's pieces at most [`PIECE`] voxels long.
fn cuts(t: &[V3; 3]) -> u32 {
    let longest = dist2(t[0], t[1])
        .max(dist2(t[1], t[2]))
        .max(dist2(t[2], t[0]));
    (longest.sqrt() / PIECE).ceil().max(1.0) as u32
}

/// Piece `p` of triangle `t` cut into `n * n` pieces. Row `i` of the cut has `2i + 1` pieces,
/// alternately pointing up and down, between the points `t0 + i/n (t1 - t0) + j/n (t2 - t1)`.
fn piece(t: &[V3; 3], n: u32, p: u32) -> [V3; 3] {
    let row = p.isqrt();
    let k = p - row * row;
    let m = k / 2;
    let point = |i: u32, j: u32| -> V3 {
        let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
        std::array::from_fn(|a| t[0][a] + u * (t[1][a] - t[0][a]) + v * (t[2][a] - t[1][a]))
    };
    if k.is_multiple_of(2) {
        [point(row, m), point(row + 1, m), point(row + 1, m + 1)]
    } else {
        [point(row, m), point(row + 1, m + 1), point(row, m + 1)]
    }
}

/// What gets binned: each triangle small enough as itself, and the pieces of the big ones.
struct Pieces {
    /// Cuts per edge of each triangle; 0 for a flat one, which has no closest point of its own.
    cuts: Vec<u32>,
    /// The big triangles' pieces as (triangle, piece). Binned with ids after the triangles'.
    big: Vec<(u32, u32)>,
}

impl Pieces {
    fn new(soup: &Soup) -> Self {
        let cuts = gs_par::map(soup.tris.len(), |t| {
            let t = t as u32;
            if soup.flat(t) {
                0
            } else {
                cuts(&soup.corners(t))
            }
        });
        let mut big = Vec::new();
        for (t, &n) in (0u32..).zip(&cuts) {
            if n >= 2 {
                big.extend((0..n * n).map(|p| (t, p)));
            }
        }
        Self { cuts, big }
    }

    /// The triangle and the corners of the piece with binning id `id`, if it is binned.
    fn get(&self, soup: &Soup, id: usize) -> Option<(u32, [V3; 3])> {
        match self.big.get(id.wrapping_sub(self.cuts.len())) {
            Some(&(t, p)) => Some((t, piece(&soup.corners(t), self.cuts[t as usize], p))),
            None => (self.cuts[id] == 1).then(|| (id as u32, soup.corners(id as u32))),
        }
    }
}

/// A band key's distance in voxels, clamped to [`FAR`], and its triangle ([`super::grid::NONE`]
/// for no key, `u64::MAX`).
pub(crate) fn split(key: u64) -> (f32, u32) {
    (
        f32::from_bits((key >> 32) as u32).sqrt().min(FAR),
        key as u32,
    )
}

/// Measures the band into a new field. For a fold of `runs` runs (`super::fold`), `run` gives
/// each triangle's run, and each brick also keeps each run's nearest triangle; for one run,
/// `run` is empty.
pub(crate) fn measure(soup: &Soup, run: &[u32], runs: usize, field: &mut Field, work: &mut Work) {
    let pieces = Pieces::new(soup);
    let ids = pieces.cuts.len() + pieces.big.len();
    let lists = gs_par::map(ids.div_ceil(CHUNK), |c| {
        let mut keys = Vec::new();
        for id in c * CHUNK..ids.min((c + 1) * CHUNK) {
            let Some((_, shape)) = pieces.get(soup, id) else {
                continue;
            };
            let r = reach(&shape);
            for z in r[2][0] / B..=r[2][1] / B {
                for y in r[1][0] / B..=r[1][1] / B {
                    for x in r[0][0] / B..=r[0][1] / B {
                        keys.push((morton([x, y, z], BRICK_BITS) << 32) + id as u64);
                    }
                }
            }
        }
        keys
    });
    let mut keys = lists.concat();
    work.binned = keys.len() as u64;
    gs_par::sort_keys(&mut keys);
    let groups: Vec<&[u64]> = keys.chunk_by(|a, b| a >> 32 == b >> 32).collect();
    let bricks = gs_par::map(groups.len(), |g| brick(soup, &pieces, groups[g], run, runs));
    for (brick, each, evaluations) in bricks {
        work.distance_evaluations += evaluations;
        // A brick that only reach boxes touched, with no sample within reach of a triangle, has
        // no boundary sample and stays uniform.
        if brick.value.iter().any(|&v| v <= REACH) {
            field.insert(brick);
            if !run.is_empty() {
                field.runs.push(each);
            }
        }
    }
}

/// One brick's samples, measured against the pieces in `keys` (all for this brick, sorted),
/// with each run's nearest triangle too when `run` (each triangle's run) is not empty (see
/// `Field::runs`; empty otherwise). Returns the brick, those, and how many distances it
/// computed.
fn brick(
    soup: &Soup,
    pieces: &Pieces,
    keys: &[u64],
    run: &[u32],
    runs: usize,
) -> (Brick, Box<[u64]>, u64) {
    let at_brick = unmorton(keys[0] >> 32, BRICK_BITS);
    let base = at_brick.map(|c| c * B);
    // Squared distance in the high half, triangle in the low half: the minimum is the nearest
    // triangle, the lowest-numbered one on a tie, whatever order the pieces come in.
    let mut best = [u64::MAX; BRICK];
    let mut each = Vec::new();
    let evaluations = if run.is_empty() {
        visit(soup, pieces, keys, base, |i, key| {
            best[i] = best[i].min(key);
        })
    } else {
        each = vec![u64::MAX; runs * BRICK];
        visit(soup, pieces, keys, base, |i, key| {
            best[i] = best[i].min(key);
            let k = run[key as u32 as usize] as usize * BRICK + i;
            each[k] = each[k].min(key);
        })
    };
    let brick = Brick {
        at: at_brick,
        value: Box::new(best.map(|b| split(b).0)),
        near: Box::new(best.map(|b| b as u32)),
    };
    (brick, each.into_boxed_slice(), evaluations)
}

/// Measures the samples of the brick whose first sample is `base` against the pieces in
/// `keys`, handing each sample's index in the brick and its key (squared distance in the high
/// half, triangle in the low half) to `keep`. Returns how many distances it computed.
fn visit(
    soup: &Soup,
    pieces: &Pieces,
    keys: &[u64],
    base: [u32; 3],
    mut keep: impl FnMut(usize, u64),
) -> u64 {
    let mut evaluations = 0;
    for &key in keys {
        let Some((t, shape)) = pieces.get(soup, key as u32 as usize) else {
            continue;
        };
        let tri = soup.corners(t);
        let r = reach(&shape);
        let lo: [u32; 3] = std::array::from_fn(|a| r[a][0].max(base[a]));
        let hi: [u32; 3] = std::array::from_fn(|a| r[a][1].min(base[a] + B - 1));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let p = [x as f32, y as f32, z as f32];
                    let d2 = dist2(p, at(&tri, closest(p, &tri)));
                    keep(
                        local([x, y, z]),
                        (u64::from(d2.to_bits()) << 32) + u64::from(t),
                    );
                    evaluations += 1;
                }
            }
        }
    }
    evaluations
}
