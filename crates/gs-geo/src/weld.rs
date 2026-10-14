//! Welding and repair, shared by decimation, voxel capping and quad remesh preparation
//! (decision 0034, section 4).
//!
//! - [`exact`]: each point's stand-in is the lowest-numbered point at the same position;
//! - [`near`]: the same within a distance;
//! - [`repair`]: removes duplicate faces, then gives every edge still on more than two faces
//!   copies of its points until each edge has at most two, and counts what it changed.
//!
//! Faces are `PlainMesh` faces: three or four point indices, a triangle's fourth slot
//! [`NO_VERTEX`](gs_protocol::NO_VERTEX).

use gs_core::math::{Bins, same_cycle};

/// Each point's stand-in: the lowest-numbered point at the same position (`-0` reads as `0`).
pub fn exact(positions: &[[f32; 3]]) -> Vec<u32> {
    // Adding zero turns -0 into +0 and changes nothing else.
    let mut keyed = gs_par::map(positions.len(), |v| {
        (positions[v].map(|c| (c + 0.0).to_bits()), v as u32)
    });
    gs_par::sort(&mut keyed);
    let mut weld = vec![0; positions.len()];
    for same in keyed.chunk_by(|a, b| a.0 == b.0) {
        for &(_, v) in same {
            weld[v as usize] = same[0].1;
        }
    }
    weld
}

/// Each point's stand-in: the lowest-numbered point within `tolerance` of it, followed until
/// it reaches a point that is its own stand-in. A tolerance of 0 welds points at the same
/// position, as [`exact`] does.
pub fn near(positions: &[[f32; 3]], tolerance: f32) -> Vec<u32> {
    let tol = f64::from(tolerance.max(0.0));
    let reach = (positions.iter().flatten()).fold(0.0_f64, |m, &x| m.max(f64::from(x).abs()));
    // Cells at least twice the tolerance wide, so a search covers at most two along each axis.
    let side = (2.0 * tol)
        .max(reach * 1e-6)
        .max(f64::from(f32::MIN_POSITIVE));
    let at = |v: usize| positions[v].map(f64::from);
    let key = |p: [f64; 3]| p.map(|x| (x / side).floor() as i64);
    let bins = Bins::new(positions.len(), |v| key(at(v)));
    let mut weld = gs_par::map(positions.len(), |v| {
        let p = at(v);
        let (lo, hi) = (key(p.map(|x| x - tol)), key(p.map(|x| x + tol)));
        let close = |&w: &u32| {
            let q = at(w as usize);
            let d2: f64 = (0..3).map(|k| (q[k] - p[k]) * (q[k] - p[k])).sum();
            d2 <= tol * tol
        };
        (bins.within(lo, hi)).filter(close).fold(v as u32, u32::min)
    });
    // Stand-ins are never above their points, so one pass upward follows every chain.
    for v in 0..weld.len() {
        weld[v] = weld[weld[v] as usize];
    }
    weld
}

/// What [`repair`] left and changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repaired {
    /// The faces left.
    pub faces: Vec<[u32; 4]>,
    /// The input face each one was.
    pub from: Vec<u32>,
    /// The points added: point `points + k` (`points` as given to [`repair`]) is a copy of
    /// point `copies[k]`.
    pub copies: Vec<u32>,
    /// Faces removed as duplicates.
    pub duplicate_faces: u64,
    /// Edges on more than two faces once the duplicates were gone.
    pub crowded_edges: u64,
}

/// Repairs `faces` over `points` points.
///
/// Faces on the same points are duplicates when they run through them in the same cycle, one
/// way or the other: opposite windings cancel in pairs and both go (a fold), and of those left
/// with one winding the first stays. Then each edge still on more than two faces has them
/// paired, faces running along it one way with faces running the other way, in face order. At
/// each of its two points, the faces there are grouped by what joins them: an edge of at most
/// two faces, or a crowded edge within one pair. The group with the first face keeps the point
/// and each other group gets a copy of it. A face still on a crowded edge after that (the
/// groups met around both points) gets copies of that edge's points of its own.
pub fn repair(faces: &[[u32; 4]], points: usize) -> Repaired {
    let (kept, duplicate_faces) = without_duplicates(faces);
    let mut out = Repaired {
        faces: kept.iter().map(|&f| faces[f as usize]).collect(),
        from: kept,
        duplicate_faces,
        ..Repaired::default()
    };
    let crowded = crowded(&edges(&out.faces));
    out.crowded_edges = crowded.len() as u64;
    if crowded.is_empty() {
        return out;
    }
    let sheets = sheets(&edges(&out.faces), &crowded);
    let mut next = points as u32;
    split_points(&mut out, &sheets, &mut next);
    // What is still crowded gets copies of its points, face by face, after the first pair.
    let census = edges(&out.faces);
    for run in census.chunk_by(|a, b| a.0 == b.0).filter(|r| r.len() > 2) {
        for &(edge, f, _) in &run[2..] {
            let face = &mut out.faces[f as usize];
            for end in [(edge >> 32) as u32, edge as u32] {
                for c in face.iter_mut().filter(|c| **c == end) {
                    out.copies.push(end);
                    *c = next;
                    next += 1;
                }
            }
        }
    }
    out
}

/// The faces to keep, by index, and how many duplicates went.
fn without_duplicates(faces: &[[u32; 4]]) -> (Vec<u32>, u64) {
    let mut keyed = gs_par::map(faces.len(), |f| {
        let mut k = faces[f];
        k.sort_unstable();
        (k, f as u32)
    });
    gs_par::sort(&mut keyed);
    let mut keep = vec![true; faces.len()];
    for same in keyed.chunk_by(|a, b| a.0 == b.0).filter(|s| s.len() > 1) {
        let first = faces[same[0].1 as usize];
        let (mut with, mut against) = (Vec::new(), Vec::new());
        for &(_, f) in same {
            if same_cycle(first, faces[f as usize]) {
                with.push(f);
            } else if same_cycle(reversed(first), faces[f as usize]) {
                against.push(f);
            }
        }
        // The folds go, then all but the first of what is left.
        let folds = with.len().min(against.len());
        for side in [&with, &against] {
            let gone = side[..folds].iter().chain(side.iter().skip(folds + 1));
            for &f in gone {
                keep[f as usize] = false;
            }
        }
    }
    let kept: Vec<u32> = (0..faces.len() as u32)
        .filter(|&f| keep[f as usize])
        .collect();
    let gone = (faces.len() - kept.len()) as u64;
    (kept, gone)
}

pub(crate) use gs_protocol::corners;

/// `f` with its winding reversed.
fn reversed(f: [u32; 4]) -> [u32; 4] {
    let mut g = f;
    let k = corners(&f).len();
    g[..k].reverse();
    g
}

/// Every edge of every face as (the edge's two points, lower in the high half; the face;
/// whether the face runs along it from the lower point), sorted.
fn edges(faces: &[[u32; 4]]) -> Vec<(u64, u32, bool)> {
    let mut e: Vec<(u64, u32, bool)> = (faces.iter().enumerate())
        .flat_map(|(f, face)| {
            let c = corners(face);
            (0..c.len()).map(move |k| {
                let (a, b) = (c[k], c[(k + 1) % c.len()]);
                let key = (u64::from(a.min(b)) << 32) | u64::from(a.max(b));
                (key, f as u32, a < b)
            })
        })
        .collect();
    gs_par::sort(&mut e);
    e
}

/// The edges on more than two faces, sorted.
fn crowded(census: &[(u64, u32, bool)]) -> Vec<u64> {
    (census.chunk_by(|a, b| a.0 == b.0))
        .filter(|run| run.len() > 2)
        .map(|run| run[0].0)
        .collect()
}

/// For each crowded edge, each face on it with its pair number: the `i`th face running along
/// the edge one way pairs with the `i`th running the other way. Sorted by (edge, face).
fn sheets(census: &[(u64, u32, bool)], crowded: &[u64]) -> Vec<(u64, u32, u32)> {
    let mut out = Vec::new();
    for &edge in crowded {
        let start = census.partition_point(|e| e.0 < edge);
        let run = census[start..].iter().take_while(|e| e.0 == edge);
        let (mut up, mut down) = (0, 0);
        for &(_, f, forward) in run {
            let n = if forward { &mut up } else { &mut down };
            out.push((edge, f, *n));
            *n += 1;
        }
    }
    out.sort_unstable();
    out
}

/// Gives each group of faces at a crowded edge's point, after the first, its own copy of the
/// point (see [`repair`]).
fn split_points(out: &mut Repaired, sheets: &[(u64, u32, u32)], next: &mut u32) {
    let census = edges(&out.faces);
    let mut ends: Vec<u32> = (sheets.iter())
        .flat_map(|s| [(s.0 >> 32) as u32, s.0 as u32])
        .collect();
    ends.sort_unstable();
    ends.dedup();
    // The faces at each of those points, by point then face.
    let mut at: Vec<(u32, u32)> = (out.faces.iter().enumerate())
        .flat_map(|(f, face)| {
            let ends = &ends;
            (corners(face).iter())
                .filter(move |v| ends.binary_search(v).is_ok())
                .map(move |&v| (v, f as u32))
        })
        .collect();
    at.sort_unstable();
    at.dedup();
    let pair = |edge: u64, f: u32| {
        let i = sheets.partition_point(|s| (s.0, s.1) < (edge, f));
        sheets[i].2
    };
    let mut moves: Vec<(u32, u32, u32)> = Vec::new();
    for fan in at.chunk_by(|a, b| a.0 == b.0) {
        let v = fan[0].0;
        let faces: Vec<u32> = fan.iter().map(|x| x.1).collect();
        let mut root: Vec<usize> = (0..faces.len()).collect();
        for (i, &f) in faces.iter().enumerate() {
            let c = corners(&out.faces[f as usize]);
            let k = c.iter().position(|&x| x == v).unwrap_or(0);
            for w in [c[(k + 1) % c.len()], c[(k + c.len() - 1) % c.len()]] {
                let edge = (u64::from(v.min(w)) << 32) | u64::from(v.max(w));
                let start = census.partition_point(|e| e.0 < edge);
                let run: Vec<u32> = (census[start..].iter())
                    .take_while(|e| e.0 == edge)
                    .map(|e| e.1)
                    .collect();
                let crowded = run.len() > 2;
                for g in run.into_iter().filter(|&g| g != f) {
                    if crowded && pair(edge, g) != pair(edge, f) {
                        continue;
                    }
                    if let Ok(j) = faces.binary_search(&g) {
                        join(&mut root, i, j);
                    }
                }
            }
        }
        // Groups in order of their first face; the first keeps the point.
        let mut copy_of_group: Vec<Option<u32>> = vec![None; faces.len()];
        let first = find(&mut root, 0);
        for (i, &f) in faces.iter().enumerate() {
            let r = find(&mut root, i);
            if r == first {
                continue;
            }
            let id = *copy_of_group[r].get_or_insert_with(|| {
                out.copies.push(v);
                *next += 1;
                *next - 1
            });
            moves.push((f, v, id));
        }
    }
    for (f, v, id) in moves {
        for c in out.faces[f as usize].iter_mut().filter(|c| **c == v) {
            *c = id;
        }
    }
}

/// The root of `i`'s group, flattening the path to it (Auto UV's cutter shares these two).
pub(crate) fn find(root: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while root[r] != r {
        r = root[r];
    }
    let mut j = i;
    while root[j] != r {
        let up = root[j];
        root[j] = r;
        j = up;
    }
    r
}

/// Joins two groups under the lower root, so a group's root is its first face.
pub(crate) fn join(root: &mut [usize], i: usize, j: usize) {
    let (a, b) = (find(root, i), find(root, j));
    root[a.max(b)] = a.min(b);
}
