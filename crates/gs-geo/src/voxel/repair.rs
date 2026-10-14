//! Repair: makes the inside set "well-composed", so Surface Nets gives a manifold.
//!
//! Surface Nets puts one vertex in every cell with a sign change and one quad across every
//! sign-changing edge, which is the boundary of the union of the cubes around the inside
//! samples with its corners moved. That boundary is a manifold surface exactly when the inside
//! set has no critical configuration (Latecki 1997): no 2 x 2 square of samples whose two
//! diagonals have opposite signs, and no 2 x 2 x 2 cube whose only two samples of one sign sit
//! on opposite corners. Each one found is fixed by moving the configuration's outside sample
//! that is closest to the surface inside. Samples only ever move inside, so the repair always
//! finishes. In practice it touches a handful of samples where two surfaces come within a voxel
//! of each other.

use super::Work;
use super::grid::{B, BRICK, BRICK_BITS, Field, cell, corner, local};
use super::sign::{Sign, around, resolve_around, settle};
use crate::math::{morton, unmorton};

/// Rounds of fixes before giving up (each round fixes everything it found).
const ROUNDS: usize = 64;
/// The three 2 x 2 squares at a cube's first corner, as cube corners (`x + 2y + 4z`), listed so
/// that entries 0 and 3 are diagonal, and so are entries 1 and 2.
const SQUARES: [[usize; 4]; 3] = [[0, 1, 2, 3], [0, 1, 4, 5], [0, 2, 4, 6]];

/// Fixes every critical configuration. Allocates the bricks the fixes reach.
pub(crate) fn repair(field: &mut Field, sign: &Sign, work: &mut Work) {
    let mut todo: Vec<[u32; 3]> = field.bricks.iter().map(|b| b.at).collect();
    for _ in 0..ROUNDS {
        let found = gs_par::map(todo.len(), |k| critical(field, todo[k]));
        let mut flips = found.concat();
        gs_par::sort(&mut flips);
        flips.dedup();
        if flips.is_empty() {
            return;
        }
        work.flips += flips.len() as u64;
        let mut next = Vec::new();
        for &p in &flips {
            flip(field, sign, p, work);
            // Every configuration that has `p` as a corner starts in one of these bricks.
            for k in 0..8 {
                let d = corner(k);
                let first: [u32; 3] = std::array::from_fn(|a| p[a].saturating_sub(d[a]));
                next.push(morton(first.map(|c| c / B), BRICK_BITS));
            }
        }
        gs_par::sort_keys(&mut next);
        next.dedup();
        todo = next.iter().map(|&key| unmorton(key, BRICK_BITS)).collect();
    }
}

/// Moves sample `p` inside. Its neighbours may now be boundary samples, so every brick around
/// its brick is allocated too (with the values it had, so nothing else changes), and an edge
/// brick found inside past them is settled.
fn flip(field: &mut Field, sign: &Sign, p: [u32; 3], work: &mut Work) {
    let home = p.map(|c| c / B);
    let slot = field.materialize(home);
    let value = &mut field.bricks[slot].value[local(p)];
    *value = -value.abs();
    for d in around() {
        if let Some(b) = field.neighbor(home, d)
            && field.slot(b).is_none()
        {
            let slot = field.materialize(b);
            let edge = resolve_around(field, sign, &[slot], work);
            settle(field, sign, edge, work);
        }
    }
}

/// The samples to move inside to fix the critical configurations whose first corner is in the
/// brick at `at`: for each, its outside corner nearest the surface.
fn critical(field: &Field, at: [u32; 3]) -> Vec<[u32; 3]> {
    const ALL: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    let block = field.gather(at);
    let mut out = Vec::new();
    for i in 0..BRICK {
        let c = [i as u32 % B, i as u32 / B % B, i as u32 / (B * B)];
        let v = cell(&block, c);
        let inside = v.map(f32::is_sign_negative);
        let mut bad: Vec<&[usize]> = Vec::new();
        for s in &SQUARES {
            if inside[s[0]] == inside[s[3]]
                && inside[s[1]] == inside[s[2]]
                && inside[s[0]] != inside[s[1]]
            {
                bad.push(s);
            }
        }
        for k in 0..4 {
            let lonely = (0..8)
                .filter(|&j| j != k && j != 7 - k)
                .all(|j| inside[j] != inside[k]);
            if inside[k] == inside[7 - k] && lonely {
                bad.push(&ALL);
            }
        }
        for corners in bad {
            let pick = corners
                .iter()
                .filter(|&&k| !inside[k])
                .min_by_key(|&&k| (v[k].abs().to_bits(), k));
            if let Some(&k) = pick {
                let d = corner(k);
                out.push(std::array::from_fn(|a| at[a] * B + c[a] + d[a]));
            }
        }
    }
    out
}
